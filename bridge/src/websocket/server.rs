use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
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

use crate::access::{self, Access, Caller, Role};
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
    let caller_slot: Arc<Mutex<Option<Caller>>> = Arc::new(Mutex::new(None));
    let caller_cb = caller_slot.clone();
    let access_cb = access.clone();
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
            match access_cb.authorize(peer.ip(), presented) {
                Some(caller) => {
                    if let Ok(mut c) = caller_cb.lock() {
                        *c = Some(caller);
                    }
                }
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
                if caller_slot.lock().map(|c| c.is_none()).unwrap_or(true) {
                    warn!("Refused WebSocket client {}: invalid or missing token", peer);
                } else {
                    warn!("WebSocket handshake failed for {}: {}", peer, e);
                }
                return;
            }
        };
    let Some(mut caller) = caller_slot.lock().ok().and_then(|c| c.clone()) else { return };
    let (mut sink, mut incoming) = ws.split();
    let fmt = if is_json.load(Ordering::Relaxed) {
        Format::Json
    } else {
        Format::MsgPack
    };

    // The team this client watches: the one it asked for, or the first, among
    // those it may see.
    let wanted = wanted_team.lock().ok().and_then(|t| t.clone());
    let team = {
        let list: Vec<Arc<TeamLink>> =
            teams.read().map(|t| t.clone()).unwrap_or_default().into_iter().filter(|t| caller.sees_team(&t.id)).collect();
        match list.iter().find(|t| Some(&t.id) == wanted.as_ref()).or(list.first()) {
            Some(t) => t.clone(),
            None => {
                warn!("Client {} ({}) has no team to watch", peer, caller.name);
                let msg = ServerMessage::Refused { message: NO_TEAM.into() };
                if let Ok(ws_msg) = serialize(&msg, fmt) {
                    let _ = sink.send(ws_msg).await;
                }
                let _ = tokio::time::timeout(Duration::from_secs(1), sink.close()).await;
                return;
            }
        }
    };
    let mut rx = team.tx.subscribe();

    let count = client_count.fetch_add(1, Ordering::Relaxed) + 1;
    let _ = count_tx.send(count);

    info!(
        "Client {} connected ({}, {}, format: {:?}, team: {}, {} total)",
        peer, caller.name, caller.role.as_str(), fmt, team.id, count
    );

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
                // Its team was deleted on the hub: let the dashboard reconnect
                // and land on a team that exists.
                if !teams.read().map(|t| t.iter().any(|x| x.id == team.id)).unwrap_or(true) {
                    info!("Client {}: team {} is gone, closing", peer, team.id);
                    break;
                }
                // Access taken away, or this team no longer shared with them:
                // say so and close.
                match access.recheck(&caller) {
                    Some(now) if now.sees_team(&team.id) => caller = now,
                    now => {
                        info!("Client {} ({}): access to team {} removed, closing", peer, caller.name, team.id);
                        let message = if now.is_some() { NO_TEAM } else { ACCESS_REMOVED };
                        if let Ok(ws_msg) = serialize(&ServerMessage::Refused { message: message.into() }, fmt) {
                            let _ = sink.send(ws_msg).await;
                        }
                        break;
                    }
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
                        let msg = visible_to(&caller, msg);
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
                                // Who may send what is decided here, whatever
                                // the dashboard shows (see `required_role`).
                                let hub = access.is_hub();
                                let needed = required_role(&cmd, hub);
                                if caller.role < needed {
                                    // A guest's dashboard saves the map it
                                    // records on its own: nothing to tell them.
                                    if matches!(cmd, ClientCommand::TrackOutlineSave { .. }) {
                                        continue;
                                    }
                                    warn!("Refused {} from {} ({}, {})", command_name(&text), peer, caller.name, caller.role.as_str());
                                    let refusal = refusal(&cmd, needed, hub);
                                    if let Ok(ws_msg) = serialize(&refusal, fmt) {
                                        if sink.send(ws_msg).await.is_err() {
                                            break;
                                        }
                                    }
                                    continue;
                                }
                                if changes_the_race(&cmd) {
                                    info!("{} by {} ({})", command_name(&text), caller.name, peer);
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
// Permissions
// ---------------------------------------------------------------------------

const NO_TEAM: &str = "No team is shared with you on this hub yet: ask the hub's admin.";
const ACCESS_REMOVED: &str = "Your access to this hub has been removed: ask the hub's admin for a new link.";

/// The least role that may send `cmd`.
///
/// | | Guest | Engineer | Admin (the PC itself) |
/// |---|---|---|---|
/// | Read: history, results, fuel calculator, map | ✓ | ✓ | ✓ |
/// | Strategy and targets, pin a driver, ask the AI | | ✓ | ✓ |
/// | AI settings and budget (hub) | | | ✓ |
/// | Teams, drivers, people (hub) | | | ✓ |
/// | Delete history or a track (map: hub only) | | | ✓ |
/// | Read results from a folder of one's choice | | | ✓ |
///
/// On a driver's bridge there are no guests, and its token is an engineer's:
/// what it could do before roles, it still can.
fn required_role(cmd: &ClientCommand, hub: bool) -> Role {
    match cmd {
        ClientCommand::PostRaceInit { results_path } => {
            if results_path.as_deref().map(|p| !p.trim().is_empty()).unwrap_or(false) {
                Role::Admin
            } else {
                Role::Guest
            }
        }
        // Deleting a session destroys data on this PC.
        ClientCommand::HistoryDelete { .. } => Role::Admin,
        ClientCommand::HubAddSource { .. } | ClientCommand::HubRemoveSource { .. } => Role::Admin,
        cmd if is_team_command(cmd) => Role::Admin,
        // The AI key and its spending belong to the hub's PC.
        ClientCommand::AiConfig { .. } if hub => Role::Admin,
        ClientCommand::TrackOutlineDelete { .. } if hub => Role::Admin,
        ClientCommand::StrategyConfig { .. }
        | ClientCommand::AiConfig { .. }
        | ClientCommand::AiAsk { .. }
        | ClientCommand::HubFollow { .. }
        | ClientCommand::FollowCar { .. }
        | ClientCommand::TrackOutlineSave { .. }
        | ClientCommand::TrackOutlineDelete { .. } => Role::Engineer,
        _ => Role::Guest,
    }
}

/// Commands worth a line in the log, with who sent them.
fn changes_the_race(cmd: &ClientCommand) -> bool {
    matches!(
        cmd,
        ClientCommand::StrategyConfig { .. }
            | ClientCommand::AiConfig { .. }
            | ClientCommand::AiAsk { .. }
            | ClientCommand::HubFollow { .. }
    )
}

/// The `command` field of a command's JSON, for the log.
fn command_name(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("command").and_then(|c| c.as_str()).map(str::to_string))
        .unwrap_or_else(|| "command".into())
}

/// What a client of a team receives of `msg`: a guest's team list holds only
/// the teams shared with them.
fn visible_to(caller: &Caller, msg: Arc<ServerMessage>) -> Arc<ServerMessage> {
    if caller.teams.is_none() {
        return msg;
    }
    match &*msg {
        ServerMessage::Teams { teams, current } => Arc::new(ServerMessage::Teams {
            teams: teams.iter().filter(|t| caller.sees_team(&t.id)).cloned().collect(),
            current: current.clone(),
        }),
        _ => msg,
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

const HUB_ONLY: &str = "Only on the hub's PC (http://localhost there).";
const RACING_PC_ONLY: &str = "This action is only available on the racing PC itself.";
const GUEST: &str = "You can watch this team, not change it: ask the engineer.";

/// The error a refused command gets back, in the family the sender listens to.
fn refusal(cmd: &ClientCommand, needed: Role, hub: bool) -> ServerMessage {
    let message = match (needed, hub) {
        (Role::Admin, true) => HUB_ONLY,
        (Role::Admin, false) => RACING_PC_ONLY,
        _ => GUEST,
    }
    .to_string();
    match cmd {
        ClientCommand::HistoryList
        | ClientCommand::HistorySession { .. }
        | ClientCommand::HistoryCarLaps { .. }
        | ClientCommand::HistoryStrategy { .. }
        | ClientCommand::HistoryDelete { .. } => ServerMessage::HistoryError { message },
        ClientCommand::HubAddSource { .. } | ClientCommand::HubRemoveSource { .. } | ClientCommand::HubFollow { .. } => {
            ServerMessage::HubError { message }
        }
        cmd if is_team_command(cmd) => ServerMessage::HubError { message },
        ClientCommand::PostRaceInit { .. } => ServerMessage::PostRaceError { message },
        _ => ServerMessage::Refused { message },
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

    use crate::protocol::messages::TeamInfo;

    fn need(cmd: ClientCommand) -> (Role, Role) {
        (required_role(&cmd, false), required_role(&cmd, true))
    }

    #[test]
    fn installs_and_custom_paths_are_for_the_pc_itself() {
        let cmd = ClientCommand::PostRaceInit { results_path: Some("C:\\Users".into()) };
        assert_eq!(need(cmd.clone()), (Role::Admin, Role::Admin));
        assert!(matches!(refusal(&cmd, Role::Admin, false), ServerMessage::PostRaceError { .. }));
        assert_eq!(need(ClientCommand::HistoryDelete { session_id: 1 }), (Role::Admin, Role::Admin));
    }

    #[test]
    fn a_hubs_drivers_and_teams_are_managed_on_its_pc() {
        for cmd in [
            ClientCommand::HubAddSource { link: "x".into(), label: None },
            ClientCommand::HubRemoveSource { id: "x".into() },
            ClientCommand::HubMoveSource { source_id: "x".into(), team_id: "y".into() },
            ClientCommand::TeamCreate { name: "Clienti".into(), color: None },
            ClientCommand::TeamDelete { id: "x".into() },
        ] {
            assert_eq!(required_role(&cmd, true), Role::Admin, "{cmd:?}");
            assert!(matches!(refusal(&cmd, Role::Admin, true), ServerMessage::HubError { .. }));
        }
        assert_eq!(team_from_query("format=json&team=clienti&token=x").as_deref(), Some("clienti"));
        assert_eq!(team_from_query("token=x&team="), None);
    }

    #[test]
    fn engineers_run_the_race_and_guests_only_watch() {
        // Pinning a driver and asking the AI: the engineers' call, from anywhere.
        for cmd in [ClientCommand::HubFollow { id: None }, ClientCommand::AiAsk { question: None }] {
            assert_eq!(need(cmd.clone()), (Role::Engineer, Role::Engineer), "{cmd:?}");
        }
        let guest = refusal(&ClientCommand::AiAsk { question: None }, Role::Engineer, true);
        assert!(matches!(guest, ServerMessage::Refused { message } if message == GUEST));
        assert!(matches!(refusal(&ClientCommand::HubFollow { id: None }, Role::Engineer, true), ServerMessage::HubError { .. }));
    }

    #[test]
    fn ai_settings_and_maps_belong_to_the_hubs_pc_but_not_to_a_drivers() {
        let ai = ClientCommand::AiConfig { config: Default::default() };
        let map = ClientCommand::TrackOutlineDelete { track: "Spa".into() };
        // A driver's bridge: unchanged, its token may still do this.
        assert_eq!(required_role(&ai, false), Role::Engineer);
        assert_eq!(required_role(&map, false), Role::Engineer);
        assert_eq!(required_role(&ai, true), Role::Admin);
        assert_eq!(required_role(&map, true), Role::Admin);
    }

    #[test]
    fn reading_data_stays_open_to_everyone() {
        for cmd in [
            ClientCommand::PostRaceInit { results_path: None },
            ClientCommand::PostRaceInit { results_path: Some("  ".into()) },
            ClientCommand::PostRaceSessionDetail { session_id: 1 },
            ClientCommand::FuelCalcInit,
            ClientCommand::HistoryList,
            ClientCommand::HistorySession { session_id: 1 },
            ClientCommand::TrackOutlineGet { track: "Spa".into() },
        ] {
            assert_eq!(need(cmd.clone()), (Role::Guest, Role::Guest), "{cmd:?}");
        }
    }

    #[test]
    fn a_guest_sees_only_their_teams_in_the_list() {
        let team = |id: &str| TeamInfo { id: id.into(), name: id.into(), ..Default::default() };
        let msg = Arc::new(ServerMessage::Teams { teams: vec![team("ufficiale"), team("clienti")], current: "clienti".into() });
        let guest = Caller { role: Role::Guest, name: "G".into(), user_id: Some("g".into()), teams: Some(vec!["clienti".into()]) };
        match &*visible_to(&guest, msg.clone()) {
            ServerMessage::Teams { teams, current } => {
                assert_eq!(teams.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["clienti"]);
                assert_eq!(current, "clienti");
            }
            other => panic!("{other:?}"),
        }
        let engineer = Caller { role: Role::Engineer, name: "E".into(), user_id: None, teams: None };
        assert!(Arc::ptr_eq(&visible_to(&engineer, msg.clone()), &msg));
    }
}
