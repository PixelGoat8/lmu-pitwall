//! Combined HTTP + WebSocket server on a single port.
//!
//! - Incoming connection has `Upgrade: websocket` → handed to the WS handler.
//! - All other HTTP GET requests → static files embedded via rust-embed.
//! - Unknown paths fall back to `index.html` for SPA client-side routing.
//! - CORS headers are set so Android/remote browsers can connect freely.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

use crate::access::{self, Access, Level};
use crate::assets::Asset;
use crate::websocket::server::WebSocketServer;

/// Bind on `0.0.0.0:port` and serve HTTP + WebSocket connections.
pub async fn run(ws: Arc<WebSocketServer>, port: u16, access: Arc<Access>) -> Result<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await?;
    info!(
        "Dashboard available at http://0.0.0.0:{} — open http://localhost:{} in your browser",
        port, port
    );

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let ws = ws.clone();
                let access = access.clone();
                tokio::spawn(async move {
                    handle_connection(stream, peer, ws, port, access).await;
                });
            }
            Err(e) => {
                warn!("TCP accept error: {}", e);
            }
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    ws: Arc<WebSocketServer>,
    port: u16,
    access: Arc<Access>,
) {
    // Peek without consuming so the WS handshake can re-read the same bytes.
    let mut buf = [0u8; 4096];
    let n = match stream.peek(&mut buf).await {
        Ok(n) => n,
        Err(e) => {
            debug!("Peek error from {}: {}", peer, e);
            return;
        }
    };

    let preview = String::from_utf8_lossy(&buf[..n]).to_lowercase();

    if preview.contains("upgrade: websocket") {
        // WebSocket upgrade — tokio-tungstenite will re-read the request.
        ws.accept_client(stream, peer);
    } else {
        if let Err(e) = handle_http(stream, peer, port, &access).await {
            debug!("HTTP handler error from {}: {}", peer, e);
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal HTTP/1.1 static-file handler
// ---------------------------------------------------------------------------

async fn handle_http(
    mut stream: TcpStream,
    peer: SocketAddr,
    port: u16,
    access: &Access,
) -> Result<()> {
    // Read until end of HTTP headers (\r\n\r\n).
    let mut request = Vec::with_capacity(2048);
    let mut buf = [0u8; 4096];

    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        request.extend_from_slice(&buf[..n]);
        if request.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if request.len() > 32_768 {
            send_response(&mut stream, 413, "text/plain", b"Request Too Large", &[]).await?;
            return Ok(());
        }
    }

    let request_str = String::from_utf8_lossy(&request);

    // Only handle GET (and HEAD/OPTIONS for CORS preflight).
    let method = request_str
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().next())
        .unwrap_or("")
        .to_uppercase();

    let path = parse_path(&request_str);

    if method == "OPTIONS" {
        // CORS preflight
        let cors = [
            ("Access-Control-Allow-Origin", "*"),
            ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
            ("Access-Control-Allow-Headers", "*"),
        ];
        send_response(&mut stream, 204, "text/plain", b"", &cors).await?;
        return Ok(());
    }

    // Who is asking. Static files and /api/version are served to anyone who
    // can reach the port; everything else needs a level (see `crate::access`).
    let level = access.authorize(peer.ip(), access::token_from_request(&request_str));
    let api_cors = [
        ("Access-Control-Allow-Origin", "*"),
        ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
    ];

    // API routes

    // Lets the dashboard tell "wrong token" apart from "bridge unreachable":
    // a browser never sees why a WebSocket handshake was refused.
    if path == "/api/auth-check" {
        return match level {
            Some(l) => {
                let body = format!(r#"{{"ok":true,"local":{}}}"#, l == Level::Local);
                send_response(&mut stream, 200, "application/json", body.as_bytes(), &api_cors).await
            }
            None => send_unauthorized(&mut stream, &api_cors).await,
        };
    }

    if path == "/api/network-info" {
        let Some(level) = level else {
            return send_unauthorized(&mut stream, &api_cors).await;
        };
        let ip = get_local_ip().unwrap_or_else(|| "unknown".to_string());
        // Only the racing PC's own browser learns the token and the Tailscale
        // address: together they are the link the driver sends the engineer.
        let body = if level == Level::Local {
            serde_json::json!({
                "ip": ip,
                "port": port,
                "local": true,
                "tailscale_ip": get_tailscale_ip(),
                "token": access.token(),
            })
        } else {
            serde_json::json!({ "ip": ip, "port": port, "local": false })
        };
        send_response(&mut stream, 200, "application/json", body.to_string().as_bytes(), &api_cors).await?;
        return Ok(());
    }

    if path == "/api/version" {
        let body = format!(r#"{{"version":"{}"}}"#, env!("CARGO_PKG_VERSION"));
        let cors = [
            ("Access-Control-Allow-Origin", "*"),
            ("Access-Control-Allow-Methods", "GET, OPTIONS"),
        ];
        send_response(&mut stream, 200, "application/json", body.as_bytes(), &cors).await?;
        return Ok(());
    }

    // Control endpoints change the racing PC, so they are for its own browser
    // only: loopback peer, and no foreign web page as the origin.
    let is_control = (path == "/api/shutdown" || path == "/api/set-port") && method == "POST";
    if is_control && !(level == Some(Level::Local) && access::origin_is_local(&request_str)) {
        warn!("Refused {} from {} (control endpoints are local-only)", path, peer);
        send_response(
            &mut stream,
            403,
            "application/json",
            b"{\"ok\":false,\"error\":\"local only\"}",
            &api_cors,
        )
        .await?;
        return Ok(());
    }

    if path == "/api/shutdown" && method == "POST" {
        let cors = [
            ("Access-Control-Allow-Origin", "*"),
            ("Access-Control-Allow-Methods", "POST, OPTIONS"),
        ];
        send_response(&mut stream, 200, "application/json", b"{\"ok\":true}", &cors).await?;
        // Response is written — exit cleanly so the new instance can take over the port.
        std::process::exit(0);
    }

    if path == "/api/set-port" && method == "POST" {
        let cors = [
            ("Access-Control-Allow-Origin", "*"),
            ("Access-Control-Allow-Methods", "POST, OPTIONS"),
        ];

        // Parse Content-Length header and extract body.
        let body_start = request_str.find("\r\n\r\n").map(|i| i + 4).unwrap_or(request.len());
        let content_length: usize = request_str
            .lines()
            .find(|l| l.to_lowercase().starts_with("content-length:"))
            .and_then(|l| l.split(':').nth(1))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        let body_bytes = &request[body_start..(body_start + content_length).min(request.len())];

        #[derive(serde::Deserialize)]
        struct SetPortRequest { port: u16 }

        let parsed = serde_json::from_slice::<SetPortRequest>(body_bytes);
        match parsed {
            Ok(req) if req.port >= 1024 => {
                if let Err(e) = crate::app_config::AppConfig::set_port(req.port) {
                    let msg = format!("{{\"ok\":false,\"error\":\"{}\"}}", e);
                    send_response(&mut stream, 500, "application/json", msg.as_bytes(), &cors).await?;
                    return Ok(());
                }
                let body = format!("{{\"ok\":true,\"port\":{}}}", req.port);
                send_response(&mut stream, 200, "application/json", body.as_bytes(), &cors).await?;
                // Spawn detached child with new port, then exit.
                if let Ok(exe) = std::env::current_exe() {
                    let mut cmd = std::process::Command::new(exe);
                    cmd.arg("--no-browser");
                    #[cfg(target_os = "windows")]
                    {
                        use std::os::windows::process::CommandExt;
                        const DETACHED_PROCESS: u32 = 0x00000008;
                        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
                        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
                    }
                    let _ = cmd.spawn();
                }
                std::process::exit(0);
            }
            _ => {
                send_response(&mut stream, 400, "application/json",
                    b"{\"ok\":false,\"error\":\"port must be 1024-65535\"}", &cors).await?;
            }
        }
        return Ok(());
    }

    serve_static(&mut stream, &path).await
}

fn parse_path(request: &str) -> String {
    let first_line = request.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.splitn(3, ' ').collect();
    if parts.len() >= 2 {
        parts[1].split('?').next().unwrap_or("/").to_string()
    } else {
        "/".to_string()
    }
}

async fn serve_static(stream: &mut TcpStream, path: &str) -> Result<()> {
    let file_path = if path == "/" {
        "index.html"
    } else {
        path.trim_start_matches('/')
    };

    let cors = [
        ("Access-Control-Allow-Origin", "*"),
        ("Access-Control-Allow-Methods", "GET, OPTIONS"),
    ];

    if let Some(file) = Asset::get(file_path) {
        let mime = mime_type(file_path);
        // Add cache headers for hashed assets (everything under assets/)
        let cache = if file_path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        let mut headers: Vec<(&str, &str)> = cors.to_vec();
        headers.push(("Cache-Control", cache));
        send_response(stream, 200, mime, &file.data, &headers).await
    } else {
        // SPA fallback: serve index.html for any unrecognised path
        if let Some(index) = Asset::get("index.html") {
            let mut headers: Vec<(&str, &str)> = cors.to_vec();
            headers.push(("Cache-Control", "no-cache"));
            send_response(
                stream,
                200,
                "text/html; charset=utf-8",
                &index.data,
                &headers,
            )
            .await
        } else {
            send_response(stream, 404, "text/plain", b"Not Found", &cors).await
        }
    }
}

async fn send_unauthorized(stream: &mut TcpStream, cors: &[(&str, &str)]) -> Result<()> {
    send_response(
        stream,
        401,
        "application/json",
        b"{\"ok\":false,\"error\":\"invalid or missing token\"}",
        cors,
    )
    .await
}

/// Returns this machine's Tailscale IPv4 address, if Tailscale is up.
///
/// Same trick as [`get_local_ip`]: "connect" a UDP socket to Tailscale's
/// service address (100.100.100.100) and read back which local address the
/// OS routed it through. Nothing is sent. Without Tailscale the route falls
/// to the normal LAN interface, so the result is only accepted when it lies
/// in Tailscale's range, 100.64.0.0/10.
fn get_tailscale_ip() -> Option<String> {
    use std::net::{IpAddr, UdpSocket};
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("100.100.100.100:53").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if is_tailscale_v4(v4) => Some(v4.to_string()),
        _ => None,
    }
}

fn is_tailscale_v4(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (o[1] & 0xC0) == 64
}

/// Returns the primary LAN IPv4 address by probing a UDP socket.
/// No packets are actually sent — connect() on a UDP socket just sets the routing destination.
fn get_local_ip() -> Option<String> {
    use std::net::UdpSocket;
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    Some(addr.ip().to_string())
}

fn mime_type(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "webmanifest" => "application/manifest+json",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

async fn send_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    extra_headers: &[(&str, &str)],
) -> Result<()> {
    let status_text = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        413 => "Request Entity Too Large",
        _ => "Unknown",
    };

    let mut response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        status, status_text, content_type, body.len()
    );
    for (name, value) in extra_headers {
        response.push_str(&format!("{}: {}\r\n", name, value));
    }
    response.push_str("\r\n");

    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_tailscale_v4;

    #[test]
    fn tailscale_range_is_100_64_slash_10() {
        let yes = ["100.64.0.1", "100.101.102.103", "100.127.255.254"];
        let no = ["100.63.255.255", "100.128.0.1", "192.168.1.10", "10.0.0.1"];
        for a in yes {
            assert!(is_tailscale_v4(a.parse().unwrap()), "{a}");
        }
        for a in no {
            assert!(!is_tailscale_v4(a.parse().unwrap()), "{a}");
        }
    }
}
