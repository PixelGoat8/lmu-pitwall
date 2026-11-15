//! Race events and lap history.
//!
//! Like the strategy engine, this reads the broadcast stream (the bridge's
//! own, a replay's, or on a hub the relayed driver's), so it runs unchanged
//! everywhere and its log survives driver swaps on a hub. It keeps:
//!
//! * a log of what happened in the session: full course yellow, pit lane
//!   open/closed, race start and end, pit stops of the player's car and its
//!   class, penalties, driver changes, class position changes, impacts, blue
//!   flags, best laps;
//! * the recent lap times of every car, for comparing rivals lap by lap.
//!
//! Both go out as one `RaceTiming` message when something changes (at most
//! once a second) and every five seconds.
//!
//! The rules are deliberately literal: an event is a change seen in the
//! game's own fields, worded plainly. Nothing is inferred beyond that, so the
//! AI layer can repeat these without adding anything.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmpv::Value;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::warn;

use crate::hub::logic::num;
use crate::protocol::messages::ServerMessage;
use crate::recording::{field, message_type};

/// Events kept; the oldest are dropped.
const MAX_EVENTS: usize = 300;
/// Laps kept per car.
const MAX_LAPS_PER_CAR: usize = 20;
const MIN_SEND_INTERVAL: Duration = Duration::from_secs(1);
const REFRESH: Duration = Duration::from_secs(5);
/// Position changes this close together are merged into one event.
const POSITION_MERGE_S: f64 = 8.0;
/// `mLastImpactMagnitude` at or above this is logged. Ordinary driving over
/// kerbs stayed below it in the one session measured (max 2650 at Monza with
/// no contact reported); this is a first guess, to be tuned on real contacts.
const IMPACT_LOG_THRESHOLD: f64 = 3000.0;
/// Game phase: full course yellow.
const PHASE_FCY: i64 = 6;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaceEvent {
    /// Session time, seconds.
    pub t: f64,
    /// Laps completed by the player's car when it happened.
    pub lap: i64,
    /// Machine-readable kind: "fcy", "fcy_end", "pits", "start", "finish",
    /// "pit_in", "pit_out", "penalty", "driver", "position", "impact",
    /// "blue_flag", "best_lap", "class_best".
    pub kind: String,
    /// "info", "warn" or "alert".
    pub severity: String,
    /// The car concerned, if any.
    pub car_id: Option<i64>,
    /// True when the car is the player's own.
    pub own: bool,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LapTime {
    pub lap: i64,
    pub time: f64,
    /// The lap went through the pit lane.
    pub pit: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CarLaps {
    pub id: i64,
    pub laps: Vec<LapTime>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RaceTiming {
    pub events: Vec<RaceEvent>,
    pub laps: Vec<CarLaps>,
}

#[derive(Debug, Clone, Default)]
struct Car {
    driver: String,
    class: String,
    position: i64,
    total_laps: i64,
    best_lap: f64,
    in_pits: bool,
    penalties: i64,
    pit_entry_t: Option<f64>,
    lap_pit: bool,
}

#[derive(Default)]
pub struct RaceLog {
    events: Vec<RaceEvent>,
    laps: HashMap<i64, Vec<LapTime>>,
    cars: HashMap<i64, Car>,
    player_id: Option<i64>,
    class_pos: Option<i64>,
    class_best: Option<f64>,
    session_type: String,
    session_time: f64,
    game_phase: Option<i64>,
    yellow_state: Option<i64>,
    last_impact_et: Option<f64>,
    blue: bool,
    dirty: bool,
}

fn n(v: &Value, key: &str) -> Option<f64> {
    field(v, key).and_then(num)
}
fn s(v: &Value, key: &str) -> String {
    field(v, key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn fmt_lap(t: f64) -> String {
    let m = (t / 60.0).floor();
    format!("{}:{:06.3}", m as i64, t - m * 60.0)
}

impl RaceLog {
    pub fn new() -> Self {
        Self::default()
    }

    fn reset(&mut self) {
        *self = RaceLog { dirty: true, ..Default::default() };
    }

    fn own_laps(&self) -> i64 {
        self.player_id.and_then(|id| self.cars.get(&id)).map(|c| c.total_laps).unwrap_or(0)
    }

    fn push(&mut self, kind: &str, severity: &str, car_id: Option<i64>, text: String) {
        let own = car_id.is_some() && car_id == self.player_id;
        self.events.push(RaceEvent {
            t: self.session_time,
            lap: self.own_laps(),
            kind: kind.into(),
            severity: severity.into(),
            car_id,
            own,
            text,
        });
        if self.events.len() > MAX_EVENTS {
            self.events.remove(0);
        }
        self.dirty = true;
    }

    pub fn timing(&self) -> RaceTiming {
        let mut laps: Vec<CarLaps> = self.laps.iter().map(|(id, l)| CarLaps { id: *id, laps: l.clone() }).collect();
        laps.sort_by_key(|c| c.id);
        RaceTiming { events: self.events.clone(), laps }
    }

    /// Whether anything changed since the last call.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub fn has_session(&self) -> bool {
        !self.cars.is_empty()
    }

    pub fn on_message(&mut self, msg: &Value) {
        match message_type(msg) {
            Some("ScoringUpdate") => self.on_scoring(msg),
            Some("VehicleStatusUpdate") => self.on_status(msg),
            Some("ConnectionStatus") => {
                let active = field(msg, "session_active").and_then(|v| v.as_bool()).unwrap_or(true);
                if !active && self.has_session() {
                    self.reset();
                }
            }
            _ => {}
        }
    }

    fn on_status(&mut self, msg: &Value) {
        if !self.has_session() {
            return;
        }
        let pid = self.player_id;

        let phase = n(msg, "game_phase").map(|v| v as i64);
        if let Some(p) = phase {
            if let Some(prev) = self.game_phase {
                if p != prev {
                    match p {
                        PHASE_FCY => self.push("fcy", "alert", None, "Full course yellow".into()),
                        5 if prev == PHASE_FCY => self.push("fcy_end", "warn", None, "Green: full course yellow over".into()),
                        5 if prev < 5 && self.session_type.to_lowercase().contains("race") => {
                            self.push("start", "info", None, "Green flag: race started".into())
                        }
                        8 => self.push("finish", "info", None, "Session over (chequered flag)".into()),
                        _ => {}
                    }
                }
            }
            self.game_phase = Some(p);
        }

        // rF2 yellow flag state: 2 pits closed, 3 pit lead lap, 4 pits open,
        // 5 last lap, 6 resume, 7 race halt.
        if let Some(y) = n(msg, "yellow_flag_state").map(|v| v as i64) {
            if self.yellow_state.is_some() && self.yellow_state != Some(y) {
                let text = match y {
                    2 => Some(("warn", "Pit lane closed")),
                    3 => Some(("info", "Pit lane open to lead-lap cars")),
                    4 => Some(("info", "Pit lane open")),
                    5 => Some(("warn", "Last lap under caution")),
                    6 => Some(("warn", "Restart: green flag next")),
                    7 => Some(("alert", "Race halted")),
                    _ => None,
                };
                if let Some((sev, t)) = text {
                    self.push("pits", sev, None, t.into());
                }
            }
            self.yellow_state = Some(y);
        }

        if let (Some(et), Some(mag)) = (n(msg, "last_impact_et"), n(msg, "last_impact_magnitude")) {
            if let Some(prev) = self.last_impact_et {
                if et > prev + 0.01 && mag >= IMPACT_LOG_THRESHOLD {
                    self.push("impact", "warn", pid, format!("Impact on our car (magnitude {:.0})", mag));
                }
            }
            self.last_impact_et = Some(et);
        }

        let blue = n(msg, "player_flag").map(|v| v as i64) == Some(6);
        if blue && !self.blue {
            self.push("blue_flag", "info", pid, "Blue flag shown to our driver".into());
        }
        self.blue = blue;
    }

    fn on_scoring(&mut self, msg: &Value) {
        let t = n(msg, "session_time").unwrap_or(0.0);
        let stype = s(msg, "session_type");
        if self.has_session() && (stype != self.session_type || t < self.session_time - 5.0) {
            self.reset();
        }
        self.session_type = stype;
        self.session_time = t;
        let pid = field(msg, "player_vehicle_id").and_then(|v| v.as_i64()).filter(|id| *id >= 0);
        self.player_id = pid;
        let Some(rows) = field(msg, "vehicles").and_then(|v| v.as_array()) else { return };

        let first_sight = self.cars.is_empty();
        let player_class = pid
            .and_then(|id| rows.iter().find(|r| field(r, "id").and_then(|x| x.as_i64()) == Some(id)))
            .map(|r| s(r, "vehicle_class"));

        for r in rows {
            let Some(id) = field(r, "id").and_then(|x| x.as_i64()) else { continue };
            let cur = Car {
                driver: s(r, "driver_name"),
                class: s(r, "vehicle_class"),
                position: n(r, "position").unwrap_or(0.0) as i64,
                total_laps: n(r, "total_laps").unwrap_or(0.0) as i64,
                best_lap: n(r, "best_lap_time").unwrap_or(-1.0),
                in_pits: field(r, "in_pits").and_then(|x| x.as_bool()).unwrap_or(false),
                penalties: n(r, "num_penalties").unwrap_or(0.0) as i64,
                pit_entry_t: None,
                lap_pit: false,
            };
            let last_lap = n(r, "last_lap_time").unwrap_or(-1.0);
            let own = Some(id) == pid;
            let rival = player_class.as_deref() == Some(cur.class.as_str()) && !own;
            let Some(prev) = self.cars.get(&id).cloned() else {
                let mut c = cur;
                c.lap_pit = c.in_pits;
                self.cars.insert(id, c);
                continue;
            };
            let mut next = cur.clone();
            next.pit_entry_t = prev.pit_entry_t;
            next.lap_pit = prev.lap_pit || cur.in_pits;
            let who = if own { "Our car".to_string() } else { format!("#{} {}", id, cur.driver) };

            // Laps
            if cur.total_laps > prev.total_laps && last_lap > 0.0 {
                let list = self.laps.entry(id).or_default();
                list.push(LapTime { lap: cur.total_laps, time: last_lap, pit: next.lap_pit });
                if list.len() > MAX_LAPS_PER_CAR {
                    list.remove(0);
                }
                next.lap_pit = cur.in_pits;
                self.dirty = true;
            }

            // Pit stops: our car and our class
            if own || rival {
                if cur.in_pits && !prev.in_pits {
                    next.pit_entry_t = Some(t);
                    self.push("pit_in", "info", Some(id), format!("{} entered the pits", who));
                }
                if !cur.in_pits && prev.in_pits {
                    let dur = prev.pit_entry_t.map(|e| format!(" after {:.1} s in the pit lane", t - e)).unwrap_or_default();
                    next.pit_entry_t = None;
                    self.push("pit_out", "info", Some(id), format!("{} left the pits{}", who, dur));
                }
                if cur.penalties > prev.penalties {
                    let sev = if own { "alert" } else { "info" };
                    self.push("penalty", sev, Some(id), format!("{} received a penalty ({} outstanding)", who, cur.penalties));
                }
            }

            if own {
                if !cur.driver.is_empty() && !prev.driver.is_empty() && cur.driver != prev.driver {
                    self.push("driver", "warn", Some(id), format!("Driver change: {} → {}", prev.driver, cur.driver));
                }
                if cur.best_lap > 0.0 && (prev.best_lap <= 0.0 || cur.best_lap < prev.best_lap - 1e-4) && prev.total_laps > 0 {
                    self.push("best_lap", "info", Some(id), format!("Our personal best: {}", fmt_lap(cur.best_lap)));
                }
            }
            self.cars.insert(id, next);
        }

        // Class best lap (any car of our class, our own included).
        if let Some(cls) = &player_class {
            let best = self
                .cars
                .iter()
                .filter(|(_, c)| &c.class == cls && c.best_lap > 0.0)
                .min_by(|a, b| a.1.best_lap.total_cmp(&b.1.best_lap))
                .map(|(id, c)| (*id, c.best_lap, c.driver.clone()));
            if let Some((id, best, driver)) = best {
                if !first_sight && self.class_best.map(|b| best < b - 1e-4).unwrap_or(false) {
                    let who = if Some(id) == pid { "Our car".to_string() } else { format!("#{} {}", id, driver) };
                    self.push("class_best", "info", Some(id), format!("Fastest lap in class: {} ({})", fmt_lap(best), who));
                }
                self.class_best = Some(best);
            }

            // Class position of our car.
            if let Some(id) = pid {
                let mut same: Vec<(i64, i64)> = self.cars.iter().filter(|(_, c)| &c.class == cls).map(|(k, c)| (c.position, *k)).collect();
                same.sort();
                let pos = same.iter().position(|(_, k)| *k == id).map(|i| i as i64 + 1);
                if let (Some(old), Some(new)) = (self.class_pos, pos) {
                    if old != new && self.session_type.to_lowercase().contains("race") {
                        self.position_event(id, old, new, same.len());
                    }
                }
                if pos.is_some() {
                    self.class_pos = pos;
                }
            }
        }
    }

    /// Log a class position change, merging changes that come in a burst
    /// (a pit stop shuffles several places within seconds).
    fn position_event(&mut self, id: i64, old: i64, new: i64, size: usize) {
        if let Some(last) = self.events.last_mut() {
            if last.kind == "position" && self.session_time - last.t < POSITION_MERGE_S {
                if let Some(from) = last.text.split(' ').nth(1).and_then(|p| p.trim_start_matches('P').parse::<i64>().ok()) {
                    if from == new {
                        self.events.pop();
                        self.dirty = true;
                        return;
                    }
                    last.text = position_text(from, new, size);
                    last.severity = if new < from { "info".into() } else { "warn".into() };
                    last.t = self.session_time;
                    self.dirty = true;
                    return;
                }
            }
        }
        let sev = if new < old { "info" } else { "warn" };
        self.push("position", sev, Some(id), position_text(old, new, size));
    }
}

fn position_text(from: i64, to: i64, size: usize) -> String {
    let verb = if to < from { "gained" } else { "lost" };
    format!("Class P{} → P{} of {} ({} {})", from, to, size, verb, if (to - from).abs() == 1 { "1 place".to_string() } else { format!("{} places", (to - from).abs()) })
}

/// Run on a broadcast stream until it closes; send `RaceTiming` on the same channel.
pub async fn run(mut rx: broadcast::Receiver<Arc<ServerMessage>>, tx: broadcast::Sender<Arc<ServerMessage>>) {
    let mut log = RaceLog::new();
    let mut last_sent = Instant::now() - REFRESH;
    let mut tick = tokio::time::interval(MIN_SEND_INTERVAL);
    loop {
        tokio::select! {
            r = rx.recv() => match r {
                Ok(msg) => {
                    let value = match &*msg {
                        ServerMessage::RaceTiming(_) | ServerMessage::StrategyUpdate(_) | ServerMessage::TelemetryUpdate { .. } => continue,
                        ServerMessage::Raw(v) => {
                            if matches!(message_type(v), Some("TelemetryUpdate") | Some("RaceTiming")) {
                                continue;
                            }
                            v.clone()
                        }
                        other => match rmpv::ext::to_value(other) {
                            Ok(v) => v,
                            Err(_) => continue,
                        },
                    };
                    log.on_message(&value);
                }
                Err(broadcast::error::RecvError::Lagged(k)) => warn!("Race log fell behind by {} messages", k),
                Err(broadcast::error::RecvError::Closed) => return,
            },
            _ = tick.tick() => {
                let changed = log.take_dirty();
                if (changed || last_sent.elapsed() >= REFRESH) && (log.has_session() || changed) {
                    let _ = tx.send(Arc::new(ServerMessage::RaceTiming(Box::new(log.timing()))));
                    last_sent = Instant::now();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(j: serde_json::Value) -> Value {
        rmpv::ext::to_value(j).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn car(id: i64, cls: &str, pos: i64, laps: i64, last: f64, best: f64, pits: bool, pen: i64, driver: &str) -> serde_json::Value {
        serde_json::json!({"id": id, "vehicle_class": cls, "position": pos, "total_laps": laps, "last_lap_time": last,
                           "best_lap_time": best, "in_pits": pits, "num_penalties": pen, "driver_name": driver})
    }

    fn scoring(t: f64, cars: Vec<serde_json::Value>) -> Value {
        v(serde_json::json!({"type": "ScoringUpdate", "session_type": "Race", "session_time": t, "player_vehicle_id": 1, "vehicles": cars}))
    }

    fn kinds(l: &RaceLog) -> Vec<String> {
        l.events.iter().map(|e| e.kind.clone()).collect()
    }

    #[test]
    fn laps_pits_penalties_and_driver_changes() {
        let mut l = RaceLog::new();
        l.on_message(&scoring(0.0, vec![car(1, "GT3", 2, 0, -1.0, -1.0, false, 0, "Alice"), car(2, "GT3", 1, 0, -1.0, -1.0, false, 0, "Rival"), car(3, "Hyper", 3, 0, -1.0, -1.0, false, 0, "Other")]));
        l.on_message(&scoring(100.0, vec![car(1, "GT3", 2, 1, 100.0, 100.0, false, 0, "Alice"), car(2, "GT3", 1, 1, 99.0, 99.0, false, 0, "Rival"), car(3, "Hyper", 3, 1, 90.0, 90.0, true, 0, "Other")]));
        // The Hypercar's pit stop is not our class: not logged. Its lap is kept.
        assert!(kinds(&l).is_empty());
        assert_eq!(l.laps[&3].len(), 1);
        l.on_message(&scoring(110.0, vec![car(1, "GT3", 2, 1, 100.0, 100.0, true, 1, "Alice"), car(2, "GT3", 1, 1, 99.0, 99.0, false, 0, "Rival"), car(3, "Hyper", 3, 1, 90.0, 90.0, false, 0, "Other")]));
        l.on_message(&scoring(150.0, vec![car(1, "GT3", 2, 1, 100.0, 100.0, false, 1, "Bob"), car(2, "GT3", 1, 1, 99.0, 99.0, false, 0, "Rival"), car(3, "Hyper", 3, 1, 90.0, 90.0, false, 0, "Other")]));
        assert_eq!(kinds(&l), vec!["pit_in", "penalty", "pit_out", "driver"]);
        assert!(l.events[1].own && l.events[1].severity == "alert");
        assert!(l.events[2].text.contains("40.0 s"), "{}", l.events[2].text);
        assert_eq!(l.events[3].text, "Driver change: Alice → Bob");
        // The next lap of our car went through the pits.
        l.on_message(&scoring(250.0, vec![car(1, "GT3", 2, 2, 140.0, 100.0, false, 1, "Bob"), car(2, "GT3", 1, 2, 99.0, 99.0, false, 0, "Rival"), car(3, "Hyper", 3, 2, 90.0, 90.0, false, 0, "Other")]));
        assert_eq!(l.laps[&1].last().unwrap(), &LapTime { lap: 2, time: 140.0, pit: true });
        assert!(!l.laps[&2].last().unwrap().pit);
    }

    #[test]
    fn class_position_changes_are_merged_in_a_burst() {
        let mut l = RaceLog::new();
        let field = |t: f64, us: i64| scoring(t, vec![car(1, "GT3", us, 3, 100.0, 99.0, false, 0, "A"), car(2, "GT3", 2, 3, 100.0, 99.5, false, 0, "B"), car(4, "GT3", 3, 3, 100.0, 99.6, false, 0, "C")]);
        l.on_message(&field(0.0, 1));
        l.on_message(&field(10.0, 2)); // lost P1 → P2 … (B now first)
        l.on_message(&field(12.0, 4)); // … and P3 within seconds
        let pos: Vec<&RaceEvent> = l.events.iter().filter(|e| e.kind == "position").collect();
        assert_eq!(pos.len(), 1);
        assert_eq!(pos[0].text, "Class P1 → P3 of 3 (lost 2 places)");
        // Back to P1 in the same burst: nothing left to report.
        l.on_message(&field(14.0, 0));
        assert!(l.events.iter().all(|e| e.kind != "position"));
    }

    #[test]
    fn flags_impacts_and_best_laps() {
        let mut l = RaceLog::new();
        l.on_message(&scoring(0.0, vec![car(1, "GT3", 1, 1, 100.0, 100.0, false, 0, "A"), car(2, "GT3", 2, 1, 101.0, 101.0, false, 0, "B")]));
        let status = |phase: i64, yellow: i64, et: f64, mag: f64, flag: i64| {
            v(serde_json::json!({"type": "VehicleStatusUpdate", "game_phase": phase, "yellow_flag_state": yellow,
                                 "last_impact_et": et, "last_impact_magnitude": mag, "player_flag": flag}))
        };
        l.on_message(&status(5, 0, 0.0, 0.0, 0));
        l.on_message(&status(6, 2, 0.0, 0.0, 0));
        l.on_message(&status(6, 4, 5.0, 1200.0, 6)); // small knock: not logged
        l.on_message(&status(5, 0, 9.0, 5200.0, 6)); // big one; blue flag still on
        assert_eq!(kinds(&l), vec!["fcy", "pits", "pits", "blue_flag", "fcy_end", "impact"]);
        assert_eq!(l.events[1].text, "Pit lane closed");
        // Personal best, then the rival takes the class best.
        l.on_message(&scoring(200.0, vec![car(1, "GT3", 1, 2, 98.0, 98.0, false, 0, "A"), car(2, "GT3", 2, 2, 101.0, 101.0, false, 0, "B")]));
        l.on_message(&scoring(300.0, vec![car(1, "GT3", 1, 3, 99.0, 98.0, false, 0, "A"), car(2, "GT3", 2, 3, 97.5, 97.5, false, 0, "B")]));
        let k = kinds(&l);
        assert_eq!(&k[6..], &["best_lap", "class_best", "class_best"]);
        assert!(l.events.last().unwrap().text.contains("#2 B"));
    }

    #[test]
    fn a_new_session_starts_a_new_log() {
        let mut l = RaceLog::new();
        l.on_message(&scoring(500.0, vec![car(1, "GT3", 1, 5, 100.0, 99.0, false, 0, "A")]));
        l.on_message(&scoring(510.0, vec![car(1, "GT3", 1, 5, 100.0, 99.0, true, 0, "A")]));
        assert!(!l.events.is_empty());
        l.on_message(&scoring(1.0, vec![car(1, "GT3", 1, 0, -1.0, -1.0, false, 0, "A")]));
        assert!(l.events.is_empty());
        assert!(l.laps.is_empty());
    }
}
