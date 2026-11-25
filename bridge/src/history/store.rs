//! The session history database (`history.sqlite`): schema, writes, queries.
//!
//! Separate from the post-race database (which imports LMU's result files):
//! this one is filled live by [`super::Recorder`] from the stream, so it also
//! covers online races, whose result files carry no fuel, VE or wear, and
//! sessions followed from the engineer seat or through a hub.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
CREATE TABLE IF NOT EXISTS h_sessions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at    INTEGER NOT NULL,          -- unix seconds
    last_seen     INTEGER NOT NULL,
    track         TEXT    NOT NULL DEFAULT '',
    session_type  TEXT    NOT NULL DEFAULT '',
    car_name      TEXT    NOT NULL DEFAULT '',
    car_class     TEXT    NOT NULL DEFAULT '',
    followed      INTEGER NOT NULL DEFAULT 0, -- watched from the engineer seat
    laps          INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS h_laps (
    session_id     INTEGER NOT NULL REFERENCES h_sessions(id) ON DELETE CASCADE,
    lap            INTEGER NOT NULL,
    session_time   REAL,
    lap_time       REAL,
    driver         TEXT,
    pit            INTEGER NOT NULL DEFAULT 0,
    caution        INTEGER NOT NULL DEFAULT 0,
    fuel           REAL,      -- litres at the line, only from the driving PC
    ve             REAL,      -- fraction 0-1
    wear_fl REAL, wear_fr REAL, wear_rl REAL, wear_rr REAL,  -- tread left 0-1
    class_position INTEGER,
    PRIMARY KEY (session_id, lap)
);
CREATE TABLE IF NOT EXISTS h_car_laps (
    session_id INTEGER NOT NULL REFERENCES h_sessions(id) ON DELETE CASCADE,
    car_id     INTEGER NOT NULL,
    lap        INTEGER NOT NULL,
    car_name   TEXT,
    driver     TEXT,
    class      TEXT,
    lap_time   REAL,
    pit        INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, car_id, lap)
);
CREATE TABLE IF NOT EXISTS h_stints (
    session_id INTEGER NOT NULL REFERENCES h_sessions(id) ON DELETE CASCADE,
    number     INTEGER NOT NULL,
    driver     TEXT, source TEXT,
    start_time REAL, end_time REAL, start_lap INTEGER, end_lap INTEGER, laps INTEGER,
    best_lap REAL, avg_lap REAL, fuel_start REAL, fuel_end REAL, ve_start REAL, ve_end REAL,
    PRIMARY KEY (session_id, number)
);
CREATE TABLE IF NOT EXISTS h_events (
    session_id INTEGER NOT NULL REFERENCES h_sessions(id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    t REAL, lap INTEGER, kind TEXT, severity TEXT, car_id INTEGER, own INTEGER, text TEXT,
    PRIMARY KEY (session_id, seq)
);
CREATE TABLE IF NOT EXISTS h_strategy (
    session_id INTEGER NOT NULL REFERENCES h_sessions(id) ON DELETE CASCADE,
    lap        INTEGER NOT NULL,
    json       TEXT NOT NULL,
    PRIMARY KEY (session_id, lap)
);
CREATE TABLE IF NOT EXISTS h_changes (
    session_id INTEGER NOT NULL REFERENCES h_sessions(id) ON DELETE CASCADE,
    at INTEGER NOT NULL, session_time REAL, lap INTEGER, who TEXT, text TEXT
);
CREATE TABLE IF NOT EXISTS h_ai (
    session_id INTEGER NOT NULL REFERENCES h_sessions(id) ON DELETE CASCADE,
    comment_id INTEGER NOT NULL,
    lap INTEGER, trigger TEXT, question TEXT, text TEXT, withheld INTEGER, unverified TEXT, model TEXT, cost_usd REAL,
    PRIMARY KEY (session_id, comment_id)
);
";

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let conn = Connection::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    init(&conn)?;
    Ok(conn)
}

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA).context("history schema")?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(())
}

/// The history database of a data folder, `<dir>\history.sqlite`: the main
/// folder on a driver's bridge, each team's folder on a hub. Opened once and
/// kept for the life of the process (a hub has a handful of teams).
pub fn db(dir: &std::path::Path) -> Result<&'static Mutex<Connection>> {
    type Open = Result<&'static Mutex<Connection>, String>;
    static DBS: OnceLock<Mutex<std::collections::HashMap<std::path::PathBuf, Open>>> = OnceLock::new();
    let mut dbs = DBS.get_or_init(Default::default).lock().map_err(|_| anyhow::anyhow!("history databases busy"))?;
    dbs.entry(dir.to_path_buf())
        .or_insert_with(|| {
            open(&dir.join("history.sqlite"))
                .map(|c| &*Box::leak(Box::new(Mutex::new(c))))
                .map_err(|e| format!("{e:#}"))
        })
        .clone()
        .map_err(|e| anyhow::anyhow!("{e}"))
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct OwnLap {
    pub lap: i64,
    pub session_time: f64,
    pub lap_time: Option<f64>,
    pub driver: String,
    pub pit: bool,
    pub caution: bool,
    pub fuel: Option<f64>,
    pub ve: Option<f64>,
    pub wear: Option<[f64; 4]>,
    pub class_position: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CarLap {
    pub car_id: i64,
    pub lap: i64,
    pub car_name: String,
    pub driver: String,
    pub class: String,
    pub lap_time: Option<f64>,
    pub pit: bool,
}

pub fn new_session(conn: &Connection, now: i64, track: &str, session_type: &str, car_name: &str, car_class: &str, followed: bool) -> Result<i64> {
    conn.execute(
        "INSERT INTO h_sessions (started_at, last_seen, track, session_type, car_name, car_class, followed) VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6)",
        params![now, track, session_type, car_name, car_class, followed as i64],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn touch_session(conn: &Connection, id: i64, now: i64, track: &str) -> Result<()> {
    conn.execute(
        "UPDATE h_sessions SET last_seen = ?2, track = CASE WHEN ?3 <> '' THEN ?3 ELSE track END,
         laps = (SELECT COUNT(*) FROM h_laps WHERE session_id = ?1) WHERE id = ?1",
        params![id, now, track],
    )?;
    Ok(())
}

pub fn insert_own_lap(conn: &Connection, sid: i64, l: &OwnLap) -> Result<()> {
    let w = l.wear.map(|w| w.map(Some)).unwrap_or([None; 4]);
    conn.execute(
        "INSERT OR REPLACE INTO h_laps (session_id, lap, session_time, lap_time, driver, pit, caution, fuel, ve, wear_fl, wear_fr, wear_rl, wear_rr, class_position)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![sid, l.lap, l.session_time, l.lap_time, l.driver, l.pit as i64, l.caution as i64, l.fuel, l.ve, w[0], w[1], w[2], w[3], l.class_position],
    )?;
    Ok(())
}

pub fn insert_car_laps(conn: &mut Connection, sid: i64, laps: &[CarLap]) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut st = tx.prepare_cached(
            "INSERT OR REPLACE INTO h_car_laps (session_id, car_id, lap, car_name, driver, class, lap_time, pit) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for l in laps {
            st.execute(params![sid, l.car_id, l.lap, l.car_name, l.driver, l.class, l.lap_time, l.pit as i64])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn upsert_stints(conn: &mut Connection, sid: i64, stints: &[crate::hub::logic::Stint]) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut st = tx.prepare_cached(
            "INSERT OR REPLACE INTO h_stints (session_id, number, driver, source, start_time, end_time, start_lap, end_lap, laps, best_lap, avg_lap, fuel_start, fuel_end, ve_start, ve_end)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        )?;
        for s in stints {
            st.execute(params![
                sid, s.number, s.driver, s.source, s.start_time, s.end_time, s.start_lap, s.end_lap,
                s.lap_times.len() as i64, s.best_lap(), s.avg_lap(), s.fuel_start, s.fuel_end, s.ve_start, s.ve_end
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn insert_events(conn: &mut Connection, sid: i64, first_seq: usize, events: &[crate::race::RaceEvent]) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut st = tx.prepare_cached(
            "INSERT OR REPLACE INTO h_events (session_id, seq, t, lap, kind, severity, car_id, own, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for (i, e) in events.iter().enumerate() {
            st.execute(params![sid, (first_seq + i) as i64, e.t, e.lap, e.kind, e.severity, e.car_id, e.own as i64, e.text])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn insert_strategy(conn: &Connection, sid: i64, lap: i64, json: &str) -> Result<()> {
    conn.execute("INSERT OR REPLACE INTO h_strategy (session_id, lap, json) VALUES (?1, ?2, ?3)", params![sid, lap, json])?;
    Ok(())
}

pub fn insert_ai(conn: &Connection, sid: i64, c: &crate::ai::AiComment) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO h_ai (session_id, comment_id, lap, trigger, question, text, withheld, unverified, model, cost_usd)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![sid, c.id as i64, c.lap, c.trigger, c.question, c.text, c.withheld as i64, c.unverified.join(","), c.model, c.cost_usd],
    )?;
    Ok(())
}

pub fn insert_change(conn: &Connection, sid: i64, c: &crate::changelog::ChangeEntry) -> Result<()> {
    conn.execute(
        "INSERT INTO h_changes (session_id, at, session_time, lap, who, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![sid, c.at, c.session_time, c.lap, c.who, c.text],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSummary {
    pub id: i64,
    pub started_at: i64,
    pub last_seen: i64,
    pub track: String,
    pub session_type: String,
    pub car_name: String,
    pub car_class: String,
    pub followed: bool,
    pub laps: i64,
    pub drivers: Vec<String>,
    pub best_lap: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LapRow {
    pub lap: i64,
    pub session_time: Option<f64>,
    pub lap_time: Option<f64>,
    pub driver: Option<String>,
    pub pit: bool,
    pub caution: bool,
    pub fuel: Option<f64>,
    pub ve: Option<f64>,
    pub wear: [Option<f64>; 4],
    pub class_position: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StintRow {
    pub number: i64,
    pub driver: Option<String>,
    pub source: Option<String>,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    pub start_lap: Option<i64>,
    pub end_lap: Option<i64>,
    pub laps: i64,
    pub best_lap: Option<f64>,
    pub avg_lap: Option<f64>,
    pub fuel_start: Option<f64>,
    pub fuel_end: Option<f64>,
    pub ve_start: Option<f64>,
    pub ve_end: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventRow {
    pub t: Option<f64>,
    pub lap: Option<i64>,
    pub kind: Option<String>,
    pub severity: Option<String>,
    pub own: bool,
    pub text: Option<String>,
}

/// A line of the change log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChangeRow {
    pub at: i64,
    pub session_time: Option<f64>,
    pub lap: Option<i64>,
    pub who: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiRow {
    pub lap: Option<i64>,
    pub trigger: Option<String>,
    pub question: Option<String>,
    pub text: Option<String>,
    pub withheld: bool,
    pub unverified: Option<String>,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
}

/// A car of the session, for picking rivals to compare.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CarSummary {
    pub car_id: i64,
    pub car_name: Option<String>,
    pub class: Option<String>,
    pub drivers: Vec<String>,
    pub laps: i64,
    pub best_lap: Option<f64>,
    /// Average of laps not through the pits.
    pub avg_lap: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionDetail {
    pub session: SessionSummary,
    pub laps: Vec<LapRow>,
    pub stints: Vec<StintRow>,
    pub events: Vec<EventRow>,
    pub ai: Vec<AiRow>,
    /// Who changed the strategy, targets, pin or AI, and when.
    pub changes: Vec<ChangeRow>,
    pub cars: Vec<CarSummary>,
    /// Laps with a strategy snapshot (fetch one with `history_strategy`).
    pub strategy_laps: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CarLapRow {
    pub car_id: i64,
    pub lap: i64,
    pub lap_time: Option<f64>,
    pub driver: Option<String>,
    pub pit: bool,
}

fn summary_from_row(conn: &Connection, r: &rusqlite::Row) -> rusqlite::Result<SessionSummary> {
    let id: i64 = r.get(0)?;
    let drivers: Vec<String> = conn
        .prepare_cached("SELECT DISTINCT driver FROM h_laps WHERE session_id = ?1 AND driver IS NOT NULL AND driver <> '' ORDER BY lap")?
        .query_map([id], |x| x.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let best: Option<f64> = conn
        .prepare_cached("SELECT MIN(lap_time) FROM h_laps WHERE session_id = ?1 AND lap_time > 0 AND pit = 0")?
        .query_row([id], |x| x.get(0))?;
    Ok(SessionSummary {
        id,
        started_at: r.get(1)?,
        last_seen: r.get(2)?,
        track: r.get(3)?,
        session_type: r.get(4)?,
        car_name: r.get(5)?,
        car_class: r.get(6)?,
        followed: r.get::<_, i64>(7)? != 0,
        laps: r.get(8)?,
        drivers,
        best_lap: best,
    })
}

const SESSION_COLS: &str = "id, started_at, last_seen, track, session_type, car_name, car_class, followed, laps";

/// Sessions with at least one lap, newest first.
pub fn list_sessions(conn: &Connection, limit: i64) -> Result<Vec<SessionSummary>> {
    let mut st = conn.prepare(&format!("SELECT {SESSION_COLS} FROM h_sessions WHERE laps > 0 ORDER BY started_at DESC LIMIT ?1"))?;
    let rows = st.query_map([limit], |r| summary_from_row(conn, r))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn session_detail(conn: &Connection, id: i64) -> Result<Option<SessionDetail>> {
    let session = conn
        .query_row(&format!("SELECT {SESSION_COLS} FROM h_sessions WHERE id = ?1"), [id], |r| summary_from_row(conn, r))
        .optional()?;
    let Some(session) = session else { return Ok(None) };

    let laps = conn
        .prepare("SELECT lap, session_time, lap_time, driver, pit, caution, fuel, ve, wear_fl, wear_fr, wear_rl, wear_rr, class_position FROM h_laps WHERE session_id = ?1 ORDER BY lap")?
        .query_map([id], |r| {
            Ok(LapRow {
                lap: r.get(0)?, session_time: r.get(1)?, lap_time: r.get(2)?, driver: r.get(3)?,
                pit: r.get::<_, i64>(4)? != 0, caution: r.get::<_, i64>(5)? != 0, fuel: r.get(6)?, ve: r.get(7)?,
                wear: [r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?], class_position: r.get(12)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let stints = conn
        .prepare("SELECT number, driver, source, start_time, end_time, start_lap, end_lap, laps, best_lap, avg_lap, fuel_start, fuel_end, ve_start, ve_end FROM h_stints WHERE session_id = ?1 ORDER BY number")?
        .query_map([id], |r| {
            Ok(StintRow {
                number: r.get(0)?, driver: r.get(1)?, source: r.get(2)?, start_time: r.get(3)?, end_time: r.get(4)?,
                start_lap: r.get(5)?, end_lap: r.get(6)?, laps: r.get::<_, Option<i64>>(7)?.unwrap_or(0), best_lap: r.get(8)?,
                avg_lap: r.get(9)?, fuel_start: r.get(10)?, fuel_end: r.get(11)?, ve_start: r.get(12)?, ve_end: r.get(13)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let events = conn
        .prepare("SELECT t, lap, kind, severity, own, text FROM h_events WHERE session_id = ?1 ORDER BY seq")?
        .query_map([id], |r| Ok(EventRow { t: r.get(0)?, lap: r.get(1)?, kind: r.get(2)?, severity: r.get(3)?, own: r.get::<_, Option<i64>>(4)?.unwrap_or(0) != 0, text: r.get(5)? }))?
        .collect::<rusqlite::Result<_>>()?;
    let ai = conn
        .prepare("SELECT lap, trigger, question, text, withheld, unverified, model, cost_usd FROM h_ai WHERE session_id = ?1 ORDER BY comment_id")?
        .query_map([id], |r| {
            Ok(AiRow { lap: r.get(0)?, trigger: r.get(1)?, question: r.get(2)?, text: r.get(3)?, withheld: r.get::<_, Option<i64>>(4)?.unwrap_or(0) != 0, unverified: r.get(5)?, model: r.get(6)?, cost_usd: r.get(7)? })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let cars = conn
        .prepare(
            "SELECT car_id, MAX(car_name), MAX(class), GROUP_CONCAT(DISTINCT driver), COUNT(*),
                    MIN(CASE WHEN lap_time > 0 AND pit = 0 THEN lap_time END),
                    AVG(CASE WHEN lap_time > 0 AND pit = 0 THEN lap_time END)
             FROM h_car_laps WHERE session_id = ?1 GROUP BY car_id ORDER BY MIN(CASE WHEN lap_time > 0 AND pit = 0 THEN lap_time END)",
        )?
        .query_map([id], |r| {
            let drivers: Option<String> = r.get(3)?;
            Ok(CarSummary {
                car_id: r.get(0)?, car_name: r.get(1)?, class: r.get(2)?,
                drivers: drivers.map(|d| d.split(',').map(str::to_string).collect()).unwrap_or_default(),
                laps: r.get(4)?, best_lap: r.get(5)?, avg_lap: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let changes = conn
        .prepare("SELECT at, session_time, lap, who, text FROM h_changes WHERE session_id = ?1 ORDER BY rowid")?
        .query_map([id], |r| {
            Ok(ChangeRow {
                at: r.get(0)?,
                session_time: r.get(1)?,
                lap: r.get(2)?,
                who: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                text: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let strategy_laps = conn
        .prepare("SELECT lap FROM h_strategy WHERE session_id = ?1 ORDER BY lap")?
        .query_map([id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(SessionDetail { session, laps, stints, events, ai, changes, cars, strategy_laps }))
}

pub fn car_laps(conn: &Connection, sid: i64, car_ids: &[i64]) -> Result<Vec<CarLapRow>> {
    let mut out = Vec::new();
    let mut st = conn.prepare("SELECT car_id, lap, lap_time, driver, pit FROM h_car_laps WHERE session_id = ?1 AND car_id = ?2 ORDER BY lap")?;
    for id in car_ids.iter().take(8) {
        let rows = st.query_map(params![sid, id], |r| Ok(CarLapRow { car_id: r.get(0)?, lap: r.get(1)?, lap_time: r.get(2)?, driver: r.get(3)?, pit: r.get::<_, i64>(4)? != 0 }))?;
        out.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
    }
    Ok(out)
}

pub fn strategy_at(conn: &Connection, sid: i64, lap: i64) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT json FROM h_strategy WHERE session_id = ?1 AND lap = ?2", params![sid, lap], |r| r.get(0)).optional()?)
}

pub fn delete_session(conn: &Connection, sid: i64) -> Result<()> {
    for t in ["h_laps", "h_car_laps", "h_stints", "h_events", "h_strategy", "h_ai", "h_changes"] {
        conn.execute(&format!("DELETE FROM {t} WHERE session_id = ?1"), [sid])?;
    }
    conn.execute("DELETE FROM h_sessions WHERE id = ?1", [sid])?;
    Ok(())
}
