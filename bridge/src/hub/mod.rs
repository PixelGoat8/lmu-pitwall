//! Hub mode: one Pitwall for the engineer, fed by the drivers' bridges.
//!
//! In an endurance race the car is driven in turns, each driver on their own
//! PC, and only the PC that is driving has the car's full telemetry (measured:
//! online, LMU hides other cars' fuel, tyre wear and temperatures). So every
//! driver runs a bridge, and the engineer runs this hub (`--hub`), which:
//!
//! * connects to each driver's bridge as an ordinary remote client, using the
//!   link (address + token) that bridge's dashboard shows;
//! * works out which bridge is driving right now and relays *that* stream to
//!   its own clients — who see a normal Pitwall that silently changes PC at
//!   every driver swap;
//! * keeps the stint history of the session across those swaps.
//!
//! The relayed messages are passed through untouched ([`ServerMessage::Raw`]),
//! so a hub works with bridges newer than itself. The decisions are in
//! [`logic`], free of I/O; this file is the plumbing around them.
//!
//! # What the hub does not do (yet)
//!
//! Commands a dashboard sends (post-race results, fuel calculator) are answered
//! from the hub's own database, not forwarded to a driver's bridge.

pub mod logic;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use rmpv::Value;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::{self, Message};
use tracing::{debug, info, warn};

use crate::access;
use crate::protocol::messages::{ClientCommand, HubSourceStatus, HubStintInfo, ServerMessage};
use crate::recording::{field, message_type};
use crate::websocket::server::WebSocketServer;
use logic::{choose_source, Candidate, ScoringSummary, StintTracker};

/// Default port of a hub, so it can run beside a bridge on the same PC.
pub const DEFAULT_PORT: u16 = 9100;

/// A bridge that sends nothing for this long is treated as gone. Bridges
/// heartbeat every 2 s, so this is three missed beats.
const SOURCE_SILENCE_TIMEOUT: Duration = Duration::from_secs(7);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Our own keep-alive towards the bridge, which drops clients silent for 20 s.
const PING_INTERVAL: Duration = Duration::from_secs(5);
const RETRY_MIN: Duration = Duration::from_millis(500);
const RETRY_MAX: Duration = Duration::from_secs(5);

/// Message types relayed from the followed bridge. Everything else a bridge
/// sends is either its own business (heartbeats, version, engineer audio) or a
/// reply to a command the hub never sent.
const RELAYED: [&str; 7] = [
    "ConnectionStatus",
    "SessionInfo",
    "AllDriversUpdate",
    "ScoringUpdate",
    "TelemetryUpdate",
    "ElectronicsUpdate",
    "VehicleStatusUpdate",
];

// ---------------------------------------------------------------------------
// Configuration (hub.json)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    pub id: String,
    pub label: String,
    /// WebSocket URL of the bridge, token included. Never logged.
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HubConfig {
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    /// Id of the bridge the engineer pinned; `None` = follow the driver.
    #[serde(default)]
    pub follow: Option<String>,
}

impl HubConfig {
    fn path() -> PathBuf {
        let base = std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| PathBuf::from(h).join(".config"))
                    .unwrap_or_else(|_| PathBuf::from("."))
            });
        base.join("LMUPitwall").join("hub.json")
    }

    pub fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let path = Self::path();
        let write = || -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default())?;
            std::fs::rename(&tmp, &path)
        };
        if let Err(e) = write() {
            warn!("Could not save hub.json: {}", e);
        }
    }
}

/// Turn the link a bridge's dashboard shows (`http://host:9000/?token=…`)
/// into the WebSocket URL to connect to, plus the host for display.
pub fn parse_link(link: &str) -> Result<(String, String)> {
    let link = link.trim();
    let rest = ["http://", "https://", "ws://", "wss://"]
        .iter()
        .find_map(|p| link.strip_prefix(p))
        .unwrap_or(link);
    let (authority, tail) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if authority.is_empty() || authority.contains(char::is_whitespace) {
        return Err(anyhow!("that does not look like a Pitwall link"));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() => {
            (h, p.parse::<u16>().map_err(|_| anyhow!("the port in the link is not a number"))?)
        }
        Some(_) => return Err(anyhow!("the link has no address")),
        None => (authority, 9000),
    };
    let query = tail.split_once('?').map(|(_, q)| q).unwrap_or("");
    let token = access::token_from_query(query)
        .ok_or_else(|| anyhow!("the link has no token (copy it from the 🔗 button of the driver's Pitwall)"))?;
    Ok((format!("ws://{host}:{port}/?token={token}"), host.to_string()))
}

fn host_of(ws_url: &str) -> String {
    ws_url
        .strip_prefix("ws://")
        .and_then(|r| r.split(['/', '?', ':']).next())
        .unwrap_or("?")
        .to_string()
}

// ---------------------------------------------------------------------------
// Commands from the dashboard
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum HubCommand {
    AddSource { link: String, label: Option<String> },
    RemoveSource { id: String },
    Follow { id: Option<String> },
}

impl HubCommand {
    pub fn from_client(cmd: &ClientCommand) -> Option<Self> {
        match cmd {
            ClientCommand::HubAddSource { link, label } => {
                Some(HubCommand::AddSource { link: link.clone(), label: label.clone() })
            }
            ClientCommand::HubRemoveSource { id } => Some(HubCommand::RemoveSource { id: id.clone() }),
            ClientCommand::HubFollow { id } => Some(HubCommand::Follow { id: id.clone() }),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// One bridge connection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    Connecting,
    Connected,
    Unauthorized,
    Unreachable,
}

impl Link {
    fn as_str(self) -> &'static str {
        match self {
            Link::Connecting => "connecting",
            Link::Connected => "connected",
            Link::Unauthorized => "unauthorized",
            Link::Unreachable => "unreachable",
        }
    }
}

enum Event {
    Link(String, Link),
    Frame(String, Value),
}

/// Keep one bridge connected for as long as the task lives, reporting link
/// changes and every message it sends.
async fn source_task(id: String, url: String, events: mpsc::UnboundedSender<Event>) {
    let mut retry = RETRY_MIN;
    loop {
        if events.send(Event::Link(id.clone(), Link::Connecting)).is_err() {
            return;
        }
        let outcome = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(&url)).await;
        let after = match outcome {
            Ok(Ok((ws, _))) => {
                retry = RETRY_MIN;
                let _ = events.send(Event::Link(id.clone(), Link::Connected));
                pump(&id, ws, &events).await;
                Link::Unreachable
            }
            Ok(Err(tungstenite::Error::Http(resp))) if resp.status().as_u16() == 401 => Link::Unauthorized,
            Ok(Err(e)) => {
                debug!("Hub source {}: connect failed: {}", id, e);
                Link::Unreachable
            }
            Err(_) => Link::Unreachable,
        };
        if events.send(Event::Link(id.clone(), after)).is_err() {
            return;
        }
        tokio::time::sleep(retry).await;
        retry = (retry * 2).min(RETRY_MAX);
    }
}

/// Read one connected bridge until it closes, errors or goes silent.
async fn pump<S>(id: &str, ws: tokio_tungstenite::WebSocketStream<S>, events: &mpsc::UnboundedSender<Event>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut sink, mut stream) = ws.split();
    // We only watch: do not send us the engineer's synthesized audio.
    let _ = sink
        .send(Message::Text(r#"{"command":"engineer_register_client_role","role":"display_only"}"#.into()))
        .await;

    let mut ping = tokio::time::interval(PING_INTERVAL);
    let mut last_frame = Instant::now();
    loop {
        tokio::select! {
            _ = ping.tick() => {
                if last_frame.elapsed() > SOURCE_SILENCE_TIMEOUT {
                    debug!("Hub source {}: silent, reconnecting", id);
                    return;
                }
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return;
                }
            }
            frame = tokio::time::timeout(SOURCE_SILENCE_TIMEOUT, stream.next()) => {
                let msg = match frame {
                    Ok(Some(Ok(m))) => m,
                    _ => return, // closed, error, or silent
                };
                last_frame = Instant::now();
                let value = match msg {
                    Message::Binary(bytes) => rmpv::decode::read_value(&mut &bytes[..]).ok(),
                    Message::Text(text) => serde_json::from_str::<serde_json::Value>(&text)
                        .ok()
                        .and_then(|j| rmpv::ext::to_value(j).ok()),
                    Message::Close(_) => return,
                    _ => None,
                };
                if let Some(v) = value {
                    if events.send(Event::Frame(id.to_string(), v)).is_err() {
                        return;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The coordinator
// ---------------------------------------------------------------------------

struct Source {
    cfg: SourceConfig,
    host: String,
    task: tokio::task::JoinHandle<()>,
    link: Link,
    game_connected: bool,
    session_active: bool,
    summary: Option<ScoringSummary>,
    /// Session time at which this bridge started driving.
    driving_since: f64,
    last_frame: Option<Instant>,
    /// Latest message of each relayed type, replayed to clients when the hub
    /// switches to this bridge.
    cache: HashMap<String, Value>,
}

impl Source {
    fn driving(&self) -> bool {
        self.session_active && self.summary.as_ref().map(|s| s.driving()).unwrap_or(false)
    }

    fn clear_live(&mut self) {
        self.game_connected = false;
        self.session_active = false;
        self.summary = None;
        self.cache.clear();
    }
}

struct Hub {
    ws: Arc<WebSocketServer>,
    events_tx: mpsc::UnboundedSender<Event>,
    sources: Vec<Source>,
    pinned: Option<String>,
    active: Option<String>,
    stints: StintTracker,
    connection_status_tx: watch::Sender<Option<ServerMessage>>,
    all_drivers_tx: watch::Sender<Option<ServerMessage>>,
    busy_tx: watch::Sender<bool>,
}

fn no_session_status() -> ServerMessage {
    ServerMessage::ConnectionStatus {
        game_connected: false,
        plugin_version: String::new(),
        telemetry_valid: true,
        telemetry_warning: None,
        session_active: false,
    }
}

impl Hub {
    fn spawn_source(&self, cfg: SourceConfig) -> Source {
        let task = tokio::spawn(source_task(cfg.id.clone(), cfg.url.clone(), self.events_tx.clone()));
        Source {
            host: host_of(&cfg.url),
            cfg,
            task,
            link: Link::Connecting,
            game_connected: false,
            session_active: false,
            summary: None,
            driving_since: 0.0,
            last_frame: None,
            cache: HashMap::new(),
        }
    }

    fn save(&self) {
        HubConfig {
            sources: self.sources.iter().map(|s| s.cfg.clone()).collect(),
            follow: self.pinned.clone(),
        }
        .save();
    }

    fn status_message(&self) -> ServerMessage {
        ServerMessage::HubStatus {
            sources: self
                .sources
                .iter()
                .map(|s| HubSourceStatus {
                    id: s.cfg.id.clone(),
                    label: s.cfg.label.clone(),
                    host: s.host.clone(),
                    link: s.link.as_str().to_string(),
                    game_connected: s.game_connected,
                    session_active: s.session_active,
                    pc_player: s.summary.as_ref().map(|x| x.pc_player.clone()).unwrap_or_default(),
                    driver_name: s.summary.as_ref().map(|x| x.driver_name.clone()).unwrap_or_default(),
                    control: s.summary.as_ref().and_then(|x| x.control).map(|c| c as i8),
                    driving: s.driving(),
                    in_pits: s.summary.as_ref().map(|x| x.in_pits).unwrap_or(false),
                    last_frame_age_ms: s.last_frame.map(|t| t.elapsed().as_millis() as u64),
                })
                .collect(),
            active_id: self.active.clone(),
            follow: self.pinned.clone(),
        }
    }

    fn stints_message(&self) -> ServerMessage {
        ServerMessage::HubStints {
            stints: self
                .stints
                .stints()
                .iter()
                .map(|s| HubStintInfo {
                    number: s.number,
                    driver: s.driver.clone(),
                    source: s.source.clone(),
                    start_time: s.start_time,
                    end_time: s.end_time,
                    start_lap: s.start_lap as i32,
                    end_lap: s.end_lap as i32,
                    laps: s.lap_times.len() as u32,
                    best_lap: s.best_lap(),
                    avg_lap: s.avg_lap(),
                    last_lap: s.lap_times.last().copied(),
                    fuel_start: s.fuel_start,
                    fuel_end: s.fuel_end,
                    ve_start: s.ve_start,
                    ve_end: s.ve_end,
                })
                .collect(),
        }
    }

    /// Send a relayed message to the hub's clients, keeping the on-connect
    /// snapshots current.
    fn relay(&self, msg: &Value) {
        let wrapped = ServerMessage::Raw(msg.clone());
        match message_type(msg) {
            Some("ConnectionStatus") => {
                let _ = self.connection_status_tx.send(Some(wrapped.clone()));
            }
            Some("AllDriversUpdate") => {
                let _ = self.all_drivers_tx.send(Some(wrapped.clone()));
            }
            _ => {}
        }
        self.ws.broadcast(wrapped);
    }

    /// Re-run the selection; on a change, bring clients onto the new bridge.
    fn reselect(&mut self) {
        let cands: Vec<Candidate> = self
            .sources
            .iter()
            .map(|s| Candidate {
                id: s.cfg.id.clone(),
                linked: s.link == Link::Connected,
                session_active: s.session_active,
                driving: s.driving(),
                driving_since: s.driving_since,
            })
            .collect();
        let chosen = choose_source(&cands, self.active.as_deref(), self.pinned.as_deref());
        let _ = self.busy_tx.send(self.sources.iter().any(|s| s.session_active));
        if chosen == self.active {
            return;
        }

        let label = |id: &Option<String>| {
            id.as_ref()
                .and_then(|i| self.sources.iter().find(|s| &s.cfg.id == i))
                .map(|s| s.cfg.label.clone())
                .unwrap_or_else(|| "nobody".into())
        };
        info!("Hub: following {} (was {})", label(&chosen), label(&self.active));
        self.active = chosen;

        // Replay the new bridge's latest state so clients are current at once
        // instead of waiting up to a second for each message type.
        match self.active.as_ref().and_then(|i| self.sources.iter().find(|s| &s.cfg.id == i)) {
            Some(src) => {
                match src.cache.get("ConnectionStatus") {
                    Some(cs) => self.relay(cs),
                    None => {
                        let m = no_session_status();
                        let _ = self.connection_status_tx.send(Some(m.clone()));
                        self.ws.broadcast(m);
                    }
                }
                // Not the telemetry: it arrives many times a second anyway,
                // and the cached frame may predate this bridge taking the
                // wheel, when it still carried the placeholder fuel and tyres
                // LMU shows for a car driven from elsewhere.
                for ty in RELAYED.iter().filter(|t| !matches!(**t, "ConnectionStatus" | "TelemetryUpdate")) {
                    if let Some(m) = src.cache.get(*ty) {
                        self.relay(m);
                    }
                }
            }
            None => {
                let m = no_session_status();
                let _ = self.connection_status_tx.send(Some(m.clone()));
                let _ = self.all_drivers_tx.send(None);
                self.ws.broadcast(m);
            }
        }
        self.ws.broadcast(self.status_message());
    }

    fn on_link(&mut self, id: &str, link: Link) {
        let Some(src) = self.sources.iter_mut().find(|s| s.cfg.id == id) else { return };
        if src.link == link {
            return;
        }
        // "connecting" between retries of a bridge known to be down or
        // refusing us is noise: keep showing the reason it is not connected.
        if link == Link::Connecting && matches!(src.link, Link::Unauthorized | Link::Unreachable) {
            return;
        }
        info!("Hub: {} ({}) is {}", src.cfg.label, src.host, link.as_str());
        src.link = link;
        if link != Link::Connected {
            src.clear_live();
        }
        self.reselect();
        self.ws.broadcast(self.status_message());
    }

    fn on_frame(&mut self, id: &str, msg: Value) {
        let Some(ty) = message_type(&msg).map(str::to_string) else { return };
        let Some(idx) = self.sources.iter().position(|s| s.cfg.id == id) else { return };
        let is_active = self.active.as_deref() == Some(id);

        let mut reselect = false;
        {
            let src = &mut self.sources[idx];
            src.last_frame = Some(Instant::now());
            match ty.as_str() {
                "ConnectionStatus" => {
                    let game = field(&msg, "game_connected").and_then(|v| v.as_bool()).unwrap_or(false);
                    // Bridges older than session_active stream whenever the game runs.
                    let session = field(&msg, "session_active").and_then(|v| v.as_bool()).unwrap_or(game);
                    reselect = game != src.game_connected || session != src.session_active;
                    src.game_connected = game;
                    src.session_active = session;
                    if !session {
                        src.summary = None;
                        src.cache.clear();
                    }
                }
                "ScoringUpdate" => {
                    let was_driving = src.driving();
                    src.summary = ScoringSummary::from_msg(&msg);
                    let now_driving = src.driving();
                    if now_driving && !was_driving {
                        src.driving_since = src.summary.as_ref().map(|s| s.session_time).unwrap_or(0.0);
                    }
                    reselect = now_driving != was_driving;
                }
                _ => {}
            }
            if RELAYED.contains(&ty.as_str()) {
                src.cache.insert(ty.clone(), msg.clone());
            }
        }
        if reselect {
            self.reselect();
        }
        // The selection may just have moved to this bridge, in which case its
        // cache (this message included) has already been replayed.
        let now_active = self.active.as_deref() == Some(id);
        if !now_active || !RELAYED.contains(&ty.as_str()) {
            return;
        }
        if is_active {
            self.relay(&msg);
        }

        // Stint history follows the relayed stream.
        let src = &self.sources[idx];
        match ty.as_str() {
            "ScoringUpdate" => {
                if let Some(summary) = &src.summary {
                    let label = src.cfg.label.clone();
                    if self.stints.on_scoring(&label, summary) {
                        self.ws.broadcast(self.stints_message());
                    }
                }
            }
            "TelemetryUpdate" if src.driving() => {
                if let Some(fuel) = field(&msg, "fuel").and_then(logic::num) {
                    self.stints.on_fuel(fuel);
                }
            }
            _ => {}
        }
    }

    fn on_command(&mut self, cmd: HubCommand) {
        match cmd {
            HubCommand::AddSource { link, label } => match parse_link(&link) {
                Ok((url, host)) => {
                    if self.sources.iter().any(|s| s.cfg.url == url) {
                        self.error("That bridge is already in the list.");
                        return;
                    }
                    let label = label
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .unwrap_or_else(|| host.clone());
                    let id = access::generate_token()[..8].to_string();
                    info!("Hub: added bridge {} ({})", label, host);
                    let src = self.spawn_source(SourceConfig { id, label, url });
                    self.sources.push(src);
                    self.save();
                }
                Err(e) => self.error(&format!("Could not add the bridge: {e}.")),
            },
            HubCommand::RemoveSource { id } => {
                if let Some(i) = self.sources.iter().position(|s| s.cfg.id == id) {
                    let src = self.sources.remove(i);
                    src.task.abort();
                    info!("Hub: removed bridge {}", src.cfg.label);
                    if self.pinned.as_deref() == Some(id.as_str()) {
                        self.pinned = None;
                    }
                    self.save();
                    self.reselect();
                }
            }
            HubCommand::Follow { id } => {
                self.pinned = id.filter(|i| self.sources.iter().any(|s| &s.cfg.id == i));
                self.save();
                self.reselect();
            }
        }
        self.ws.broadcast(self.status_message());
    }

    fn error(&self, message: &str) {
        self.ws.broadcast(ServerMessage::HubError { message: message.to_string() });
    }
}

/// Run the hub until the process exits.
///
/// `busy_tx` is set while any bridge has a live session; the idle shutdown
/// uses it the way a bridge uses "the game is running".
pub async fn run(
    ws: Arc<WebSocketServer>,
    mut commands: mpsc::UnboundedReceiver<HubCommand>,
    connection_status_tx: watch::Sender<Option<ServerMessage>>,
    all_drivers_tx: watch::Sender<Option<ServerMessage>>,
    busy_tx: watch::Sender<bool>,
) {
    let cfg = HubConfig::load();
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let mut hub = Hub {
        ws,
        events_tx,
        sources: Vec::new(),
        pinned: cfg.follow.clone(),
        active: None,
        stints: StintTracker::new(),
        connection_status_tx,
        all_drivers_tx,
        busy_tx,
    };
    info!("Hub mode: {} bridge(s) configured", cfg.sources.len());
    for sc in cfg.sources {
        let src = hub.spawn_source(sc);
        hub.sources.push(src);
    }

    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut ticks = 0u64;
    loop {
        tokio::select! {
            Some(ev) = events.recv() => match ev {
                Event::Link(id, link) => hub.on_link(&id, link),
                Event::Frame(id, msg) => hub.on_frame(&id, msg),
            },
            Some(cmd) = commands.recv() => hub.on_command(cmd),
            _ = tick.tick() => {
                // Periodic, so a dashboard that has just connected is current
                // within a second without the hub tracking who has seen what.
                ticks += 1;
                hub.ws.broadcast(hub.status_message());
                if ticks % 5 == 0 {
                    hub.ws.broadcast(hub.stints_message());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dashboard_link_becomes_a_websocket_url() {
        let (url, host) = parse_link("http://100.81.2.3:9000/?token=abc123").unwrap();
        assert_eq!(url, "ws://100.81.2.3:9000/?token=abc123");
        assert_eq!(host, "100.81.2.3");
        // Pasted with spaces, another port, extra parameters, a machine name.
        let (url, host) = parse_link("  http://marco-pc:9005/?format=json&token=t0k  ").unwrap();
        assert_eq!(url, "ws://marco-pc:9005/?token=t0k");
        assert_eq!(host, "marco-pc");
        // No port: the bridge default.
        assert_eq!(parse_link("ws://10.0.0.5/?token=x").unwrap().0, "ws://10.0.0.5:9000/?token=x");
        assert_eq!(host_of("ws://100.81.2.3:9000/?token=abc"), "100.81.2.3");
    }

    #[test]
    fn links_without_a_token_or_host_are_refused() {
        assert!(parse_link("http://100.81.2.3:9000/").is_err());
        assert!(parse_link("http://100.81.2.3:9000/?token=").is_err());
        assert!(parse_link("http://:9000/?token=abc").is_err());
        assert!(parse_link("http://host:port/?token=abc").is_err());
        assert!(parse_link("").is_err());
        assert!(parse_link("hello world").is_err());
    }
}
