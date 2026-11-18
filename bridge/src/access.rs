//! Who may talk to the bridge.
//!
//! Upstream Pitwall served everyone who could reach the port. That is fine on a
//! home LAN and not fine once the bridge is reachable from other machines
//! (Tailscale, a shared network), so this fork splits callers in two:
//!
//! * **Local** — the connection comes from the loopback interface, i.e. a
//!   browser on the racing PC itself. No token needed, full control.
//! * **Remote** — everything else. Must present the access token, and gets
//!   the data stream but none of the commands that change the racing PC
//!   (shut down, change port, delete history, read arbitrary folders).
//!
//! The token is generated on first run and stored in `config.json`. The local
//! dashboard shows it as a ready-made link the driver sends to the engineer.
//!
//! # What loopback trust assumes
//!
//! "Local" is decided by the TCP peer address. A reverse proxy running on the
//! racing PC (for example `tailscale serve`) would make every remote request
//! look local. Do not put one in front of the bridge; connect to the port
//! directly.

use std::net::IpAddr;

/// Query parameter and config key carrying the token.
pub const TOKEN_PARAM: &str = "token";

/// Environment variable that overrides the stored token.
pub const TOKEN_ENV: &str = "LMU_PITWALL_TOKEN";

/// What an accepted caller is allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Loopback peer: full control.
    Local,
    /// Authenticated non-loopback peer: data and harmless commands only.
    Remote,
}

/// The bridge's access policy: one shared token, loopback exempt.
pub struct Access {
    token: String,
}

impl Access {
    pub fn new(token: String) -> Self {
        Access { token }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Decide what `peer` may do, given the token it presented (if any).
    /// `None` means the caller is refused.
    pub fn authorize(&self, peer: IpAddr, presented: Option<&str>) -> Option<Level> {
        if is_loopback(peer) {
            return Some(Level::Local);
        }
        match presented {
            Some(t) if constant_time_eq(t.as_bytes(), self.token.as_bytes()) => Some(Level::Remote),
            _ => None,
        }
    }
}

/// Loopback check that also covers `::ffff:127.0.0.1`.
pub fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().map(|v4| v4.is_loopback()).unwrap_or(false)
        }
    }
}

/// A fresh 128-bit token as 32 hex characters, from the OS random source.
pub fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Extract `token=…` from a URL query string (without the leading `?`).
pub fn token_from_query(query: &str) -> Option<&str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == TOKEN_PARAM && !v.is_empty()).then_some(v)
    })
}

/// Extract the token from a raw HTTP request head: `Authorization: Bearer …`
/// first, then the `?token=` query parameter of the request line.
pub fn token_from_request(request: &str) -> Option<&str> {
    let bearer = request.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case("authorization") {
            return None;
        }
        let value = value.trim();
        let (scheme, token) = value.split_once(' ')?;
        (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
    });
    if bearer.is_some() {
        return bearer;
    }
    let target = request.lines().next()?.split_whitespace().nth(1)?;
    let (_, query) = target.split_once('?')?;
    token_from_query(query)
}

/// True when the request carries no `Origin`, or one that is the racing PC
/// itself. Guards the control endpoints against a web page open in the
/// driver's browser posting to `localhost` behind their back.
pub fn origin_is_local(request: &str) -> bool {
    let origin = request.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim().eq_ignore_ascii_case("origin").then(|| value.trim())
    });
    let Some(origin) = origin else { return true };
    let host = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .unwrap_or(origin);
    let host = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().map(is_loopback).unwrap_or(false)
}

/// Compare without leaking, through timing, how many leading bytes matched.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn loopback_needs_no_token() {
        let a = Access::new("secret".into());
        assert_eq!(a.authorize(ip("127.0.0.1"), None), Some(Level::Local));
        assert_eq!(a.authorize(ip("::1"), None), Some(Level::Local));
        assert_eq!(a.authorize(ip("::ffff:127.0.0.1"), Some("wrong")), Some(Level::Local));
    }

    #[test]
    fn remote_needs_the_exact_token() {
        let a = Access::new("secret".into());
        // LAN and Tailscale (100.64.0.0/10) addresses are both "remote".
        for peer in ["192.168.1.20", "100.101.102.103", "fd7a:115c:a1e0::1"] {
            assert_eq!(a.authorize(ip(peer), Some("secret")), Some(Level::Remote));
            assert_eq!(a.authorize(ip(peer), Some("secre")), None);
            assert_eq!(a.authorize(ip(peer), Some("secret2")), None);
            assert_eq!(a.authorize(ip(peer), Some("")), None);
            assert_eq!(a.authorize(ip(peer), None), None);
        }
    }

    #[test]
    fn generated_tokens_are_long_and_differ() {
        let (a, b) = (generate_token(), generate_token());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn token_is_found_in_the_query() {
        assert_eq!(token_from_query("token=abc"), Some("abc"));
        assert_eq!(token_from_query("format=json&token=abc"), Some("abc"));
        assert_eq!(token_from_query("format=json"), None);
        assert_eq!(token_from_query("token="), None);
        assert_eq!(token_from_query("xtoken=abc"), None);
    }

    #[test]
    fn token_is_found_in_a_request_head() {
        let q = "GET /api/network-info?token=abc HTTP/1.1\r\nHost: x\r\n\r\n";
        assert_eq!(token_from_request(q), Some("abc"));
        let h = "GET /api/network-info HTTP/1.1\r\nauthorization: bearer  xyz \r\n\r\n";
        assert_eq!(token_from_request(h), Some("xyz"));
        let both = "GET /a?token=abc HTTP/1.1\r\nAuthorization: Bearer xyz\r\n\r\n";
        assert_eq!(token_from_request(both), Some("xyz"));
        assert_eq!(token_from_request("GET / HTTP/1.1\r\nHost: x\r\n\r\n"), None);
    }

    #[test]
    fn only_local_origins_pass_the_origin_check() {
        let req = |origin: &str| format!("POST /api/shutdown HTTP/1.1\r\nOrigin: {origin}\r\n\r\n");
        assert!(origin_is_local("POST /api/shutdown HTTP/1.1\r\nHost: x\r\n\r\n"));
        assert!(origin_is_local(&req("http://localhost:9000")));
        assert!(origin_is_local(&req("http://127.0.0.1:9000")));
        assert!(origin_is_local(&req("http://[::1]:9000")));
        assert!(!origin_is_local(&req("https://evil.example")));
        assert!(!origin_is_local(&req("http://192.168.1.20:9000")));
        assert!(!origin_is_local(&req("http://localhost.evil.example")));
        assert!(!origin_is_local(&req("null")));
    }
}
