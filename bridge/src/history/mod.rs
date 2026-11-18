//! Session history: everything worth keeping from a session, saved to SQLite
//! as it happens, for the post-race analysis and for comparing stints.
//!
//! Like the strategy engine, the recorder reads the broadcast stream, so it
//! works the same on a driver's bridge, a hub, a replay and in the engineer
//! seat. Per session it keeps:
//!
//! * our car's laps: time, driver, pit and caution flags, class position, and
//!   fuel, VE and tyre wear at the line (fuel and wear only when read on the
//!   PC that was driving, the same trust rule as the strategy engine);
//! * every car's lap times, to compare rivals afterwards;
//! * our stints, built with the hub's stint rules;
//! * the race events, the strategy plan at each lap and the AI comments.
//!
//! A session starts when our car first appears in a live session and ends at
//! a session change; one without a completed lap is not listed.

pub mod store;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rmpv::Value;
use rusqlite::Connection;
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::hub::logic::{num, ScoringSummary, StintTracker};
use crate::protocol::messages::ServerMessage;
use crate::recording::{field, message_type, to_raw};
use store::{CarLap, OwnLap};

const TELEMETRY_MAX_AGE_S: f64 = 3.0;

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Default, Clone)]
struct CarState {
    total_laps: i64,
    lap_pit: bool,
}

/// What the recorder writes, as plain operations (so the logic is testable
/// without a database).
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    NewSession { track: String, session_type: String, car_name: String, car_class: String, followed: bool },
    OwnLap(OwnLap),
    CarLaps(Vec<CarLap>),
    Stints(Vec<crate::hub::logic::Stint>),
    Events { first_seq: usize, events: Vec<crate::race::RaceEvent> },
    Strategy { lap: i64, json: String },
    Ai(crate::ai::AiComment),
    Touch { track: String },
}

#[derive(Default)]
pub struct Recorder {
    open: bool,
    session_type: String,
    session_time: f64,
    track: String,
    cars: HashMap<i64, CarState>,
    own_laps: i64,
    own_lap_pit: bool,
    own_caution: bool,
    game_phase: i64,
    fuel: Option<f64>,
    wear: Option<[f64; 4]>,
    trusted_at: Option<Instant>,
    driving: bool,
    stints: StintTracker,
    events_written: usize,
    strategy_lap: i64,
    ai_written: u64,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    fn close(&mut self) {
        // AI comment ids run on across sessions: keep the high-water mark.
        *self = Recorder { ai_written: self.ai_written, ..Recorder::default() };
    }

    pub fn on_message(&mut self, msg: &ServerMessage) -> Vec<Op> {
        match msg {
            ServerMessage::RaceTiming(t) => return self.on_events(&t.events),
            ServerMessage::StrategyUpdate(s) => {
                if self.open && s.laps_completed > self.strategy_lap {
                    self.strategy_lap = s.laps_completed;
                    if let Ok(json) = serde_json::to_string(&**s) {
                        return vec![Op::Strategy { lap: s.laps_completed, json }];
                    }
                }
                return Vec::new();
            }
            ServerMessage::AiFeed(f) => {
                // Comments seen while nothing was recorded (practice, menus)
                // count as handled, so they never land in the next session.
                let new: Vec<Op> = if self.open {
                    f.comments.iter().filter(|c| c.id > self.ai_written).map(|c| Op::Ai(c.clone())).collect()
                } else {
                    Vec::new()
                };
                if let Some(max) = f.comments.iter().map(|c| c.id).max() {
                    self.ai_written = self.ai_written.max(max);
                }
                return new;
            }
            _ => {}
        }
        let Some(v) = to_raw(msg) else { return Vec::new() };
        match message_type(&v) {
            Some("ScoringUpdate") => self.on_scoring(&v),
            Some("TelemetryUpdate") => {
                if self.driving {
                    self.fuel = field(&v, "fuel").and_then(num);
                    self.wear = field(&v, "tires").and_then(|t| t.as_array()).and_then(|t| {
                        let w: Vec<f64> = t.iter().filter_map(|x| field(x, "wear").and_then(num)).collect();
                        (w.len() == 4 && w.iter().any(|x| *x > 0.0)).then(|| [w[0], w[1], w[2], w[3]])
                    });
                    self.trusted_at = Some(Instant::now());
                    if let Some(f) = self.fuel {
                        self.stints.on_fuel(f);
                    }
                }
                Vec::new()
            }
            Some("SessionInfo") => {
                let track = field(&v, "track_name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                if !track.is_empty() && !self.track.is_empty() && track != self.track {
                    self.close();
                }
                self.track = track;
                Vec::new()
            }
            Some("VehicleStatusUpdate") => {
                self.game_phase = field(&v, "game_phase").and_then(num).unwrap_or(0.0) as i64;
                if self.game_phase == 6 {
                    self.own_caution = true;
                }
                Vec::new()
            }
            Some("ConnectionStatus") => {
                let active = field(&v, "session_active").and_then(|x| x.as_bool()).unwrap_or(true);
                if !active && self.open {
                    info!("History: session closed");
                    self.close();
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_events(&mut self, events: &[crate::race::RaceEvent]) -> Vec<Op> {
        if !self.open {
            return Vec::new();
        }
        if events.len() < self.events_written {
            // The race log started over (new session): start over too.
            self.events_written = 0;
        }
        if events.len() == self.events_written {
            return Vec::new();
        }
        let first = self.events_written;
        self.events_written = events.len();
        vec![Op::Events { first_seq: first, events: events[first..].to_vec() }]
    }

    fn on_scoring(&mut self, v: &Value) -> Vec<Op> {
        let mut ops = Vec::new();
        let t = field(v, "session_time").and_then(num).unwrap_or(0.0);
        let stype = field(v, "session_type").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if self.open && (stype != self.session_type || t < self.session_time - 5.0) {
            self.close();
        }
        self.session_type = stype.clone();
        self.session_time = t;

        let Some(rows) = field(v, "vehicles").and_then(|x| x.as_array()) else { return ops };
        let pid = field(v, "player_vehicle_id").and_then(|x| x.as_i64()).unwrap_or(-1);
        let me = rows.iter().find(|r| field(r, "id").and_then(|x| x.as_i64()) == Some(pid));
        let s = |r: &Value, k: &str| field(r, k).and_then(|x| x.as_str()).unwrap_or("").to_string();

        if !self.open {
            // Practice before the race is not recorded.
            if crate::session_kind::is_practice(&stype) {
                return ops;
            }
            let Some(me) = me else { return ops };
            self.open = true;
            self.own_laps = field(me, "total_laps").and_then(num).unwrap_or(0.0) as i64;
            self.strategy_lap = self.own_laps;
            ops.push(Op::NewSession {
                track: self.track.clone(),
                session_type: stype,
                car_name: s(me, "car_name"),
                car_class: s(me, "vehicle_class"),
                followed: field(v, "followed").and_then(|x| x.as_bool()).unwrap_or(false),
            });
            info!("History: recording a new session");
        }

        // Stints, with the hub's rules.
        if let Some(summary) = ScoringSummary::from_msg(v) {
            self.driving = summary.driving();
            let source = if summary.pc_player.is_empty() { "this PC".to_string() } else { summary.pc_player.clone() };
            if self.stints.on_scoring(&source, &summary) {
                ops.push(Op::Stints(self.stints.stints().to_vec()));
            }
        }

        // Every car's laps.
        let mut car_laps = Vec::new();
        for r in rows {
            let Some(id) = field(r, "id").and_then(|x| x.as_i64()) else { continue };
            let total = field(r, "total_laps").and_then(num).unwrap_or(0.0) as i64;
            let in_pits = field(r, "in_pits").and_then(|x| x.as_bool()).unwrap_or(false);
            let st = self.cars.entry(id).or_insert(CarState { total_laps: total, lap_pit: in_pits });
            if total > st.total_laps {
                let lt = field(r, "last_lap_time").and_then(num).filter(|x| *x > 0.0);
                car_laps.push(CarLap {
                    car_id: id,
                    lap: total,
                    car_name: s(r, "car_name"),
                    driver: s(r, "driver_name"),
                    class: s(r, "vehicle_class"),
                    lap_time: lt,
                    pit: st.lap_pit || in_pits,
                });
                st.lap_pit = in_pits;
            } else if in_pits {
                st.lap_pit = true;
            }
            st.total_laps = total;
        }
        if !car_laps.is_empty() {
            ops.push(Op::CarLaps(car_laps));
        }

        // Our laps, with consumables.
        if let Some(me) = me {
            let total = field(me, "total_laps").and_then(num).unwrap_or(0.0) as i64;
            let in_pits = field(me, "in_pits").and_then(|x| x.as_bool()).unwrap_or(false);
            if in_pits {
                self.own_lap_pit = true;
            }
            if total > self.own_laps {
                let fresh = self.trusted_at.map(|a| a.elapsed().as_secs_f64() < TELEMETRY_MAX_AGE_S).unwrap_or(false);
                let trusted = fresh && self.driving;
                let ve = field(me, "virtual_energy").and_then(num).filter(|x| *x > 0.0);
                let class = s(me, "vehicle_class");
                let my_pos = field(me, "position").and_then(num).unwrap_or(0.0);
                let class_pos = rows
                    .iter()
                    .filter(|r| s(r, "vehicle_class") == class && field(r, "position").and_then(num).unwrap_or(f64::MAX) <= my_pos)
                    .count() as i64;
                ops.push(Op::OwnLap(OwnLap {
                    lap: total,
                    session_time: t,
                    lap_time: field(me, "last_lap_time").and_then(num).filter(|x| *x > 0.0),
                    driver: s(me, "driver_name"),
                    pit: self.own_lap_pit,
                    caution: self.own_caution,
                    fuel: if trusted { self.fuel } else { None },
                    ve,
                    wear: if trusted { self.wear } else { None },
                    class_position: (class_pos > 0).then_some(class_pos),
                }));
                ops.push(Op::Touch { track: self.track.clone() });
                self.own_laps = total;
                self.own_lap_pit = in_pits;
                self.own_caution = self.game_phase == 6;
            }
        }
        ops
    }
}

/// Apply operations to the database; returns the session id in use.
pub fn apply(conn: &mut Connection, sid: &mut Option<i64>, ops: Vec<Op>) -> anyhow::Result<()> {
    for op in ops {
        match op {
            Op::NewSession { track, session_type, car_name, car_class, followed } => {
                *sid = Some(store::new_session(conn, now_unix(), &track, &session_type, &car_name, &car_class, followed)?);
            }
            other => {
                let Some(id) = *sid else { continue };
                match other {
                    Op::OwnLap(l) => store::insert_own_lap(conn, id, &l)?,
                    Op::CarLaps(l) => store::insert_car_laps(conn, id, &l)?,
                    Op::Stints(s) => store::upsert_stints(conn, id, &s)?,
                    Op::Events { first_seq, events } => store::insert_events(conn, id, first_seq, &events)?,
                    Op::Strategy { lap, json } => store::insert_strategy(conn, id, lap, &json)?,
                    Op::Ai(c) => store::insert_ai(conn, id, &c)?,
                    Op::Touch { track } => store::touch_session(conn, id, now_unix(), &track)?,
                    Op::NewSession { .. } => unreachable!(),
                }
            }
        }
    }
    Ok(())
}

/// Run on a broadcast stream until it closes.
/// Record the stream into the history database of `dir`.
pub async fn run(mut rx: broadcast::Receiver<Arc<ServerMessage>>, dir: std::path::PathBuf) {
    let db = match store::db(&dir) {
        Ok(db) => db,
        Err(e) => {
            warn!("History disabled: {:#}", e);
            return;
        }
    };
    let mut rec = Recorder::new();
    let mut sid: Option<i64> = None;
    loop {
        match rx.recv().await {
            Ok(msg) => {
                let ops = rec.on_message(&msg);
                if ops.is_empty() {
                    continue;
                }
                let mut conn = match db.lock() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                if let Err(e) = apply(&mut conn, &mut sid, ops) {
                    warn!("History write failed: {:#}", e);
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => warn!("History recorder fell behind by {} messages", n),
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

// ---------------------------------------------------------------------------
// Commands from dashboards
// ---------------------------------------------------------------------------

use crate::protocol::messages::ClientCommand;

/// Answer a history command (called from a blocking task).
/// Answer a history command from the database of `dir`.
pub fn handle_command(cmd: ClientCommand, dir: &std::path::Path) -> ServerMessage {
    let result = (|| -> anyhow::Result<ServerMessage> {
        let db = store::db(dir)?;
        let conn = db.lock().map_err(|_| anyhow::anyhow!("history database busy"))?;
        Ok(match cmd {
            ClientCommand::HistoryList => ServerMessage::HistorySessions { sessions: store::list_sessions(&conn, 200)? },
            ClientCommand::HistorySession { session_id } => match store::session_detail(&conn, session_id)? {
                Some(d) => ServerMessage::HistorySession(Box::new(d)),
                None => ServerMessage::HistoryError { message: "Session not found.".into() },
            },
            ClientCommand::HistoryCarLaps { session_id, car_ids } => ServerMessage::HistoryCarLaps { session_id, laps: store::car_laps(&conn, session_id, &car_ids)? },
            ClientCommand::HistoryStrategy { session_id, lap } => ServerMessage::HistoryStrategy { session_id, lap, json: store::strategy_at(&conn, session_id, lap)? },
            ClientCommand::HistoryDelete { session_id } => {
                store::delete_session(&conn, session_id)?;
                ServerMessage::HistorySessions { sessions: store::list_sessions(&conn, 200)? }
            }
            _ => ServerMessage::HistoryError { message: "not a history command".into() },
        })
    })();
    result.unwrap_or_else(|e| ServerMessage::HistoryError { message: format!("{e:#}") })
}

pub fn is_history_command(cmd: &ClientCommand) -> bool {
    matches!(
        cmd,
        ClientCommand::HistoryList
            | ClientCommand::HistorySession { .. }
            | ClientCommand::HistoryCarLaps { .. }
            | ClientCommand::HistoryStrategy { .. }
            | ClientCommand::HistoryDelete { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(j: serde_json::Value) -> ServerMessage {
        ServerMessage::Raw(rmpv::ext::to_value(j).unwrap())
    }

    fn car(id: i64, cls: &str, pos: i64, laps: i64, last: f64, pits: bool, control: i64, driver: &str) -> serde_json::Value {
        serde_json::json!({"id": id, "vehicle_class": cls, "position": pos, "total_laps": laps, "last_lap_time": last,
            "in_pits": pits, "control": control, "driver_name": driver, "car_name": format!("Car #{id}"), "virtual_energy": 0.8})
    }

    fn scoring(t: f64, cars: Vec<serde_json::Value>) -> ServerMessage {
        scoring_in("Race", t, cars)
    }

    fn scoring_in(stype: &str, t: f64, cars: Vec<serde_json::Value>) -> ServerMessage {
        raw(serde_json::json!({"type": "ScoringUpdate", "session_type": stype, "session_time": t,
            "player_vehicle_id": 1, "player_name": "Alice PC", "vehicles": cars}))
    }

    fn ai_feed(ids: &[u64]) -> ServerMessage {
        let comments = ids.iter().map(|&id| crate::ai::AiComment { id, text: format!("comment {id}"), ..Default::default() }).collect();
        ServerMessage::AiFeed(Box::new(crate::ai::AiFeed { status: Default::default(), comments }))
    }

    fn tel(fuel: f64) -> ServerMessage {
        let w = serde_json::json!({"wear": 0.95});
        raw(serde_json::json!({"type": "TelemetryUpdate", "fuel": fuel, "tires": [w, w, w, w]}))
    }

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        store::init(&c).unwrap();
        c
    }

    #[test]
    fn a_session_is_recorded_and_read_back() {
        let mut rec = Recorder::new();
        let mut conn = mem();
        let mut sid = None;
        let mut feed = |rec: &mut Recorder, conn: &mut Connection, sid: &mut Option<i64>, m: ServerMessage| {
            let ops = rec.on_message(&m);
            apply(conn, sid, ops).unwrap();
        };
        feed(&mut rec, &mut conn, &mut sid, raw(serde_json::json!({"type": "SessionInfo", "track_name": "Monza"})));
        feed(&mut rec, &mut conn, &mut sid, scoring(0.0, vec![car(1, "GT3", 2, 0, -1.0, false, 0, "Alice"), car(2, "GT3", 1, 0, -1.0, false, 2, "Rival")]));
        assert!(sid.is_some());
        for lap in 1..=3 {
            feed(&mut rec, &mut conn, &mut sid, tel(90.0 - 3.0 * lap as f64));
            feed(&mut rec, &mut conn, &mut sid, scoring(100.0 * lap as f64, vec![
                car(1, "GT3", 2, lap, 100.0 + lap as f64, false, 0, "Alice"),
                car(2, "GT3", 1, lap, 99.0, false, 2, "Rival"),
            ]));
        }
        let events = vec![crate::race::RaceEvent { t: 5.0, lap: 1, kind: "fcy".into(), severity: "alert".into(), car_id: None, own: false, text: "Full course yellow".into() }];
        feed(&mut rec, &mut conn, &mut sid, ServerMessage::RaceTiming(Box::new(crate::race::RaceTiming { events: events.clone(), laps: vec![] })));
        // The same events again: nothing duplicated.
        feed(&mut rec, &mut conn, &mut sid, ServerMessage::RaceTiming(Box::new(crate::race::RaceTiming { events, laps: vec![] })));

        let list = store::list_sessions(&conn, 10).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].track.as_str(), list[0].laps, list[0].car_class.as_str()), ("Monza", 3, "GT3"));
        assert_eq!(list[0].drivers, vec!["Alice"]);
        assert_eq!(list[0].best_lap, Some(101.0));

        let d = store::session_detail(&conn, list[0].id).unwrap().unwrap();
        assert_eq!(d.laps.len(), 3);
        assert_eq!(d.laps[2].fuel, Some(81.0));
        assert_eq!(d.laps[2].wear, [Some(0.95); 4]);
        assert_eq!(d.laps[0].class_position, Some(2));
        assert_eq!(d.laps[0].ve, Some(0.8));
        assert_eq!(d.events.len(), 1);
        assert_eq!(d.stints.len(), 1);
        assert_eq!(d.stints[0].driver.as_deref(), Some("Alice"));
        assert_eq!(d.stints[0].source.as_deref(), Some("Alice PC"));
        assert_eq!(d.cars.len(), 2);
        let laps = store::car_laps(&conn, list[0].id, &[2]).unwrap();
        assert_eq!(laps.len(), 3);
        assert!(laps.iter().all(|l| l.lap_time == Some(99.0)));
    }

    #[test]
    fn readings_from_a_pc_that_is_not_driving_are_not_kept() {
        let mut rec = Recorder::new();
        let mut conn = mem();
        let mut sid = None;
        for (t, lap) in [(0.0, 0), (100.0, 1), (200.0, 2)] {
            let ops = rec.on_message(&tel(50.0));
            apply(&mut conn, &mut sid, ops).unwrap();
            let ops = rec.on_message(&scoring(t, vec![car(1, "GT3", 1, lap, 100.0, false, 2, "Bob")]));
            apply(&mut conn, &mut sid, ops).unwrap();
        }
        let id = store::list_sessions(&conn, 10).unwrap()[0].id;
        let d = store::session_detail(&conn, id).unwrap().unwrap();
        assert!(d.laps.iter().all(|l| l.fuel.is_none() && l.wear == [None; 4]));
    }

    #[test]
    fn practice_is_not_recorded_and_leaves_nothing_behind() {
        let mut rec = Recorder::new();
        let mut conn = mem();
        let mut sid = None;
        let mut feed = |m: ServerMessage| {
            let ops = rec.on_message(&m);
            apply(&mut conn, &mut sid, ops).unwrap();
        };
        for (t, lap) in [(0.0, 0), (100.0, 1), (200.0, 2)] {
            feed(scoring_in("Practice", t, vec![car(1, "GT3", 1, lap, 100.0, false, 0, "A")]));
        }
        feed(ai_feed(&[1, 2]));
        // The race: recorded, without the practice comments.
        feed(scoring_in("Race", 0.0, vec![car(1, "GT3", 1, 0, -1.0, false, 0, "A")]));
        feed(ai_feed(&[1, 2, 3]));
        feed(scoring_in("Race", 100.0, vec![car(1, "GT3", 1, 1, 100.0, false, 0, "A")]));
        let list = store::list_sessions(&conn, 10).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].session_type, "Race");
        let d = store::session_detail(&conn, list[0].id).unwrap().unwrap();
        assert_eq!(d.ai.iter().map(|c| c.text.as_deref().unwrap_or("")).collect::<Vec<_>>(), vec!["comment 3"]);
    }

    #[test]
    fn a_new_session_gets_a_new_row() {
        let mut rec = Recorder::new();
        let mut conn = mem();
        let mut sid = None;
        for m in [scoring(500.0, vec![car(1, "GT3", 1, 5, 100.0, false, 0, "A")]), scoring(600.0, vec![car(1, "GT3", 1, 6, 100.0, false, 0, "A")])] {
            let ops = rec.on_message(&m);
            apply(&mut conn, &mut sid, ops).unwrap();
        }
        let first = sid;
        // Clock back to the start: a new session.
        for m in [scoring(1.0, vec![car(1, "GT3", 1, 0, -1.0, false, 0, "A")]), scoring(100.0, vec![car(1, "GT3", 1, 1, 100.0, false, 0, "A")])] {
            let ops = rec.on_message(&m);
            apply(&mut conn, &mut sid, ops).unwrap();
        }
        assert_ne!(first, sid);
        assert_eq!(store::list_sessions(&conn, 10).unwrap().len(), 2);
        store::delete_session(&conn, sid.unwrap()).unwrap();
        assert_eq!(store::list_sessions(&conn, 10).unwrap().len(), 1);
    }
}
