//! The change log of a team: who changed what in the race, and when.
//!
//! "21:14 Marco: VE target 3.2 %/lap". Every change an engineer makes that
//! steers the race (strategy settings and targets, pinning a driver's PC, the
//! AI's settings, a question to the AI) becomes one line, with the name the
//! hub knows the sender by (see `crate::access`). The lines go to the team's
//! dashboards and into the session history.
//!
//! The engines that hold the settings write the lines, because only they know
//! what the setting was before: the strategy engine compares its old and new
//! settings with [`describe_strategy`], the AI engine with [`describe_ai`].
//! This task only stamps them with the time, the session clock and the lap,
//! keeps the latest, and broadcasts them.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

use crate::ai::AiConfig;
use crate::hub::logic::num;
use crate::protocol::messages::ServerMessage;
use crate::recording::{field, message_type, to_raw};
use crate::strategy::StrategyConfig;

/// Lines kept for the dashboards (the history keeps them all).
const KEEP: usize = 200;
/// How often the whole list goes out, so a dashboard that has just connected
/// has it within this long.
const RESEND: Duration = Duration::from_secs(3);

/// One change, as an engine reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub who: String,
    pub text: String,
}

impl Note {
    pub fn new(who: &str, text: impl Into<String>) -> Self {
        Note { who: who.to_string(), text: text.into() }
    }
}

pub type Notes = mpsc::UnboundedSender<Note>;

/// One line of the log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeEntry {
    /// Increasing within a run of the bridge.
    pub seq: u64,
    /// Wall clock, Unix seconds.
    pub at: i64,
    /// Session clock and our completed laps at the time, when in a session.
    pub session_time: Option<f64>,
    pub lap: Option<i64>,
    pub who: String,
    pub text: String,
}

// ---------------------------------------------------------------------------
// What changed, in words
// ---------------------------------------------------------------------------

fn trim(x: f64, decimals: usize) -> String {
    let s = format!("{x:.decimals$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// A target as the engineer set it: per lap or "box at the end of lap N".
fn target(per_lap: Option<f64>, box_lap: Option<i64>, percent: bool) -> Option<String> {
    if let Some(lap) = box_lap {
        return Some(format!("box at the end of lap {lap}"));
    }
    per_lap.map(|v| if percent { format!("{} %/lap", trim(v * 100.0, 2)) } else { format!("{} L/lap", trim(v, 2)) })
}

/// The changes between two strategy settings, one line each; empty when
/// nothing a person would notice changed.
pub fn describe_strategy(old: &StrategyConfig, new: &StrategyConfig) -> Vec<String> {
    let mut out = Vec::new();
    for (name, before, after) in [
        ("VE target", target(old.ve_target_per_lap, old.ve_target_box_lap, true), target(new.ve_target_per_lap, new.ve_target_box_lap, true)),
        ("Fuel target", target(old.fuel_target_per_lap, old.fuel_target_box_lap, false), target(new.fuel_target_per_lap, new.fuel_target_box_lap, false)),
    ] {
        if before != after {
            out.push(match after {
                Some(t) => format!("{name} {t}"),
                None => format!("{name} cleared"),
            });
        }
    }
    if trim(old.reserve_laps, 2) != trim(new.reserve_laps, 2) {
        let r = trim(new.reserve_laps, 2);
        out.push(format!("reserve {r} {}", if r == "1" { "lap" } else { "laps" }));
    }
    let stint = |m: Option<f64>| m.map(|m| trim(m, 1));
    if stint(old.max_stint_minutes) != stint(new.max_stint_minutes) {
        out.push(match stint(new.max_stint_minutes) {
            Some(m) => format!("max stint {m} min"),
            None => "no stint limit".into(),
        });
    }
    if trim(old.tyre_wear_limit * 100.0, 0) != trim(new.tyre_wear_limit * 100.0, 0) {
        out.push(format!("tyre limit {} % tread", trim(new.tyre_wear_limit * 100.0, 0)));
    }
    if old.change_tyres != new.change_tyres {
        out.push(if new.change_tyres { "new tyres at the stop".into() } else { "no tyre change at the stop".into() });
    }
    let loss = |l: Option<f64>| l.map(|l| trim(l, 1));
    if loss(old.pit_lane_loss_s) != loss(new.pit_lane_loss_s) {
        out.push(match loss(new.pit_lane_loss_s) {
            Some(l) => format!("pit lane loss {l} s"),
            None => "pit lane loss: measured".into(),
        });
    }
    out
}

/// The changes between two AI settings worth a line.
pub fn describe_ai(old: &AiConfig, new: &AiConfig) -> Vec<String> {
    let mut out = Vec::new();
    if old.enabled != new.enabled {
        out.push(if new.enabled { "AI on".into() } else { "AI off".into() });
    }
    if trim(old.daily_budget_usd, 2) != trim(new.daily_budget_usd, 2) {
        out.push(format!("AI budget ${} a day", trim(new.daily_budget_usd, 2)));
    }
    if old.model != new.model {
        out.push(format!("AI model {}", new.model));
    }
    if old.every_laps != new.every_laps {
        out.push(match new.every_laps {
            0 => "AI comments only on events and on request".into(),
            n => format!("AI comment every {n} laps"),
        });
    }
    // The rest (effort, limits, language…) in one line rather than none.
    let mut rest_old = old.clone();
    rest_old.enabled = new.enabled;
    rest_old.daily_budget_usd = new.daily_budget_usd;
    rest_old.model = new.model.clone();
    rest_old.every_laps = new.every_laps;
    if serde_json::to_value(&rest_old).ok() != serde_json::to_value(new).ok() {
        out.push("AI settings changed".into());
    }
    out
}

// ---------------------------------------------------------------------------
// The log
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Log {
    next_seq: u64,
    entries: Vec<ChangeEntry>,
    session_time: Option<f64>,
    lap: Option<i64>,
}

impl Log {
    /// Follow the session clock and our laps from the team's stream.
    pub fn on_message(&mut self, msg: &ServerMessage) {
        match msg {
            ServerMessage::StrategyUpdate(s) => self.lap = Some(s.laps_completed),
            _ => {
                let Some(v) = to_raw(msg) else { return };
                match message_type(&v) {
                    Some("ScoringUpdate") => self.session_time = field(&v, "session_time").and_then(num),
                    Some("ConnectionStatus") => {
                        if field(&v, "session_active").and_then(|x| x.as_bool()) == Some(false) {
                            self.session_time = None;
                            self.lap = None;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn add(&mut self, note: Note, at: i64) -> ChangeEntry {
        self.next_seq += 1;
        let entry = ChangeEntry { seq: self.next_seq, at, session_time: self.session_time, lap: self.lap, who: note.who, text: note.text };
        self.entries.push(entry.clone());
        if self.entries.len() > KEEP {
            self.entries.remove(0);
        }
        entry
    }

    pub fn entries(&self) -> &[ChangeEntry] {
        &self.entries
    }
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Keep a team's change log until the team stops.
pub async fn run(mut rx: broadcast::Receiver<Arc<ServerMessage>>, tx: broadcast::Sender<Arc<ServerMessage>>, mut notes: mpsc::UnboundedReceiver<Note>) {
    let mut log = Log::default();
    let mut resend = tokio::time::interval(RESEND);
    // Empty too: a dashboard can tell "no changes yet" from "no log".
    let send_all = |log: &Log| {
        let _ = tx.send(Arc::new(ServerMessage::ChangeLog { entries: log.entries().to_vec() }));
    };
    loop {
        tokio::select! {
            r = rx.recv() => match r {
                Ok(msg) => log.on_message(&msg),
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            },
            Some(note) = notes.recv() => {
                tracing::info!("Change: {}: {}", note.who, note.text);
                let entry = log.add(note, now_unix());
                // One message per line, for the history to store, then the list.
                let _ = tx.send(Arc::new(ServerMessage::ChangeLogEntry { entry }));
                send_all(&log);
            }
            _ = resend.tick() => send_all(&log),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategy_changes_read_like_an_engineer_would_say_them() {
        let old = StrategyConfig::default();
        let mut new = old.clone();
        assert!(describe_strategy(&old, &new).is_empty(), "nothing changed, nothing said");

        new.ve_target_per_lap = Some(0.032);
        assert_eq!(describe_strategy(&old, &new), vec!["VE target 3.2 %/lap"]);

        let mut lap = new.clone();
        lap.ve_target_box_lap = Some(170);
        assert_eq!(describe_strategy(&new, &lap), vec!["VE target box at the end of lap 170"]);
        let mut cleared = lap.clone();
        cleared.ve_target_per_lap = None;
        cleared.ve_target_box_lap = None;
        assert_eq!(describe_strategy(&lap, &cleared), vec!["VE target cleared"]);

        let mut all = old.clone();
        all.fuel_target_per_lap = Some(2.85);
        all.reserve_laps = 1.0;
        all.max_stint_minutes = Some(45.0);
        all.tyre_wear_limit = 0.25;
        all.change_tyres = !old.change_tyres;
        all.pit_lane_loss_s = Some(24.0);
        assert_eq!(
            describe_strategy(&old, &all),
            vec![
                "Fuel target 2.85 L/lap",
                "reserve 1 lap",
                "max stint 45 min",
                "tyre limit 25 % tread",
                if all.change_tyres { "new tyres at the stop" } else { "no tyre change at the stop" },
                "pit lane loss 24 s",
            ]
        );
        assert_eq!(describe_strategy(&all, &old)[2..4], ["no stint limit".to_string(), "tyre limit 30 % tread".to_string()]);
        assert_eq!(describe_strategy(&all, &old).last().unwrap(), "pit lane loss: measured");
    }

    #[test]
    fn ai_changes_name_what_matters_and_sum_up_the_rest() {
        let old = AiConfig::default();
        let mut new = old.clone();
        assert!(describe_ai(&old, &new).is_empty());
        new.enabled = !old.enabled;
        new.daily_budget_usd = 5.0;
        assert_eq!(describe_ai(&old, &new), vec![if new.enabled { "AI on" } else { "AI off" }, "AI budget $5 a day"]);
        let mut lang = old.clone();
        lang.language = "Klingon".into();
        assert_eq!(describe_ai(&old, &lang), vec!["AI settings changed"]);
    }

    #[test]
    fn a_line_carries_the_session_clock_and_lap_of_its_time() {
        let mut log = Log::default();
        let first = log.add(Note::new("hub PC", "AI on"), 100);
        assert_eq!((first.seq, first.session_time, first.lap), (1, None, None));

        let scoring = ServerMessage::Raw(rmpv::ext::to_value(serde_json::json!({"type": "ScoringUpdate", "session_time": 1234.5})).unwrap());
        log.on_message(&scoring);
        let e = log.add(Note::new("Marco", "VE target 3.2 %/lap"), 200);
        assert_eq!((e.seq, e.session_time, e.who.as_str()), (2, Some(1234.5), "Marco"));

        let menus = ServerMessage::Raw(rmpv::ext::to_value(serde_json::json!({"type": "ConnectionStatus", "session_active": false})).unwrap());
        log.on_message(&menus);
        assert_eq!(log.add(Note::new("x", "y"), 300).session_time, None);

        for i in 0..KEEP + 10 {
            log.add(Note::new("x", format!("{i}")), 400);
        }
        assert_eq!(log.entries().len(), KEEP);
        assert_eq!(log.entries().last().unwrap().text, format!("{}", KEEP + 9));
    }
}
