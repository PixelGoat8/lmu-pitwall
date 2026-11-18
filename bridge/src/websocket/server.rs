use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    Arc, Mutex, RwLock,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::access::{self, Access, Level};
use crate::fuel_calculator::api::handle_command as fuel_handle;
use crate::hub::HubCommand;
use crate::post_race::api::handle_command as post_race_handle;
use crate::protocol::messages::{ClientCommand, ServerMessage};
use crate::team::TeamLink;

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

/// Commands about a hub's teams, with the team the client was watching (where
/// an error goes back to).
pub type TeamRequests = mpsc::UnboundedSender<(String, ClientCommand)>;

/// The WebSocket server of a Pitwall: a driver's bridge (one team, its own
/// car) or a hub (one team per car). Each client watches one team, chosen with
/// `?team=<id>` when it connects; without one, or with an unknown one, the
/// first team.
pub struct WebSocketServer {
    port: u16,
    teams: Arc<RwLock<Vec<Arc<TeamLink>>>>,
    /// Live count of connected WebSocket clients.
    client_count: Arc<AtomicUsize>,
    /// Watch channel — subscribers receive the new count on every change.
    count_tx: watch::Sender<usize>,
    /// Latest VersionInfo — sent immediately to newly connecting clients.
    version_info_rx: watch::Receiver<Option<ServerMessage>>,
    access: Arc<Access>,
    /// Where team commands go; `None` on a driver's bridge.
    team_requests: Option<TeamRequests>,
}

impl WebSocketServer {
    pub fn new(
        port: u16,
        version_info_rx: watch::Receiver<Option<ServerMessage>>,
        access: Arc<Access>,
        team_requests: Option<TeamRequests>,
    ) -> Self {
        let (count_tx, _) = watch::channel(0usize);
        WebSocketServer {
            port,
            teams: Arc::new(RwLock::new(Vec::new())),
            client_count: Arc::new(AtomicUsize::new(0)),
            count_tx,
            version_info_rx,
            access,
            team_requests,
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Subscribe to client-count changes.
    pub fn client_count_rx(&self) -> watch::Receiver<usize> {
        self.count_tx.subscribe()
    }

    pub fn add_team(&self, link: Arc<TeamLink>) {
        if let Ok(mut teams) = self.teams.write() {
            teams.retain(|t| t.id != link.id);
            teams.push(link);
        }
    }

    /// Stop serving a team. Its clients are disconnected once its stream
    /// closes (when its engines and coordinator are stopped).
    pub fn remove_team(&self, id: &str) {
        if let Ok(mut teams) = self.teams.write() {
            teams.retain(|t| t.id != id);
        }
    }

    pub fn teams(&self) -> Vec<Arc<TeamLink>> {
        self.teams.read().map(|t| t.clone()).unwrap_or_default()
    }

    fn first_team(&self) -> Arc<TeamLink> {
        self.teams().into_iter().next().expect("a Pitwall always has a team")
    }

    /// The first team's stream: a driver's bridge has only that one.
    pub fn broadcaster(&self) -> broadcast::Sender<Arc<ServerMessage>> {
        self.first_team().tx.clone()
    }

    /// Send a message to the first team's clients (all of a driver's bridge).
    pub fn broadcast(&self, msg: ServerMessage) -> usize {
        self.first_team().tx.send(Arc::new(msg)).unwrap_or(0)
    }

    /// Send a message to every client of every team.
    pub fn broadcast_all(&self, msg: ServerMessage) {
        let msg = Arc::new(msg);
        for t in self.teams() {
            let _ = t.tx.send(msg.clone());
        }
    }

    /// Accept a single WebSocket client from an already-accepted `TcpStream`.
    pub fn accept_client(&self, stream: TcpStream, peer: SocketAddr) {
        debug!("New WebSocket connection from {}", peer);
        // The client is counted only once its handshake has been accepted
        // (see `handle_client`): a refused connection must neither keep the
        // bridge alive nor arm the idle shutdown.
        tokio::spawn(handle_client(
            stream,
            peer,
            self.teams.clone(),
            self.client_count.clone(),
            self.count_tx.clone(),
            self.version_info_rx.clone(),
            self.access.clone(),
            self.team_requests.clone(),
        ));
    }
}

// ---------------------------------------------------------------------------
// Per-client handler
// ---------------------------------------------------------------------------

/// The `team=` value of a query string.
fn team_from_query(query: &str) -> Option<String> {
    query.split('&').find_map(|p| p.strip_prefix("team=")).map(str::to_string).filter(|t| !t.is_empty())
}

#[allow(clippy::too_many_arguments)]
async fn handle_client(
    stream: TcpStream,
    peer: SocketAddr,
    teams: Arc<RwLock<Vec<Arc<TeamLink>>>>,
    client_count: Arc<AtomicUsize>,
    count_tx: watch::Sender<usize>,
    version_info_rx: watch::Receiver<Option<ServerMessage>>,
    access: Arc<Access>,
    team_requests: Option<TeamRequests>,
) {
    let is_json = Arc::new(AtomicBool::new(false));
    let is_json_cb = is_json.clone();
    let level_flag = Arc::new(AtomicU8::new(LEVEL_NONE));
    let level_cb = level_flag.clone();
    let wanted_team: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let wanted_team_cb = wanted_team.clone();

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
            if let Ok(mut t) = wanted_team_cb.lock() {
                *t = req.uri().query().and_then(team_from_query);
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

    // The team this client watches: the one it asked for, or the first.
    let wanted = wanted_team.lock().ok().and_then(|t| t.clone());
    let team = {
        let list = teams.read().map(|t| t.clone()).unwrap_or_default();
        match list.iter().find(|t| Some(&t.id) == wanted.as_ref()).or(list.first()) {
            Some(t) => t.clone(),
            None => return,
        }
    };
    let mut rx = team.tx.subscribe();

    let count = client_count.fetch_add(1, Ordering::Relaxed) + 1;
    let _ = count_tx.send(count);

    let fmt = if is_json.load(Ordering::Relaxed) {
        Format::Json
    } else {
        Format::MsgPack
    };
    info!("Client {} connected ({:?}, format: {:?}, team: {}, {} total)", peer, level, fmt, team.id, count);

    let (mut sink, mut incoming) = ws.split();

    // The latest snapshots first, so the dashboard does not wait for the next
    // one: every car (until the next car crosses the line), the version check,
    // and whether the game is running (never "Waiting" when it already was).
    // (Cloned out first: a watch borrow must not be held across a send.)
    let snapshots: Vec<ServerMessage> = [
        team.all_drivers_rx.borrow().clone(),
        version_info_rx.borrow().clone(),
        team.connection_status_rx.borrow().clone(),
    ]
    .into_iter()
    .flatten()
    .collect();
    for latest in snapshots {
        if let Ok(ws_msg) = serialize(&latest, fmt) {
            let _ = sink.send(ws_msg).await;
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
                                // Commands that change the racing PC, or a hub's
                                // teams and drivers, are for its own browser only.
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

                                if let ClientCommand::FollowCar { car_name } = cmd {
                                    if team.hub_tx.is_some() {
                                        let msg = ServerMessage::HubError {
                                            message: "Choose the car on the Pitwall connected to the game (on the engineer's PC: http://localhost:9000).".into(),
                                        };
                                        if let Ok(ws_msg) = serialize(&msg, fmt) {
                                            let _ = sink.send(ws_msg).await;
                                        }
                                    } else {
                                        info!("Following car: {:?}", car_name);
                                        crate::follow::set(car_name);
                                    }
                                    continue;
                                }
                                // Strategy settings: the team's engine answers
                                // with a fresh StrategyUpdate to its clients.
                                if let ClientCommand::StrategyConfig { config } = cmd {
                                    let _ = team.strategy_tx.send(config);
                                    continue;
                                }
                                // AI: settings and "ask now". Remote clients hold
                                // the token, so they are the team; the key and its
                                // spending stay on the PC that runs this.
                                match cmd {
                                    ClientCommand::AiConfig { config } => {
                                        let _ = team.ai_tx.send(crate::ai::AiCommand::Config(config));
                                        continue;
                                    }
                                    ClientCommand::AiAsk { question } => {
                                        let _ = team.ai_tx.send(crate::ai::AiCommand::Ask(question));
                                        continue;
                                    }
                                    _ => {}
                                }

                                // Hub commands go to the team's coordinator, which
                                // answers by broadcasting the new HubStatus; team
                                // commands to the hub's team keeper.
                                let not_a_hub = || ServerMessage::HubError {
                                    message: "This Pitwall is not running as a hub (start it with --hub).".into(),
                                };
                                if let Some(hub_cmd) = HubCommand::from_client(&cmd) {
                                    match &team.hub_tx {
                                        Some(tx) => {
                                            let _ = tx.send(hub_cmd);
                                        }
                                        None => {
                                            if let Ok(ws_msg) = serialize(&not_a_hub(), fmt) {
                                                let _ = sink.send(ws_msg).await;
                                            }
                                        }
                                    }
                                    continue;
                                }
                                if is_team_command(&cmd) {
                                    match &team_requests {
                                        Some(tx) => {
                                            let _ = tx.send((team.id.clone(), cmd));
                                        }
                                        None => {
                                            if let Ok(ws_msg) = serialize(&not_a_hub(), fmt) {
                                                let _ = sink.send(ws_msg).await;
                                            }
                                        }
                                    }
                                    continue;
                                }

                                let dir = team.dir.clone();
                                let response = tokio::task::spawn_blocking(move || {
                                    dispatch_command(cmd, &dir)
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

/// Commands a remote client may not send: they delete files on the racing PC,
/// make the bridge read a folder of the caller's choosing, or change which
/// drivers and teams a hub follows (the hub's owner does that).
fn is_local_only(cmd: &ClientCommand) -> bool {
    match cmd {
        ClientCommand::PostRaceInit { results_path } => {
            results_path.as_deref().map(|p| !p.trim().is_empty()).unwrap_or(false)
        }
        // Deleting a session destroys data on this PC.
        ClientCommand::HistoryDelete { .. } => true,
        ClientCommand::HubAddSource { .. } | ClientCommand::HubRemoveSource { .. } => true,
        cmd if is_team_command(cmd) => true,
        _ => false,
    }
}

/// Commands about a hub's teams, handled by its team keeper.
fn is_team_command(cmd: &ClientCommand) -> bool {
    matches!(
        cmd,
        ClientCommand::HubMoveSource { .. }
            | ClientCommand::TeamCreate { .. }
            | ClientCommand::TeamUpdate { .. }
            | ClientCommand::TeamDelete { .. }
    )
}

/// The error a refused command gets back, in the family the sender listens to.
fn local_only_refusal(cmd: &ClientCommand) -> ServerMessage {
    let message = "This action is only available on the racing PC itself.".to_string();
    match cmd {
        ClientCommand::HistoryDelete { .. } => ServerMessage::HistoryError { message },
        ClientCommand::HubAddSource { .. } | ClientCommand::HubRemoveSource { .. } => ServerMessage::HubError { message },
        cmd if is_team_command(cmd) => ServerMessage::HubError { message },
        _ => ServerMessage::PostRaceError { message },
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
fn dispatch_command(cmd: ClientCommand, dir: &std::path::Path) -> ServerMessage {
    if crate::history::is_history_command(&cmd) {
        return crate::history::handle_command(cmd, dir);
    }
    if crate::track_store::is_track_command(&cmd) {
        return crate::track_store::handle_command(cmd);
    }
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
        assert!(is_local_only(&ClientCommand::PostRaceInit {
            results_path: Some("C:\\Users".into())
        }));
    }

    #[test]
    fn a_hubs_drivers_and_teams_are_managed_locally() {
        assert!(is_local_only(&ClientCommand::HubAddSource { link: "x".into(), label: None }));
        assert!(is_local_only(&ClientCommand::HubRemoveSource { id: "x".into() }));
        assert!(is_local_only(&ClientCommand::HubMoveSource { source_id: "x".into(), team_id: "y".into() }));
        assert!(is_local_only(&ClientCommand::TeamCreate { name: "Clienti".into(), color: None }));
        assert!(is_local_only(&ClientCommand::TeamDelete { id: "x".into() }));
        // Pinning a driver is the engineers' call, from anywhere.
        assert!(!is_local_only(&ClientCommand::HubFollow { id: None }));
        assert_eq!(team_from_query("format=json&team=clienti&token=x").as_deref(), Some("clienti"));
        assert_eq!(team_from_query("token=x&team="), None);
    }

    #[test]
    fn reading_data_stays_open_to_remote_clients() {
        assert!(!is_local_only(&ClientCommand::PostRaceInit { results_path: None }));
        assert!(!is_local_only(&ClientCommand::PostRaceInit { results_path: Some("  ".into()) }));
        assert!(!is_local_only(&ClientCommand::PostRaceSessionDetail { session_id: 1 }));
        assert!(!is_local_only(&ClientCommand::FuelCalcInit));
    }
}
