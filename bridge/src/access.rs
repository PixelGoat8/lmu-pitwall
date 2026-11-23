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
//! A hub also gives each person a link of their own (see `crate::users`), with
//! a role: the remote callers become *engineers* and *guests*, and the PC
//! itself is the *admin*. What each role may send is decided by
//! `crate::websocket::server`, on the server: hiding a button in the
//! dashboard is only a courtesy.
//!
//! # What loopback trust assumes
//!
//! "Local" is decided by the TCP peer address. A reverse proxy running on the
//! racing PC (for example `tailscale serve`) would make every remote request
//! look local. Do not put one in front of the bridge; connect to the port
//! directly.

use std::net::IpAddr;
use std::sync::Arc;

use serde::Serialize;

use crate::users::{User, UsersStore};

/// Query parameter and config key carrying the token.
pub const TOKEN_PARAM: &str = "token";

/// Environment variable that overrides the stored token.
pub const TOKEN_ENV: &str = "LMU_PITWALL_TOKEN";

/// What an accepted caller may do, from most to least.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Only watches (a hub's guest).
    #[default]
    Guest,
    /// Runs the race: everything but changing the PC's own settings. On a
    /// driver's bridge, whoever holds its token.
    Engineer,
    /// The PC itself (a loopback peer): full control.
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Guest => "guest",
            Role::Engineer => "engineer",
            Role::Admin => "admin",
        }
    }
}

/// Who a connection is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub role: Role,
    /// For the logs and, later, the change log: a person's name, or what the
    /// shared access is.
    pub name: String,
    /// The person in `users.json`; `None` for the PC itself and the shared
    /// token.
    pub user_id: Option<String>,
    /// Teams this caller may watch; `None` = all of them.
    pub teams: Option<Vec<String>>,
}

impl Caller {
    pub fn is_local(&self) -> bool {
        self.role == Role::Admin
    }

    /// May this caller watch team `id`?
    pub fn sees_team(&self, id: &str) -> bool {
        self.teams.as_ref().map(|t| t.iter().any(|x| x == id)).unwrap_or(true)
    }

    fn from_user(u: &User) -> Self {
        Caller {
            role: u.role.min(Role::Engineer),
            name: u.name.clone(),
            user_id: Some(u.id.clone()),
            teams: (u.role == Role::Guest).then(|| u.teams.clone()),
        }
    }
}

/// The access policy: loopback is the admin, the shared token an engineer,
/// and on a hub each person in `users.json` what their entry says.
pub struct Access {
    token: String,
    /// The hub's people; `None` on a driver's bridge.
    users: Option<Arc<UsersStore>>,
}

impl Access {
    pub fn new(token: String, users: Option<Arc<UsersStore>>) -> Self {
        Access { token, users }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Running as a hub (with people of its own).
    pub fn is_hub(&self) -> bool {
        self.users.is_some()
    }

    /// Decide who `peer` is, given the token it presented (if any). `None`
    /// means the caller is refused.
    pub fn authorize(&self, peer: IpAddr, presented: Option<&str>) -> Option<Caller> {
        if is_loopback(peer) {
            return Some(Caller {
                role: Role::Admin,
                name: if self.is_hub() { "hub PC".into() } else { "this PC".into() },
                user_id: None,
                teams: None,
            });
        }
        let presented = presented?;
        if constant_time_eq(presented.as_bytes(), self.token.as_bytes()) {
            return Some(Caller {
                role: Role::Engineer,
                name: if self.is_hub() { "hub link".into() } else { "shared link".into() },
                user_id: None,
                teams: None,
            });
        }
        self.users.as_ref()?.by_token(presented).map(|u| Caller::from_user(&u))
    }

    /// The caller as `users.json` says now: `None` once their access has been
    /// taken away. The PC and the shared token do not change.
    pub fn recheck(&self, caller: &Caller) -> Option<Caller> {
        match (&caller.user_id, &self.users) {
            (Some(id), Some(users)) => users.get(id).map(|u| Caller::from_user(&u)),
            (Some(_), None) => None,
            (None, _) => Some(caller.clone()),
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
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
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

    fn role(a: &Access, peer: &str, token: Option<&str>) -> Option<Role> {
        a.authorize(ip(peer), token).map(|c| c.role)
    }

    #[test]
    fn loopback_needs_no_token() {
        let a = Access::new("secret".into(), None);
        assert_eq!(role(&a, "127.0.0.1", None), Some(Role::Admin));
        assert_eq!(role(&a, "::1", None), Some(Role::Admin));
        assert_eq!(role(&a, "::ffff:127.0.0.1", Some("wrong")), Some(Role::Admin));
        assert!(a.authorize(ip("::1"), None).unwrap().is_local());
    }

    #[test]
    fn remote_needs_the_exact_token() {
        let a = Access::new("secret".into(), None);
        // LAN and Tailscale (100.64.0.0/10) addresses are both "remote".
        for peer in ["192.168.1.20", "100.101.102.103", "fd7a:115c:a1e0::1"] {
            assert_eq!(role(&a, peer, Some("secret")), Some(Role::Engineer));
            assert_eq!(role(&a, peer, Some("secre")), None);
            assert_eq!(role(&a, peer, Some("secret2")), None);
            assert_eq!(role(&a, peer, Some("")), None);
            assert_eq!(role(&a, peer, None), None);
        }
    }

    fn hub_with(json: &str) -> (tempfile::TempDir, Arc<UsersStore>, Access) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("users.json"), json).unwrap();
        let users = Arc::new(UsersStore::open(dir.path()));
        let access = Access::new("hubtoken".into(), Some(users.clone()));
        (dir, users, access)
    }

    #[test]
    fn on_a_hub_each_person_is_known_by_their_link() {
        let (_dir, users, a) = hub_with(
            r#"{"users":[
                {"id":"m","name":"Marco","role":"engineer","token":"marco-token-0123456789"},
                {"id":"g","name":"Gianni","role":"guest","teams":["clienti"],"token":"gianni-token-0123456789"}
            ]}"#,
        );
        let peer = "100.101.102.103";

        let marco = a.authorize(ip(peer), Some("marco-token-0123456789")).unwrap();
        assert_eq!((marco.role, marco.name.as_str(), marco.user_id.as_deref()), (Role::Engineer, "Marco", Some("m")));
        assert!(marco.sees_team("ufficiale") && marco.sees_team("clienti"));

        let gianni = a.authorize(ip(peer), Some("gianni-token-0123456789")).unwrap();
        assert_eq!(gianni.role, Role::Guest);
        assert!(gianni.sees_team("clienti"));
        assert!(!gianni.sees_team("ufficiale"));

        // The hub's own link is still an engineer's, for every team.
        let link = a.authorize(ip(peer), Some("hubtoken")).unwrap();
        assert_eq!((link.role, link.user_id.clone(), link.teams.clone()), (Role::Engineer, None, None));
        assert!(a.authorize(ip(peer), Some("nobody-0123456789abcdef")).is_none());

        // The checks again, as on every heartbeat: still there.
        assert_eq!(a.recheck(&marco), Some(marco.clone()));
        assert_eq!(a.recheck(&link), Some(link));
        assert!(users.get("m").is_some());
    }

    #[test]
    fn a_person_removed_from_the_file_loses_access() {
        let (dir, _users, a) = hub_with(r#"{"users":[{"id":"m","name":"Marco","role":"engineer","token":"marco-token-0123456789"}]}"#);
        let marco = a.authorize(ip("10.0.0.2"), Some("marco-token-0123456789")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(dir.path().join("users.json"), r#"{"users":[]}"#).unwrap();
        assert_eq!(a.recheck(&marco), None);
        assert!(a.authorize(ip("10.0.0.2"), Some("marco-token-0123456789")).is_none());
    }

    #[test]
    fn a_bridge_knows_no_people() {
        let a = Access::new("secret".into(), None);
        assert!(!a.is_hub());
        let ghost = Caller { role: Role::Engineer, name: "x".into(), user_id: Some("m".into()), teams: None };
        assert_eq!(a.recheck(&ghost), None);
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
