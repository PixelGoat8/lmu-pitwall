//! One team's live stream, the engines that work on it, and where its
//! dashboards' commands go.
//!
//! A driver's bridge has one team (its own car, in the main data folder). A
//! hub has one per team, each in its own folder: the hub's coordinator writes
//! the followed driver's messages into the team's stream, and the team's
//! strategy, race log, history and AI engines read it and add to it. A
//! dashboard watches one team at a time (`?team=` on the WebSocket).

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;

use crate::hub::HubCommand;
use crate::protocol::messages::ServerMessage;

/// Messages queued per slow client before the oldest are dropped.
pub const BROADCAST_CAPACITY: usize = 128;

/// What a client of a team needs: the stream, the latest state to start
/// from, and where its commands go.
pub struct TeamLink {
    pub id: String,
    /// Where the team keeps its settings and history.
    pub dir: PathBuf,
    pub tx: broadcast::Sender<Arc<ServerMessage>>,
    /// Latest AllDriversUpdate and ConnectionStatus, sent to a client on
    /// connect so it does not wait for the next one.
    pub all_drivers_rx: watch::Receiver<Option<ServerMessage>>,
    pub connection_status_rx: watch::Receiver<Option<ServerMessage>>,
    /// The team's hub coordinator; `None` on a driver's bridge.
    pub hub_tx: Option<mpsc::UnboundedSender<HubCommand>>,
    /// New strategy settings, with who sent them (for the change log).
    pub strategy_tx: mpsc::UnboundedSender<(crate::strategy::StrategyConfig, String)>,
    pub ai_tx: mpsc::UnboundedSender<crate::ai::AiCommand>,
    /// Lines for the team's change log.
    pub notes_tx: crate::changelog::Notes,
}

/// What the team's live source (the game reader, a replay, or the hub's
/// coordinator) writes to.
pub struct TeamFeed {
    pub tx: broadcast::Sender<Arc<ServerMessage>>,
    pub all_drivers_tx: watch::Sender<Option<ServerMessage>>,
    pub connection_status_tx: watch::Sender<Option<ServerMessage>>,
}

/// "No game": the state a team starts in, so a dashboard that connects before
/// anything happens is told so instead of having to infer it from silence.
pub fn no_game() -> ServerMessage {
    ServerMessage::ConnectionStatus {
        game_connected: false,
        plugin_version: String::new(),
        telemetry_valid: true,
        telemetry_warning: None,
        session_active: false,
    }
}

/// A team's channels, with its engines started on `dir`. The engines stop
/// when the returned tasks are aborted.
pub fn start(
    id: &str,
    dir: PathBuf,
    hub_tx: Option<mpsc::UnboundedSender<HubCommand>>,
) -> (Arc<TeamLink>, TeamFeed, Vec<JoinHandle<()>>) {
    let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
    let (all_drivers_tx, all_drivers_rx) = watch::channel(None);
    let (connection_status_tx, connection_status_rx) = watch::channel(Some(no_game()));
    let (strategy_tx, strategy_rx) = mpsc::unbounded_channel();
    let (ai_tx, ai_rx) = mpsc::unbounded_channel();
    let (notes_tx, notes_rx) = mpsc::unbounded_channel();

    let tasks = vec![
        // The plan, from this team's stream, back into it.
        tokio::spawn(crate::strategy::run(tx.subscribe(), tx.clone(), strategy_rx, notes_tx.clone(), dir.clone())),
        // Race events and lap history.
        tokio::spawn(crate::race::run(tx.subscribe(), tx.clone())),
        // Session history in the team's database.
        tokio::spawn(crate::history::run(tx.subscribe(), dir.clone())),
        // AI comments (off unless enabled and ANTHROPIC_API_KEY is set).
        tokio::spawn(crate::ai::run(tx.subscribe(), tx.clone(), ai_rx, notes_tx.clone(), dir.clone())),
        // Who changed what, for the dashboards and the history.
        tokio::spawn(crate::changelog::run(tx.subscribe(), tx.clone(), notes_rx)),
    ];

    let link = Arc::new(TeamLink {
        id: id.to_string(),
        dir,
        tx: tx.clone(),
        all_drivers_rx,
        connection_status_rx,
        hub_tx,
        strategy_tx,
        ai_tx,
        notes_tx,
    });
    (link, TeamFeed { tx, all_drivers_tx, connection_status_tx }, tasks)
}
