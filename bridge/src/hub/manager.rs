//! The teams of a hub, running.
//!
//! Each team is started with its own stream, engines and coordinator (see
//! `crate::team` and `super::run`). This keeper starts and stops them as teams
//! are created and removed, moves drivers' bridges between teams, and sends
//! every dashboard the team list each second, for the team switcher and the
//! sister-car marks.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use super::teams::{TeamConfig, TeamsStore};
use super::{HubCommand, HubTeam, TeamSummary};
use crate::protocol::messages::{ClientCommand, ServerMessage, TeamInfo};
use crate::team::TeamLink;
use crate::websocket::server::WebSocketServer;

struct Running {
    link: Arc<TeamLink>,
    tasks: Vec<JoinHandle<()>>,
    summary_rx: watch::Receiver<TeamSummary>,
}

impl Running {
    fn hub(&self, cmd: HubCommand) {
        if let Some(tx) = &self.link.hub_tx {
            let _ = tx.send(cmd);
        }
    }
}

fn start_team(cfg: &TeamConfig, store: &Arc<TeamsStore>, ws: &WebSocketServer) -> Running {
    let (hub_tx, hub_rx) = mpsc::unbounded_channel();
    let (link, feed, mut tasks) = crate::team::start(&cfg.id, store.team_dir(&cfg.id), Some(hub_tx));
    let (summary_tx, summary_rx) = watch::channel(TeamSummary::default());
    let team = HubTeam {
        id: cfg.id.clone(),
        sources: cfg.sources.clone(),
        follow: cfg.follow.clone(),
        store: store.clone(),
        feed,
        summary_tx,
    };
    tasks.push(tokio::spawn(super::run(team, hub_rx)));
    ws.add_team(link.clone());
    Running { link, tasks, summary_rx }
}

/// The list every dashboard gets, in the teams' order.
fn team_list(store: &TeamsStore, running: &HashMap<String, Running>) -> Vec<TeamInfo> {
    store
        .snapshot()
        .teams
        .into_iter()
        .map(|t| {
            let s = running.get(&t.id).map(|r| r.summary_rx.borrow().clone()).unwrap_or_default();
            TeamInfo {
                id: t.id,
                name: t.name,
                color: t.color,
                sources: t.sources.len() as u32,
                active: s.active,
                driving: s.driving,
                car_name: s.car_name,
                driver: s.driver,
            }
        })
        .collect()
}

/// A hub's teams, started.
pub struct Teams(HashMap<String, Running>);

/// Start every team and register it with the server, in the teams' order (the
/// first is the one a dashboard gets when it asks for none).
pub fn start_teams(store: &Arc<TeamsStore>, ws: &WebSocketServer) -> Teams {
    let mut running = HashMap::new();
    for cfg in store.snapshot().teams {
        info!("Hub: starting team \"{}\" ({} bridge(s))", cfg.name, cfg.sources.len());
        running.insert(cfg.id.clone(), start_team(&cfg, store, ws));
    }
    Teams(running)
}

/// Run a hub's teams until the process exits. `busy_tx` is set while any
/// team's bridges have a live session (the idle shutdown waits for that).
pub async fn run(
    ws: Arc<WebSocketServer>,
    store: Arc<TeamsStore>,
    teams: Teams,
    mut requests: mpsc::UnboundedReceiver<(String, ClientCommand)>,
    busy_tx: watch::Sender<bool>,
) {
    let mut running = teams.0;

    let send_list = |running: &HashMap<String, Running>| {
        let teams = team_list(&store, running);
        for r in running.values() {
            let _ = r.link.tx.send(Arc::new(ServerMessage::Teams { teams: teams.clone(), current: r.link.id.clone() }));
        }
    };

    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            Some((from, cmd)) = requests.recv() => {
                if let Err(e) = handle(&cmd, &store, &ws, &mut running) {
                    warn!("Hub: {}", e);
                    if let Some(r) = running.get(&from) {
                        let _ = r.link.tx.send(Arc::new(ServerMessage::HubError { message: format!("{e}.") }));
                    }
                }
                send_list(&running);
            }
            _ = tick.tick() => {
                send_list(&running);
                let busy = running.values().any(|r| r.summary_rx.borrow().busy);
                busy_tx.send_if_modified(|b| {
                    let changed = *b != busy;
                    *b = busy;
                    changed
                });
            }
        }
    }
}

fn handle(
    cmd: &ClientCommand,
    store: &Arc<TeamsStore>,
    ws: &WebSocketServer,
    running: &mut HashMap<String, Running>,
) -> anyhow::Result<()> {
    match cmd {
        ClientCommand::TeamCreate { name, color } => {
            let cfg = store.create(name, color.as_deref())?;
            info!("Hub: team \"{}\" created", cfg.name);
            running.insert(cfg.id.clone(), start_team(&cfg, store, ws));
        }
        ClientCommand::TeamUpdate { id, name, color } => {
            store.update(id, name.as_deref(), color.as_deref())?;
        }
        ClientCommand::TeamDelete { id } => {
            store.delete(id)?;
            ws.remove_team(id);
            if let Some(r) = running.remove(id) {
                for t in r.tasks {
                    t.abort();
                }
            }
            info!("Hub: team {} removed", id);
        }
        ClientCommand::HubMoveSource { source_id, team_id } => {
            let (from, cfg) = store.find_source(source_id).ok_or_else(|| anyhow::anyhow!("that driver is not in any team"))?;
            if &from == team_id {
                return Ok(());
            }
            let to = running.get(team_id).ok_or_else(|| anyhow::anyhow!("no such team"))?;
            to.hub(HubCommand::AddConfigured(cfg.clone()));
            if let Some(r) = running.get(&from) {
                r.hub(HubCommand::RemoveSource { id: source_id.clone() });
            }
            info!("Hub: {} moved from team {} to team {}", cfg.label, from, team_id);
        }
        _ => {}
    }
    Ok(())
}
