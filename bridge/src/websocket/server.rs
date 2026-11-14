use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, watch};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::access::{self, Access, Level};
use crate::fuel_calculator::api::handle_command as fuel_handle;
use crate::post_race::api::handle_command as post_race_handle;
use crate::protocol::messages::{ClientCommand, ServerMessage};
use crate::race_engineer::{self, RaceEngineerService};

/// Broadcast channel capacity — number of queued messages per slow client
/// before older messages are dropped (lagged receiver).
const BROADCAST_CAPACITY: usize = 128;

/// How often each client gets a `Heartbeat` message and a WebSocket ping.
///
/// The heartbeat is for the dashboard: outside a session the bridge can be
/// silent for minutes, and without a regular message the client cannot tell
/// "nothing to say" from "the link died" (a tablet that slept, a Tailscale
/// path that dropped). The ping is for us: the browser answers it on its own,
/// and that answer is how we notice a client that vanished without closing.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);

/// A client that has sent nothing — not even a pong — for this long is gone.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(20);

/// Wire serialization format negotiated per-client via query parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// MessagePack binary — primary format, compact and fast.
    MsgPack,
    /// JSON text — enabled via `?format=json`, useful for debugging.
    Json,
}

/// WebSocket broadcast server.
///
/// Accepts clients on `ws://0.0.0.0:<port>` and fans out every
/// [`ServerMessage`] sent via [`broadcast`] to all connected clients.
/// `EngineerAudio` messages use a separate `audio_tx` channel so that clients
/// registered as `display_only` never receive large WAV payloads.
pub struct WebSocketServer {
    port: u16,
    tx: broadcast::Sender<Arc<ServerMessage>>,
    /// Audio-only channel — only `EngineerAudio` messages are sent here.
    audio_tx: broadcast::Sender<Arc<ServerMessage>>,
    /// Live count of connected WebSocket clients.
    client_count: Arc<AtomicUsize>,
    /// Watch channel — subscribers receive the new count on every change.
    count_tx: watch::Sender<usize>,
    /// Latest AllDriversUpdate — sent immediately to newly connecting clients.
    all_drivers_rx: watch::Receiver<Option<ServerMessage>>,
    /// Latest VersionInfo — sent immediately to newly connecting clients.
    version_info_rx: watch::Receiver<Option<ServerMessage>>,
    /// Latest ConnectionStatus — sent immediately to newly connecting clients.
    connection_status_rx: watch::Receiver<Option<ServerMessage>>,
    engineer_service: Arc<RaceEngineerService>,
    access: Arc<Access>,
}

impl WebSocketServer {
    pub fn new(
        port: u16,
        all_drivers_rx: watch::Receiver<Option<ServerMessage>>,
        version_info_rx: watch::Receiver<Option<ServerMessage>>,
        connection_status_rx: watch::Receiver<Option<ServerMessage>>,
        engineer_service: Arc<RaceEngineerService>,
        access: Arc<Access>,
    ) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (audio_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (count_tx, _) = watch::channel(0usize);
        WebSocketServer {
            port,
            tx,
            audio_tx,
            client_count: Arc::new(AtomicUsize::new(0)),
            count_tx,
            all_drivers_rx,
            version_info_rx,
            connection_status_rx,
            engineer_service,
            access,
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Subscribe to client-count changes.
    pub fn client_count_rx(&self) -> watch::Receiver<usize> {
        self.count_tx.subscribe()
    }

    /// Returns a cloned sender for broadcasting from another task.
    pub fn broadcaster(&self) -> broadcast::Sender<Arc<ServerMessage>> {
        self.tx.clone()
    }

    /// Returns a cloned sender for audio-only broadcast (EngineerAudio messages).
    /// Only clients registered as `audio` role receive from this channel.
    pub fn audio_broadcaster(&self) -> broadcast::Sender<Arc<ServerMessage>> {
        self.audio_tx.clone()
    }

    /// Send a message to every connected client.
    pub fn broadcast(&self, msg: ServerMessage) -> usize {
        match self.tx.send(Arc::new(msg)) {
            Ok(n) => n,
            Err(_) => 0,
        }
    }

    /// Accept a single WebSocket client from an already-accepted `TcpStream`.
    pub fn accept_client(&self, stream: TcpStream, peer: SocketAddr) {
        debug!("New WebSocket connection from {}", peer);
        let rx = self.tx.subscribe();
        let audio_rx = self.audio_tx.subscribe();

        // The client is counted only once its handshake has been accepted
        // (see `handle_client`): a refused connection must neither keep the
        // bridge alive nor arm the idle shutdown.
        tokio::spawn(handle_client(
            stream,
            peer,
            rx,
            audio_rx,
            self.client_count.clone(),
            self.count_tx.clone(),
            self.all_drivers_rx.clone(),
            self.version_info_rx.clone(),
            self.connection_status_rx.clone(),
            self.tx.clone(),
            self.audio_tx.clone(),
            self.engineer_service.clone(),
            self.access.clone(),
        ));
    }
}

// ---------------------------------------------------------------------------
// Per-client handler
// ---------------------------------------------------------------------------

async fn handle_client(
    stream: TcpStream,
    peer: SocketAddr,
    mut rx: broadcast::Receiver<Arc<ServerMessage>>,
    mut audio_rx: broadcast::Receiver<Arc<ServerMessage>>,
    client_count: Arc<AtomicUsize>,
    count_tx: watch::Sender<usize>,
    all_drivers_rx: watch::Receiver<Option<ServerMessage>>,
    version_info_rx: watch::Receiver<Option<ServerMessage>>,
    connection_status_rx: watch::Receiver<Option<ServerMessage>>,
    ws_broadcaster: broadcast::Sender<Arc<ServerMessage>>,
    _audio_broadcaster: broadcast::Sender<Arc<ServerMessage>>,
    engineer_service: Arc<RaceEngineerService>,
    access: Arc<Access>,
) {
    // Default role is audio — this device plays engineer callouts.
    let mut is_audio = true;
    let is_json = Arc::new(AtomicBool::new(false));
    let is_json_cb = is_json.clone();
    let level_flag = Arc::new(AtomicU8::new(LEVEL_NONE));
    let level_cb = level_flag.clone();

    let ws =
        match tokio_tungstenite::accept_hdr_async(stream, move |req: &Request, resp: Response| {
            let json_requested = req
                .uri()
                .query()
                .map(|q| q.split('&').any(|p| p == "format=json"))
                .unwrap_or(false);
            if json_requested {
                is_json_cb.store(true, Ordering::Relaxed);
            }

            // Browsers cannot set headers on a WebSocket, so the token comes
            // as `?token=`. Loopback peers need none.
            let presented = req.uri().query().and_then(access::token_from_query);
            match access.authorize(peer.ip(), presented) {
                Some(Level::Local) => level_cb.store(LEVEL_LOCAL, Ordering::Relaxed),
                Some(Level::Remote) => level_cb.store(LEVEL_REMOTE, Ordering::Relaxed),
                None => {
                    let mut refusal = ErrorResponse::new(Some("invalid or missing token".into()));
                    *refusal.status_mut() = StatusCode::UNAUTHORIZED;
                    return Err(refusal);
                }
            }
            Ok(resp)
        })
        .await
        {
            Ok(ws) => ws,
            Err(e) => {
                if level_flag.load(Ordering::Relaxed) == LEVEL_NONE {
                    warn!("Refused WebSocket client {}: invalid or missing token", peer);
                } else {
                    warn!("WebSocket handshake failed for {}: {}", peer, e);
                }
                return;
            }
        };
    let level = if level_flag.load(Ordering::Relaxed) == LEVEL_LOCAL {
        Level::Local
    } else {
        Level::Remote
    };

    let count = client_count.fetch_add(1, Ordering::Relaxed) + 1;
    let _ = count_tx.send(count);

    let fmt = if is_json.load(Ordering::Relaxed) {
        Format::Json
    } else {
        Format::MsgPack
    };
    info!("Client {} connected ({:?}, format: {:?}, {} total)", peer, level, fmt, count);

    let (mut sink, mut incoming) = ws.split();

    // Send the latest AllDriversUpdate snapshot immediately on connect so the
    // frontend doesn't wait until the next car crosses the S/F line.
    {
        let latest = all_drivers_rx.borrow().clone();
        if let Some(msg) = latest {
            if let Ok(ws_msg) = serialize(&msg, fmt) {
                let _ = sink.send(ws_msg).await;
            }
        }
    }

    // Send the latest VersionInfo immediately on connect (if check has completed).
    {
        let latest = version_info_rx.borrow().clone();
        if let Some(msg) = latest {
            if let Ok(ws_msg) = serialize(&msg, fmt) {
                let _ = sink.send(ws_msg).await;
            }
        }
    }

    // Send the latest ConnectionStatus immediately on connect so the frontend
    // never shows "Waiting" when the game was already running before connect.
    {
        let latest = connection_status_rx.borrow().clone();
        if let Some(msg) = latest {
            if let Ok(ws_msg) = serialize(&msg, fmt) {
                let _ = sink.send(ws_msg).await;
            }
        }
    }

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_seen = Instant::now();

    loop {
        tokio::select! {
            // Liveness, both ways (see HEARTBEAT_INTERVAL).
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > CLIENT_TIMEOUT {
                    info!("Client {} timed out (silent for {:?})", peer, last_seen.elapsed());
                    break;
                }
                let server_time_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let beat = ServerMessage::Heartbeat { server_time_ms };
                let sent = match serialize(&beat, fmt) {
                    Ok(ws_msg) => sink.send(ws_msg).await,
                    Err(_) => Ok(()),
                };
                if sent.is_err() || sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    debug!("Heartbeat to {} failed", peer);
                    break;
                }
            }

            // Outbound: receive broadcast message and forward to this client.
            result = rx.recv() => {
                match result {
                    Ok(msg) => {
                        match serialize(&msg, fmt) {
                            Ok(ws_msg) => {
                                if let Err(e) = sink.send(ws_msg).await {
                                    debug!("Send to {} failed: {}", peer, e);
                                    break;
                                }
                            }
                            Err(e) => {
                                warn!("Serialization error for {}: {}", peer, e);
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(dropped)) => {
                        warn!(
                            "Client {} is too slow, dropped {} messages",
                            peer, dropped
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }

            // Audio-only channel: only forwarded to clients with `audio` role.
            audio_result = audio_rx.recv() => {
                if is_audio {
                    match audio_result {
                        Ok(msg) => {
                            match serialize(&msg, fmt) {
                                Ok(ws_msg) => {
                                    if let Err(e) = sink.send(ws_msg).await {
                                        debug!("Send audio to {} failed: {}", peer, e);
                                        break;
                                    }
                                }
                                Err(e) => {
                                    warn!("Audio serialization error for {}: {}", peer, e);
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(dropped)) => {
                            warn!("Client {} audio too slow, dropped {}", peer, dropped);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                } else {
                    // display_only: drain audio_rx without forwarding.
                    if let Err(broadcast::error::RecvError::Closed) = audio_result {
                        break;
                    }
                }
            }

            // Inbound: dispatch text commands; close and drop everything else.
            frame = incoming.next() => {
                if matches!(frame, Some(Ok(_))) {
                    last_seen = Instant::now();
                }
                match frame {
                    Some(Ok(Message::Close(_))) | None => {
                        debug!("Client {} sent close", peer);
                        break;
                    }
                    Some(Err(e)) => {
                        debug!("Client {} connection error: {}", peer, e);
                        break;
                    }
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientCommand>(&text) {
                            Ok(cmd) => {
                                // Per-client role registration — handled locally.
                                if let ClientCommand::EngineerRegisterClientRole { ref role } = cmd {
                                    is_audio = role != "display_only";
                                    info!(
                                        "Client {} registered as {} (is_audio={})",
                                        peer, role, is_audio
                                    );
                                    continue;
                                }

                                // Commands that change the racing PC are for its own
                                // browser only.
                                if level == Level::Remote && is_local_only(&cmd) {
                                    warn!("Refused local-only command from remote client {}", peer);
                                    let refusal = local_only_refusal(&cmd);
                                    if let Ok(ws_msg) = serialize(&refusal, fmt) {
                                        if sink.send(ws_msg).await.is_err() {
                                            break;
                                        }
                                    }
                                    continue;
                                }

                                let is_engineer = matches!(
                                    cmd,
                                    ClientCommand::EngineerGetStatus
                                        | ClientCommand::EngineerInstallPiper
                                        | ClientCommand::EngineerInstallVoice { .. }
                                        | ClientCommand::EngineerUninstallVoice { .. }
                                        | ClientCommand::EngineerSynthesize { .. }
                                        | ClientCommand::EngineerUpdateBehavior { .. }
                                );

                                if is_engineer {
                                    let svc = engineer_service.clone();
                                    let bcast = ws_broadcaster.clone();
                                    let audio_bcast = _audio_broadcaster.clone();
                                    tokio::spawn(async move {
                                        race_engineer::api::handle_command(cmd, svc, bcast, audio_bcast).await;
                                    });
                                } else {
                                    let response = tokio::task::spawn_blocking(move || {
                                        dispatch_command(cmd)
                                    })
                                    .await;
                                    match response {
                                        Ok(msg) => {
                                            match serialize(&msg, fmt) {
                                                Ok(ws_msg) => {
                                                    if let Err(e) = sink.send(ws_msg).await {
                                                        debug!("Send to {} failed: {}", peer, e);
                                                        break;
                                                    }
                                                }
                                                Err(e) => {
                                                    warn!(
                                                        "Serialization error for {}: {}",
                                                        peer, e
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            warn!("Post-race task panicked for {}: {}", peer, e);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Unparseable client command from {}: {}", peer, e);
                            }
                        }
                    }
                    Some(Ok(_)) => {
                        // Binary and other frames ignored.
                    }
                }
            }
        }
    }

    // Finish the close handshake instead of just dropping the socket, so the
    // client sees a clean close (1000) rather than an abnormal one (1006) and
    // can tell "the bridge said goodbye" from "the link broke". Bounded: a
    // peer that is already gone must not hold this task open.
    let _ = tokio::time::timeout(Duration::from_secs(1), sink.close()).await;

    let prev = client_count.fetch_sub(1, Ordering::Relaxed);
    let new_count = prev.saturating_sub(1);
    let _ = count_tx.send(new_count);
    info!("Client {} disconnected ({} remaining)", peer, new_count);
}

// ---------------------------------------------------------------------------
// Access levels
// ---------------------------------------------------------------------------

const LEVEL_NONE: u8 = 0;
const LEVEL_LOCAL: u8 = 1;
const LEVEL_REMOTE: u8 = 2;

/// Commands a remote client may not send: they download and install programs
/// on the racing PC, delete files there, or make the bridge read a folder of
/// the caller's choosing.
fn is_local_only(cmd: &ClientCommand) -> bool {
    match cmd {
        ClientCommand::EngineerInstallPiper
        | ClientCommand::EngineerInstallVoice { .. }
        | ClientCommand::EngineerUninstallVoice { .. } => true,
        ClientCommand::PostRaceInit { results_path } => {
            results_path.as_deref().map(|p| !p.trim().is_empty()).unwrap_or(false)
        }
        _ => false,
    }
}

/// The error a refused command gets back, in the family the sender listens to.
fn local_only_refusal(cmd: &ClientCommand) -> ServerMessage {
    let message = "This action is only available on the racing PC itself.".to_string();
    match cmd {
        ClientCommand::PostRaceInit { .. } => ServerMessage::PostRaceError { message },
        _ => ServerMessage::EngineerError { message },
    }
}

// ---------------------------------------------------------------------------
// Command dispatcher
// ---------------------------------------------------------------------------

/// Route an incoming [`ClientCommand`] to the appropriate module handler.
///
/// Fuel-calculator commands go to `fuel_calculator::api`, all others to
/// `post_race::api`. Both handlers are synchronous and must be called from
/// `tokio::task::spawn_blocking`.
fn dispatch_command(cmd: ClientCommand) -> ServerMessage {
    match cmd {
        ClientCommand::FuelCalcInit | ClientCommand::FuelCalcCompute { .. } => {
            fuel_handle(cmd)
        }
        _ => post_race_handle(cmd),
    }
}

// ---------------------------------------------------------------------------
// Serialization helpers
// ---------------------------------------------------------------------------

fn serialize(msg: &ServerMessage, fmt: Format) -> Result<Message> {
    match fmt {
        Format::MsgPack => {
            let bytes = rmp_serde::to_vec_named(msg)?;
            Ok(Message::Binary(bytes.into()))
        }
        Format::Json => {
            let text = serde_json::to_string(msg)?;
            Ok(Message::Text(text.into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_and_custom_paths_are_local_only() {
        assert!(is_local_only(&ClientCommand::EngineerInstallPiper));
        assert!(is_local_only(&ClientCommand::EngineerInstallVoice { voice_id: "x".into() }));
        assert!(is_local_only(&ClientCommand::EngineerUninstallVoice { voice_id: "x".into() }));
        assert!(is_local_only(&ClientCommand::PostRaceInit {
            results_path: Some("C:\\Users".into())
        }));
    }

    #[test]
    fn reading_data_stays_open_to_remote_clients() {
        assert!(!is_local_only(&ClientCommand::PostRaceInit { results_path: None }));
        assert!(!is_local_only(&ClientCommand::PostRaceInit { results_path: Some("  ".into()) }));
        assert!(!is_local_only(&ClientCommand::PostRaceSessionDetail { session_id: 1 }));
        assert!(!is_local_only(&ClientCommand::EngineerGetStatus));
        assert!(!is_local_only(&ClientCommand::FuelCalcInit));
    }
}
