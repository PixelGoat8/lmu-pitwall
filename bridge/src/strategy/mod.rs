//! Live strategy engine.
//!
//! Reads the same message stream a dashboard receives — the bridge's own
//! broadcast, or on a hub the stream relayed from whichever driver is in the
//! car — and recomputes the plan at every completed lap: consumption, laps
//! left, energy to the finish, stops, pit window, "box in N laps", tyre wear
//! and pace, pit lane loss, weather. Working from the stream is what lets the
//! one engine run unchanged on a bridge and on a hub, and keep its history
//! across driver swaps.
//!
//! The maths is in [`model`]; this file turns messages into lap samples and
//! the results into a `StrategyUpdate`.
//!
//! # Trusting fuel and tyre readings
//!
//! Online, LMU shows a placeholder for the fuel and tyre wear of a car driven
//! from another PC (measured: 50 L and 1.0 on every car). On a hub the stream
//! can come from such a PC between a driver swap and the switch to the new
//! driver. So fuel and wear are only sampled while the stream's own car is
//! under local control (`control == 0`), i.e. read on the PC that drives it.

pub mod model;
pub mod zones;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rmpv::Value;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

use crate::hub::logic::num;
use crate::pit_api::{PitEstimate, PitInfo};
use crate::protocol::messages::ServerMessage;
use crate::recording::{field, message_type};
use model::*;
use zones::{LicoCost, ZoneTracker};

/// How often the plan is re-sent between laps (time remaining moves on).
const REFRESH: Duration = Duration::from_secs(2);
/// A telemetry reading older than this is not used for a lap sample.
const TELEMETRY_MAX_AGE: Duration = Duration::from_secs(3);
/// Plausible pit lane loss (seconds): outside this a measurement is discarded.
const PIT_LOSS_RANGE: (f64, f64) = (3.0, 120.0);
/// Pit lane losses kept per track.
const PIT_LOSS_KEEP: usize = 10;
/// Coasting, for lift-and-coast: at least this fast (72 km/h, so slow
/// corners and the pit lane do not count)…
const COAST_MIN_SPEED_MS: f64 = 20.0;
/// …with throttle and brake both under this.
const COAST_PEDAL: f64 = 0.05;
/// A lap with fewer telemetry samples than this gets no coasting figure.
const COAST_MIN_SAMPLES: u32 = 20;
/// Another car this close ahead on track, or this close behind, metres: time
/// lost near it may be traffic, not lift-and-coast (see `zones`). A first
/// guess: following close (about half a second at speed) or being passed.
/// Wider, and in multiclass traffic most zones would be left out.
const TRAFFIC_AHEAD_M: f64 = 40.0;
const TRAFFIC_BEHIND_M: f64 = 20.0;

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// What the engineer sets; everything else is measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategyConfig {
    /// Energy kept in the tank at all times, in laps.
    #[serde(default = "default_reserve")]
    pub reserve_laps: f64,
    /// Longest stint a driver may do, minutes; `None` = no limit.
    #[serde(default)]
    pub max_stint_minutes: Option<f64>,
    /// Tread fraction (0–1) below which a tyre counts as finished.
    #[serde(default = "default_wear_limit")]
    pub tyre_wear_limit: f64,
    /// Fit new tyres at the stop the plan shows.
    #[serde(default)]
    pub change_tyres: bool,
    /// Pit lane loss to use instead of the measured one, seconds.
    #[serde(default)]
    pub pit_lane_loss_s: Option<f64>,
    /// Virtual energy per lap the engineer asks for, as a fraction (0.032 =
    /// 3.2 %/lap); `None` = no target.
    #[serde(default)]
    pub ve_target_per_lap: Option<f64>,
    /// Fuel per lap the engineer asks for, litres; `None` = no target.
    #[serde(default)]
    pub fuel_target_per_lap: Option<f64>,
    /// Target set in laps instead: make the virtual energy last to the end
    /// of this lap. Wins over `ve_target_per_lap`.
    #[serde(default)]
    pub ve_target_box_lap: Option<i64>,
    /// The same for fuel.
    #[serde(default)]
    pub fuel_target_box_lap: Option<i64>,
}

fn default_reserve() -> f64 {
    0.5
}
fn default_wear_limit() -> f64 {
    0.3
}

impl Default for StrategyConfig {
    fn default() -> Self {
        StrategyConfig {
            reserve_laps: default_reserve(),
            max_stint_minutes: None,
            tyre_wear_limit: default_wear_limit(),
            change_tyres: false,
            pit_lane_loss_s: None,
            ve_target_per_lap: None,
            fuel_target_per_lap: None,
            ve_target_box_lap: None,
            fuel_target_box_lap: None,
        }
    }
}

impl StrategyConfig {
    /// Clamp what a client sent into sane ranges.
    pub fn sanitized(mut self) -> Self {
        self.reserve_laps = self.reserve_laps.clamp(0.0, 5.0);
        self.tyre_wear_limit = self.tyre_wear_limit.clamp(0.0, 0.95);
        self.max_stint_minutes = self.max_stint_minutes.filter(|m| *m > 0.0 && *m < 24.0 * 60.0);
        self.pit_lane_loss_s = self.pit_lane_loss_s.filter(|s| *s > 0.0 && *s < 300.0);
        self.ve_target_per_lap = self.ve_target_per_lap.filter(|t| *t > 0.0 && *t < 1.0);
        self.fuel_target_per_lap = self.fuel_target_per_lap.filter(|t| *t > 0.0 && *t < 100.0);
        self.ve_target_box_lap = self.ve_target_box_lap.filter(|l| *l > 0 && *l < 100_000);
        self.fuel_target_box_lap = self.fuel_target_box_lap.filter(|l| *l > 0 && *l < 100_000);
        self
    }
}

fn load_json<T: serde::de::DeserializeOwned + Default>(dir: &Path, name: &str) -> T {
    std::fs::read(dir.join(name)).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
}

fn save_json<T: Serialize>(dir: &Path, name: &str, value: &T) {
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(value).unwrap_or_default())?;
        std::fs::rename(tmp, dir.join(name))
    };
    if let Err(e) = write() {
        warn!("Could not save {}: {}", name, e);
    }
}

/// The pit lane losses measured so far, one store for the whole process: on
/// a hub every team's engine adds to it, and a stop measured by one team is
/// known to the others. Kept in `pit_loss.json` in the main data folder.
fn shared_pit_loss() -> &'static Mutex<PitLossStore> {
    static STORE: OnceLock<Mutex<PitLossStore>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(load_json(&crate::paths::root(), "pit_loss.json")))
}

// ---------------------------------------------------------------------------
// Pit lane loss, per track, kept across sessions
// ---------------------------------------------------------------------------

/// Measured pit lane losses by track name: the time a stop costs on top of
/// the stationary time, compared with staying on track.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PitLossStore {
    #[serde(default)]
    tracks: HashMap<String, Vec<f64>>,
}

impl PitLossStore {
    pub fn add(&mut self, track: &str, loss_s: f64) {
        let v = self.tracks.entry(track.to_string()).or_default();
        v.push(loss_s);
        if v.len() > PIT_LOSS_KEEP {
            v.remove(0);
        }
    }

    /// Median of the measurements for a track, and how many there are.
    pub fn get(&self, track: &str) -> Option<(f64, u32)> {
        let v = self.tracks.get(track).filter(|v| !v.is_empty())?;
        let mut s = v.clone();
        s.sort_by(f64::total_cmp);
        let m = if s.len() % 2 == 1 { s[s.len() / 2] } else { (s[s.len() / 2 - 1] + s[s.len() / 2]) / 2.0 };
        Some((m, v.len() as u32))
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PaceInfo {
    /// Reference lap for planning: median of the last clean laps.
    pub ref_lap_s: Option<f64>,
    pub clean_laps: u32,
    pub stat: Option<Stat>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanSummary {
    /// Stops from here to the finish.
    pub stops: u32,
    /// What sets the stint length: "fuel", "ve" or "stint time".
    pub limiting: String,
    pub stint_laps_max: f64,
    /// How far the car can go from here before it must stop, laps.
    pub range_laps: f64,
    pub window: Option<PitWindow>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StintInfo {
    /// Session time the current stint started (pit exit), seconds.
    pub started_at: Option<f64>,
    pub elapsed_s: Option<f64>,
    pub limit_s: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PitLossInfo {
    /// The figure the plan uses.
    pub lane_loss_s: Option<f64>,
    /// "manual", "measured" or "none".
    pub source: String,
    pub samples: u32,
    /// The game's estimate for the pit menu as set, and the model's figure for
    /// the same work: the two side by side show how far to trust the model.
    pub game_estimate: Option<PitEstimate>,
}

/// Everything the engine knows, recomputed at each lap.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StrategyReport {
    pub track: String,
    pub session_type: String,
    pub session_time: f64,
    /// Laps completed by the car.
    pub laps_completed: i64,
    pub lap_progress: f64,
    pub driver: String,
    /// Laps sampled so far this session.
    pub samples: u32,
    pub horizon: Option<Horizon>,
    pub pace: PaceInfo,
    pub fuel: Option<EnergyPlan>,
    pub ve: Option<EnergyPlan>,
    pub plan: Option<PlanSummary>,
    /// The plan at the engineer's target consumption, when one is set.
    #[serde(default)]
    pub plan_at_target: Option<PlanSummary>,
    pub what_if: Vec<WhatIf>,
    pub tyres: TyrePlan,
    pub tyre_choice: Option<TyreChoice>,
    pub stint: StintInfo,
    pub pit_loss: PitLossInfo,
    pub weather: WeatherOutlook,
    /// What lift-and-coast costs, measured zone by zone (see `zones`).
    #[serde(default)]
    pub lico_cost: Option<LicoCost>,
    /// Why some figure is missing, in plain words.
    pub notes: Vec<String>,
    pub config: StrategyConfig,
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct Row {
    total_laps: i64,
    lap_dist: f64,
    in_pits: bool,
    pit_state: i64,
    control: Option<i64>,
    last_lap_time: f64,
    ve: f64,
    driver: String,
    lap_start_et: f64,
}

#[derive(Debug, Clone)]
struct PendingLoss {
    first_lap: i64,
    last_lap: i64,
    stationary_s: f64,
}

#[derive(Default)]
pub struct Engine {
    pub cfg: StrategyConfig,
    pub(crate) pit_loss: PitLossStore,
    laps: Vec<LapSample>,

    session_type: String,
    session_time: f64,
    track: String,
    track_length: f64,
    session_laps: i64,
    session_length_s: Option<f64>,
    time_remaining_s: Option<f64>,
    forecast: Vec<ForecastNode>,
    raining: f64,
    wetness: f64,
    game_phase: i64,

    row: Option<Row>,
    /// Latest fuel / capacity / wear read on the driving PC, and when.
    fuel: Option<f64>,
    fuel_capacity: Option<f64>,
    wear: Option<[f64; 4]>,
    trusted_at: Option<Instant>,
    pit_info: Option<PitInfo>,

    lap_pit: bool,
    lap_caution: bool,
    pit_entry_lap: Option<i64>,
    stationary_since: Option<f64>,
    stationary_s: f64,
    pending_loss: Option<PendingLoss>,
    /// The latest pit lane loss measured here, for the shared store.
    pub(crate) last_measured_loss: Option<(String, f64)>,
    stint_start: Option<f64>,

    /// Lift-and-coast on the lap in progress: telemetry samples on track,
    /// and those coasting. Counted only on a lap watched from its start.
    lap_samples: u32,
    lap_coast_samples: u32,
    lap_watched: bool,

    /// Lift-and-coast zone by zone, from the telemetry.
    zones: ZoneTracker,
    /// Another car close on track at the last scoring update.
    traffic: bool,
}

/// Stops and the pit window from the limits on a stint: for each, its name,
/// how far the car can go from here, the longest stint it allows, and the
/// stops it needs on its own (`None` for the stint time limit).
fn plan_from(limits: Vec<(&'static str, f64, f64, Option<u32>)>, to_go: Option<f64>, p: f64, current_lap: i64) -> Option<PlanSummary> {
    if limits.is_empty() {
        return None;
    }
    let mut stops = limits.iter().filter_map(|l| l.3).max().unwrap_or(0);
    let range = limits.iter().map(|l| l.1).fold(f64::INFINITY, f64::min);
    let (limiting, _, stint_max, _) = limits.iter().copied().min_by(|a, b| a.2.total_cmp(&b.2)).unwrap();
    // The stint limit can force stops the tank does not.
    if let Some(go) = to_go {
        if go > range {
            let more = ((go - range) / stint_max.max(1.0)).ceil() as u32;
            stops = stops.max(more.max(1));
        }
    }
    let window = to_go.and_then(|go| pit_window(go, p, current_lap, range, stint_max, stops));
    Some(PlanSummary {
        stops: window.as_ref().map(|w| w.stops).unwrap_or(stops),
        limiting: limiting.to_string(),
        stint_laps_max: stint_max,
        range_laps: range,
        window,
    })
}

/// The engineer's target for one energy: "box at the end of lap N" if set,
/// else a consumption per lap. `laps_completed + lap_progress` is where the
/// car is now.
fn target_spec(box_lap: Option<i64>, per_lap: Option<f64>, laps_completed: i64, lap_progress: f64) -> Option<TargetSpec> {
    match box_lap {
        Some(lap) => Some(TargetSpec::BoxAfter { lap, laps_to_box: lap as f64 - (laps_completed as f64 + lap_progress) }),
        None => per_lap.map(TargetSpec::PerLap),
    }
}

fn n(msg: &Value, key: &str) -> Option<f64> {
    field(msg, key).and_then(num)
}
fn s(msg: &Value, key: &str) -> String {
    field(msg, key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}
fn b(msg: &Value, key: &str) -> bool {
    field(msg, key).and_then(|v| v.as_bool()).unwrap_or(false)
}

impl Engine {
    pub fn new(cfg: StrategyConfig, pit_loss: PitLossStore) -> Self {
        Engine { cfg, pit_loss, ..Default::default() }
    }

    pub fn pit_loss_store(&self) -> &PitLossStore {
        &self.pit_loss
    }

    /// Forget the session (keeps the settings and the pit loss history).
    fn reset_session(&mut self) {
        *self = Engine { cfg: self.cfg.clone(), pit_loss: std::mem::take(&mut self.pit_loss), ..Default::default() };
    }

    /// Feed one message. Returns true when a lap was completed (the plan
    /// should go out now rather than at the next refresh), and whether a new
    /// pit loss measurement was recorded.
    pub fn on_message(&mut self, msg: &Value) -> (bool, bool) {
        match message_type(msg) {
            Some("ScoringUpdate") => self.on_scoring(msg),
            Some("TelemetryUpdate") => {
                self.on_telemetry(msg);
                (false, false)
            }
            Some("SessionInfo") => {
                self.on_session_info(msg);
                (false, false)
            }
            Some("VehicleStatusUpdate") => {
                self.game_phase = n(msg, "game_phase").map(|v| v as i64).unwrap_or(0);
                if self.game_phase == 6 {
                    self.lap_caution = true;
                }
                (false, false)
            }
            Some("PitInfo") => {
                if let Ok(p) = rmpv::ext::from_value::<PitInfo>(msg.clone()) {
                    self.pit_info = Some(p);
                }
                (false, false)
            }
            Some("ConnectionStatus") => {
                let active = field(msg, "session_active")
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| b(msg, "game_connected"));
                if !active && (!self.laps.is_empty() || self.row.is_some()) {
                    self.reset_session();
                }
                (false, false)
            }
            _ => (false, false),
        }
    }

    fn driving(&self) -> bool {
        self.row.as_ref().and_then(|r| r.control) == Some(0)
    }

    fn on_telemetry(&mut self, msg: &Value) {
        // Pedals and speed are live for any car, even from the engineer seat.
        self.sample_coasting(msg);
        self.sample_zones(msg);
        if !self.driving() {
            return;
        }
        self.fuel = n(msg, "fuel");
        self.fuel_capacity = n(msg, "fuel_capacity").filter(|c| *c > 0.0);
        self.wear = field(msg, "tires").and_then(|t| t.as_array()).and_then(|t| {
            if t.len() < 4 {
                return None;
            }
            let mut w = [0.0; 4];
            for (i, x) in w.iter_mut().enumerate() {
                *x = n(&t[i], "wear")?;
            }
            // All zero: no tyre data (garage, menus).
            w.iter().any(|v| *v > 0.0).then_some(w)
        });
        self.trusted_at = Some(Instant::now());
    }

    /// One telemetry sample towards the lap's coasting: on track, at speed,
    /// off both pedals. A fraction of samples, so the telemetry rate (or a
    /// replay speed) does not matter.
    fn sample_coasting(&mut self, msg: &Value) {
        let (Some(throttle), Some(brake), Some(speed)) = (n(msg, "throttle"), n(msg, "brake"), n(msg, "speed_ms")) else { return };
        if self.row.as_ref().map(|r| r.in_pits).unwrap_or(true) {
            return;
        }
        self.lap_samples += 1;
        if speed >= COAST_MIN_SPEED_MS && throttle < COAST_PEDAL && brake < COAST_PEDAL {
            self.lap_coast_samples += 1;
        }
    }

    /// One telemetry sample towards the lift-and-coast zones (see `zones`).
    /// Fuel only from the PC that is driving.
    fn sample_zones(&mut self, msg: &Value) {
        let (Some(et), Some(start), Some(speed), Some(throttle), Some(brake)) =
            (n(msg, "current_et"), n(msg, "lap_start_et"), n(msg, "speed_ms"), n(msg, "throttle"), n(msg, "brake"))
        else {
            return;
        };
        let Some((in_pits, row_start, laps, driver)) = self.row.as_ref().map(|r| (r.in_pits, r.lap_start_et, r.total_laps, r.driver.clone()))
        else {
            return;
        };
        let fuel = if self.driving() && !b(msg, "spectated") { n(msg, "fuel").filter(|f| *f >= 0.0) } else { None };
        let sample = zones::Sample {
            et,
            speed,
            coasting: speed >= COAST_MIN_SPEED_MS && throttle < COAST_PEDAL && brake < COAST_PEDAL,
            excluded: in_pits || self.game_phase == 6,
            traffic: self.traffic,
            fuel,
            // The ground plane: y is up in the game's world.
            pos: field(msg, "position").and_then(|p| Some((n(p, "x")?, n(p, "z")?))),
        };
        // The lap that closes when this sample starts a new one: scoring may
        // not have counted it yet.
        let closed = if row_start >= start - 0.5 { laps } else { laps + 1 };
        self.zones.on_sample(start, sample, self.track_length, closed, &driver);
    }

    /// Another car close on track, ahead or behind.
    fn close_car(&self, vehicles: &[Value], pid: i64, me: &Row) -> bool {
        let len = self.track_length;
        if len <= 0.0 || me.in_pits {
            return false;
        }
        vehicles.iter().any(|v| {
            field(v, "id").and_then(|i| i.as_i64()) != Some(pid)
                && !b(v, "in_pits")
                && n(v, "lap_dist").is_some_and(|d| {
                    let ahead = (d - me.lap_dist).rem_euclid(len);
                    ahead < TRAFFIC_AHEAD_M || len - ahead < TRAFFIC_BEHIND_M
                })
        })
    }

    fn on_session_info(&mut self, msg: &Value) {
        let track = s(msg, "track_name");
        if !self.track.is_empty() && track != self.track {
            self.reset_session();
        }
        self.track = track;
        self.track_length = n(msg, "track_length").unwrap_or(0.0);
        self.session_laps = n(msg, "session_laps").unwrap_or(0.0) as i64;
        self.session_length_s = n(msg, "session_minutes").filter(|m| *m > 0.0).map(|m| m * 60.0);
        self.time_remaining_s = n(msg, "time_remaining");
        if let Some(w) = field(msg, "weather") {
            self.raining = n(w, "rain_intensity").unwrap_or(0.0);
            self.wetness = n(w, "avg_path_wetness").unwrap_or(0.0);
            if let Some(nodes) = field(w, "forecast").and_then(|f| f.as_array()) {
                let count = nodes.len();
                self.forecast = nodes
                    .iter()
                    .enumerate()
                    .map(|(i, node)| ForecastNode {
                        at: if count > 1 { i as f64 / (count - 1) as f64 } else { 0.0 },
                        sky: n(node, "sky_type").unwrap_or(0.0) as i32,
                        rain_chance: n(node, "rain_chance").unwrap_or(0.0),
                    })
                    .collect();
            }
        }
    }

    fn on_scoring(&mut self, msg: &Value) -> (bool, bool) {
        let t = n(msg, "session_time").unwrap_or(0.0);
        let stype = s(msg, "session_type");
        if (!self.session_type.is_empty() && stype != self.session_type) || t < self.session_time - 5.0 {
            self.reset_session();
        }
        self.session_type = stype;
        self.session_time = t;

        let Some(pid) = field(msg, "player_vehicle_id").and_then(|v| v.as_i64()) else { return (false, false) };
        let Some(vehicles) = field(msg, "vehicles").and_then(|v| v.as_array()) else { return (false, false) };
        let Some(r) = vehicles.iter().find(|v| field(v, "id").and_then(|i| i.as_i64()) == Some(pid)) else {
            return (false, false);
        };
        let row = Row {
            total_laps: n(r, "total_laps").unwrap_or(0.0) as i64,
            lap_dist: n(r, "lap_dist").unwrap_or(0.0),
            in_pits: b(r, "in_pits"),
            pit_state: n(r, "pit_state").unwrap_or(0.0) as i64,
            control: field(r, "control").and_then(|v| v.as_i64()),
            last_lap_time: n(r, "last_lap_time").unwrap_or(-1.0),
            ve: n(r, "virtual_energy").unwrap_or(0.0),
            driver: s(r, "driver_name"),
            lap_start_et: n(r, "lap_start_et").unwrap_or(0.0),
        };
        self.traffic = self.close_car(vehicles, pid, &row);
        let prev = self.row.replace(row.clone());
        let lap_in_progress = row.total_laps + 1;
        let mut new_loss = false;

        if row.in_pits {
            self.lap_pit = true;
        }
        if self.game_phase == 6 {
            self.lap_caution = true;
        }

        // --- pit stop timing --------------------------------------------
        let was_in_pits = prev.as_ref().map(|p| p.in_pits).unwrap_or(row.in_pits);
        if row.in_pits && !was_in_pits {
            self.pit_entry_lap = Some(lap_in_progress);
            self.stationary_s = 0.0;
            self.stationary_since = None;
        }
        if row.in_pits {
            match (row.pit_state == 3, self.stationary_since) {
                (true, None) => self.stationary_since = Some(t),
                (false, Some(since)) => {
                    self.stationary_s += t - since;
                    self.stationary_since = None;
                }
                _ => {}
            }
        }
        if !row.in_pits && was_in_pits && prev.is_some() {
            if let Some(since) = self.stationary_since.take() {
                self.stationary_s += t - since;
            }
            if let Some(first) = self.pit_entry_lap.take() {
                self.pending_loss = Some(PendingLoss { first_lap: first, last_lap: lap_in_progress, stationary_s: self.stationary_s });
            }
            self.stint_start = Some(t);
        }
        if self.stint_start.is_none() && !row.in_pits {
            self.stint_start = Some(t);
        }

        // --- lap completed --------------------------------------------------
        let completed = prev.as_ref().map(|p| row.total_laps > p.total_laps).unwrap_or(false);
        if completed {
            let fresh = self.trusted_at.map(|at| at.elapsed() < TELEMETRY_MAX_AGE).unwrap_or(false);
            let trusted = fresh && self.driving();
            let coast_s = (self.lap_watched
                && self.lap_samples >= COAST_MIN_SAMPLES
                && row.last_lap_time > 0.0
                && !self.lap_pit
                && !self.lap_caution)
                .then(|| self.lap_coast_samples as f64 / self.lap_samples as f64 * row.last_lap_time);
            self.lap_samples = 0;
            self.lap_coast_samples = 0;
            self.lap_watched = true;
            self.laps.push(LapSample {
                lap: row.total_laps,
                session_time: t,
                lap_time: row.last_lap_time,
                fuel: if trusted { self.fuel } else { None },
                ve: (row.ve > 0.0).then_some(row.ve),
                wear: if trusted { self.wear } else { None },
                pit: self.lap_pit,
                caution: self.lap_caution,
                coast_s,
            });
            self.lap_pit = row.in_pits;
            self.lap_caution = self.game_phase == 6;
            new_loss = self.try_measure_pit_loss();
        }
        (completed, new_loss)
    }

    /// Once the out-lap is done: pit loss = (time of the laps the stop
    /// touched) − (as many reference laps) − (stationary time).
    fn try_measure_pit_loss(&mut self) -> bool {
        let Some(p) = self.pending_loss.clone() else { return false };
        let done = self.laps.last().map(|l| l.lap).unwrap_or(0);
        if done < p.last_lap {
            return false;
        }
        self.pending_loss = None;
        let touched: Vec<f64> = (p.first_lap..=p.last_lap)
            .filter_map(|lap| self.laps.iter().find(|l| l.lap == lap).map(|l| l.lap_time))
            .collect();
        let count = (p.last_lap - p.first_lap + 1) as usize;
        let Some(reference) = self.ref_lap() else { return false };
        if touched.len() != count || touched.iter().any(|t| *t <= 0.0) || self.track.is_empty() {
            return false;
        }
        let lane = touched.iter().sum::<f64>() - count as f64 * reference - p.stationary_s;
        if lane < PIT_LOSS_RANGE.0 || lane > PIT_LOSS_RANGE.1 {
            info!("Strategy: pit loss measurement {:.1} s discarded as implausible", lane);
            return false;
        }
        if crate::session_kind::is_practice(&self.session_type) {
            // Practice stops (garage visits, setup changes) do not feed the
            // figure the race plan uses.
            info!("Strategy: pit loss {:.1} s measured in {}, not kept", lane, self.session_type);
            return false;
        }
        info!("Strategy: pit lane loss at {} measured {:.1} s (stationary {:.1} s)", self.track, lane, p.stationary_s);
        self.pit_loss.add(&self.track, lane);
        self.last_measured_loss = Some((self.track.clone(), lane));
        true
    }

    fn ref_lap(&self) -> Option<f64> {
        let clean = clean_laps(&self.laps);
        let recent = &clean[clean.len().saturating_sub(5)..];
        Stat::of(recent).map(|s| s.median).or_else(|| {
            let any: Vec<(f64, f64)> = self
                .laps
                .iter()
                .filter(|l| l.lap_time > 0.0 && !l.pit)
                .map(|l| (l.lap as f64, l.lap_time))
                .collect();
            Stat::of(&any).map(|s| s.median)
        })
    }

    /// Whether there is anything to report.
    pub fn has_session(&self) -> bool {
        self.row.is_some()
    }

    pub fn report(&self) -> StrategyReport {
        let mut notes = Vec::new();
        let row = self.row.clone().unwrap_or_default();
        let p = if self.track_length > 0.0 && row.lap_dist > 0.0 {
            (row.lap_dist / self.track_length).clamp(0.0, 0.999)
        } else {
            0.0
        };
        let current_lap = row.total_laps + 1;

        // --- pace ---------------------------------------------------------
        let clean = clean_laps(&self.laps);
        let ref_lap = self.ref_lap();
        let pace = PaceInfo { ref_lap_s: ref_lap, clean_laps: clean.len() as u32, stat: Stat::of(&clean) };
        if ref_lap.is_none() {
            notes.push("No lap time yet: complete a lap for pace and the race length in laps.".into());
        }

        // --- how much race is left ----------------------------------------
        let time_remaining = self
            .time_remaining_s
            .filter(|t| *t >= 0.0)
            .or_else(|| self.session_length_s.map(|l| (l - self.session_time).max(0.0)));
        let laps_remaining = (self.session_laps > 0).then(|| (self.session_laps - row.total_laps) as f64);
        let horizon = horizon(time_remaining, laps_remaining, p, ref_lap);
        let to_go = horizon.as_ref().map(|h| h.laps_to_go);

        // --- energy -------------------------------------------------------
        let fuel_per_lap = Stat::of(&consumption(&self.laps, |l| l.fuel));
        let fuel_rate = planning_rate(&self.laps, |l| l.fuel);
        let fuel_now = self.fuel.or_else(|| self.laps.iter().rev().find_map(|l| l.fuel));
        let fuel_cap = self.pit_info.as_ref().and_then(|p| p.fuel_capacity).or(self.fuel_capacity);
        let fuel = match (fuel_per_lap, fuel_rate, fuel_now, fuel_cap) {
            (Some(st), Some(rate), Some(now), Some(cap)) => energy_plan(
                "L",
                now,
                cap,
                st,
                rate,
                self.cfg.reserve_laps,
                to_go,
                lico_saving_per_s(&self.laps, |l| l.fuel),
                target_spec(self.cfg.fuel_target_box_lap, self.cfg.fuel_target_per_lap, row.total_laps, p),
            ),
            _ => None,
        };
        if fuel.is_none() {
            notes.push(if self.driving() || self.laps.iter().any(|l| l.fuel.is_some()) {
                "Fuel consumption needs two consecutive green laps.".into()
            } else {
                "No fuel data: it is only read on the PC that is driving.".into()
            });
        }
        let has_ve = row.ve > 0.0 || self.laps.iter().any(|l| l.ve.is_some());
        let ve = if has_ve {
            let st = Stat::of(&consumption(&self.laps, |l| l.ve));
            let plan = st.zip(planning_rate(&self.laps, |l| l.ve)).and_then(|(st, rate)| {
                energy_plan(
                    "%",
                    row.ve,
                    1.0,
                    st,
                    rate,
                    self.cfg.reserve_laps,
                    to_go,
                    lico_saving_per_s(&self.laps, |l| l.ve),
                    target_spec(self.cfg.ve_target_box_lap, self.cfg.ve_target_per_lap, row.total_laps, p),
                )
            });
            if plan.is_none() {
                notes.push("Virtual energy use needs two consecutive green laps.".into());
            }
            plan
        } else {
            None
        };

        // --- stint time ---------------------------------------------------
        let elapsed = self.stint_start.map(|s| (self.session_time - s).max(0.0));
        let limit_s = self.cfg.max_stint_minutes.map(|m| m * 60.0);
        let stint = StintInfo { started_at: self.stint_start, elapsed_s: elapsed, limit_s };

        // --- plan ---------------------------------------------------------
        // (name, range from here, stint max, stops) for each limit.
        let stint_limit = match (limit_s, ref_lap) {
            (Some(lim), Some(lap)) => Some(("stint time", (lim - elapsed.unwrap_or(0.0)).max(0.0) / lap, lim / lap, None)),
            _ => None,
        };
        let measured = |e: &Option<EnergyPlan>, name: &'static str| e.as_ref().map(|e| (name, e.laps_left, e.stint_laps_max, e.stops));
        let plan = plan_from(
            [measured(&fuel, "fuel"), measured(&ve, "ve"), stint_limit].into_iter().flatten().collect(),
            to_go,
            p,
            current_lap,
        );
        // The same plan at the engineer's target consumption, where one is
        // set: the race the team intends to run, against the one the
        // consumption so far gives.
        let at_target = |e: &Option<EnergyPlan>, name: &'static str| {
            e.as_ref().map(|e| match &e.target {
                Some(t) => (name, t.laps_left, t.stint_laps_max, t.stops),
                None => (name, e.laps_left, e.stint_laps_max, e.stops),
            })
        };
        let has_target = [&fuel, &ve].iter().any(|e| e.as_ref().is_some_and(|e| e.target.is_some()));
        let plan_at_target = if has_target {
            plan_from(
                [at_target(&fuel, "fuel"), at_target(&ve, "ve"), stint_limit].into_iter().flatten().collect(),
                to_go,
                p,
                current_lap,
            )
        } else {
            None
        };
        if to_go.is_none() {
            notes.push("Race length unknown: no stop count or pit window.".into());
        }

        // --- pit loss and what-if -----------------------------------------
        let measured = self.pit_loss.get(&self.track);
        let (lane_loss, source, samples) = match (self.cfg.pit_lane_loss_s, measured) {
            (Some(m), _) => (Some(m), "manual", measured.map(|x| x.1).unwrap_or(0)),
            (None, Some((m, k))) => (Some(m), "measured", k),
            (None, None) => (None, "none", 0),
        };
        if lane_loss.is_none() {
            notes.push("Pit lane loss not measured yet at this track: it is after the first stop, or set it by hand.".into());
        }
        let pit_times = self.pit_info.as_ref().and_then(|p| p.times.clone());
        let tank = |e: &Option<EnergyPlan>| {
            e.as_ref().map(|e| Tank { now: e.now, capacity: e.capacity, per_lap: e.planning_per_lap, reserve: e.reserve })
        };
        let tyres_at_stop = if self.cfg.change_tyres { 4 } else { 0 };
        let what_if = match (plan.as_ref().and_then(|p| p.window.clone()), to_go) {
            (Some(w), Some(go)) => what_if(&w, current_lap, p, go, tank(&fuel), tank(&ve), pit_times.as_ref(), tyres_at_stop, lane_loss),
            _ => Vec::new(),
        };

        // --- tyres --------------------------------------------------------
        let tyres = tyre_plan(&self.laps, self.cfg.tyre_wear_limit);
        let tyre_choice = match (&pit_times, plan.as_ref(), what_if.iter().find(|r| r.reachable && r.stops_after == 0).or(what_if.first())) {
            (Some(pt), Some(pl), Some(row)) => {
                let stop_in = (1.0 - p) + row.in_laps as f64;
                let next = to_go.map(|g| (g - stop_in).min(pl.stint_laps_max)).unwrap_or(pl.stint_laps_max).max(0.0);
                Some(tyre_choice(&tyres, pt, stop_in, next, row.fuel_add.unwrap_or(0.0), row.ve_add.unwrap_or(0.0)))
            }
            _ => None,
        };

        StrategyReport {
            track: self.track.clone(),
            session_type: self.session_type.clone(),
            session_time: self.session_time,
            laps_completed: row.total_laps,
            lap_progress: p,
            driver: row.driver.clone(),
            samples: self.laps.len() as u32,
            horizon,
            pace,
            fuel,
            ve,
            plan,
            plan_at_target,
            what_if,
            tyres,
            tyre_choice,
            stint,
            pit_loss: PitLossInfo {
                lane_loss_s: lane_loss,
                source: source.into(),
                samples,
                game_estimate: self.pit_info.as_ref().and_then(|p| p.estimate.clone()),
            },
            weather: weather_outlook(&self.forecast, self.session_length_s, self.session_time, self.raining, self.wetness, ref_lap),
            lico_cost: self.zones.summary(),
            notes,
            config: self.cfg.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

/// Run the engine on a broadcast stream until it closes.
///
/// `rx` is a subscription to the same channel the plan goes out on; the
/// engine's own messages come back on it and are skipped.
///
/// `dir` holds this engine's settings (`strategy.json`): the main data folder
/// on a driver's bridge, the team's folder on a hub.
pub async fn run(
    mut rx: broadcast::Receiver<Arc<ServerMessage>>,
    tx: broadcast::Sender<Arc<ServerMessage>>,
    mut config_rx: mpsc::UnboundedReceiver<StrategyConfig>,
    dir: PathBuf,
) {
    let initial_loss = shared_pit_loss().lock().map(|s| s.clone()).unwrap_or_default();
    let mut engine = Engine::new(load_json::<StrategyConfig>(&dir, "strategy.json").sanitized(), initial_loss);
    let mut refresh = tokio::time::interval(REFRESH);
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let send = |engine: &Engine| {
        if engine.has_session() {
            let _ = tx.send(Arc::new(ServerMessage::StrategyUpdate(Box::new(engine.report()))));
        }
    };
    loop {
        tokio::select! {
            r = rx.recv() => match r {
                Ok(msg) => {
                    let value = match &*msg {
                        ServerMessage::StrategyUpdate(_) | ServerMessage::RaceTiming(_) | ServerMessage::AiFeed(_) => continue,
                        other => match crate::recording::to_raw(other) {
                            Some(v) => v,
                            None => continue,
                        },
                    };
                    let (lap_done, new_loss) = engine.on_message(&value);
                    if new_loss {
                        if let Some((track, loss)) = engine.last_measured_loss.take() {
                            if let Ok(mut shared) = shared_pit_loss().lock() {
                                shared.add(&track, loss);
                                save_json(&crate::paths::root(), "pit_loss.json", &*shared);
                                engine.pit_loss = shared.clone();
                            }
                        }
                    }
                    if lap_done {
                        send(&engine);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(k)) => warn!("Strategy engine fell behind by {} messages", k),
                Err(broadcast::error::RecvError::Closed) => return,
            },
            Some(cfg) = config_rx.recv() => {
                engine.cfg = cfg.sanitized();
                save_json(&dir, "strategy.json", &engine.cfg);
                send(&engine);
            }
            _ = refresh.tick() => send(&engine),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(j: serde_json::Value) -> Value {
        rmpv::ext::to_value(j).unwrap()
    }

    fn scoring(t: f64, laps: i64, dist: f64, in_pits: bool, pit_state: i64, last: f64, ve: f64, control: i64) -> Value {
        scoring_in("Race", t, laps, dist, in_pits, pit_state, last, ve, control)
    }

    #[allow(clippy::too_many_arguments)]
    fn scoring_in(stype: &str, t: f64, laps: i64, dist: f64, in_pits: bool, pit_state: i64, last: f64, ve: f64, control: i64) -> Value {
        v(serde_json::json!({
            "type": "ScoringUpdate", "session_type": stype, "session_time": t, "player_vehicle_id": 3,
            "vehicles": [{"id": 3, "total_laps": laps, "lap_dist": dist, "in_pits": in_pits, "pit_state": pit_state,
                          "control": control, "last_lap_time": last, "virtual_energy": ve, "driver_name": "Alice"}]
        }))
    }

    fn telemetry(fuel: f64, wear: f64) -> Value {
        let tyre = serde_json::json!({"wear": wear});
        v(serde_json::json!({"type": "TelemetryUpdate", "fuel": fuel, "fuel_capacity": 100.0,
                             "tires": [tyre, tyre, tyre, tyre]}))
    }

    fn session_info() -> Value {
        v(serde_json::json!({"type": "SessionInfo", "track_name": "Test Ring", "track_length": 5000.0,
                             "session_laps": 0, "session_minutes": 60.0, "time_remaining": null,
                             "weather": {"rain_intensity": 0.0, "avg_path_wetness": 0.0, "forecast": []}}))
    }

    /// Drive `laps` laps of 90 s at 3 L and 4 % VE a lap, starting at session time `t0`.
    fn drive(e: &mut Engine, from_lap: i64, laps: i64, t0: f64, fuel0: f64, ve0: f64) -> f64 {
        let mut t = t0;
        for i in 0..laps {
            let lap = from_lap + i;
            // mid-lap sample, then the line
            e.on_message(&telemetry(fuel0 - 3.0 * i as f64 - 1.5, 1.0 - 0.01 * lap as f64));
            e.on_message(&scoring(t + 45.0, lap, 2500.0, false, 0, if lap > 0 { 90.0 } else { -1.0 }, ve0 - 0.04 * i as f64 - 0.02, 0));
            e.on_message(&telemetry(fuel0 - 3.0 * (i + 1) as f64, 1.0 - 0.01 * (lap + 1) as f64));
            e.on_message(&scoring(t + 90.0, lap + 1, 10.0, false, 0, 90.0, ve0 - 0.04 * (i + 1) as f64, 0));
            t += 90.0;
        }
        t
    }

    #[test]
    fn a_stint_gives_consumption_horizon_and_a_window() {
        let mut e = Engine::new(StrategyConfig::default(), PitLossStore::default());
        e.on_message(&session_info());
        e.on_message(&scoring(0.0, 0, 10.0, false, 0, -1.0, 1.0, 0));
        let t = drive(&mut e, 0, 6, 0.0, 90.0, 1.0);
        let r = e.report();
        assert_eq!(r.laps_completed, 6);
        assert_eq!(r.pace.ref_lap_s, Some(90.0));
        let fuel = r.fuel.as_ref().unwrap();
        assert!((fuel.planning_per_lap - 3.0).abs() < 1e-9);
        assert!((fuel.now - 72.0).abs() < 1e-9);
        let ve = r.ve.as_ref().unwrap();
        assert!((ve.planning_per_lap - 0.04).abs() < 1e-9);
        // 60-minute race, 540 s gone: 3060 s left at 90 s/lap → 34 laps.
        let h = r.horizon.as_ref().unwrap();
        assert!((h.time_remaining_s.unwrap() - (3600.0 - t)).abs() < 1e-6);
        assert!((h.laps_to_go - 34.0).abs() < 1.0);
        // VE: 0.76 left at 4 %/lap → 18.5 laps of range, the limit.
        let plan = r.plan.as_ref().unwrap();
        assert_eq!(plan.limiting, "ve");
        assert_eq!(plan.stops, 1);
        assert!(plan.window.is_some());
        assert!(!r.what_if.is_empty());
        assert_eq!(r.pit_loss.source, "none");
        assert!(r.notes.iter().any(|n| n.contains("Pit lane loss")));
    }

    #[test]
    fn readings_from_a_pc_that_is_not_driving_are_ignored() {
        let mut e = Engine::new(StrategyConfig::default(), PitLossStore::default());
        e.on_message(&session_info());
        e.on_message(&scoring(0.0, 0, 10.0, false, 0, -1.0, 1.0, 2));
        for lap in 0..4 {
            e.on_message(&telemetry(50.0, 1.0)); // the online placeholder
            e.on_message(&scoring(90.0 * (lap + 1) as f64, lap + 1, 10.0, false, 0, 90.0, 0.9, 2));
        }
        let r = e.report();
        assert!(r.fuel.is_none());
        assert!(e.laps.iter().all(|l| l.fuel.is_none() && l.wear.is_none()));
        assert!(r.notes.iter().any(|n| n.contains("only read on the PC that is driving")));
    }

    /// Not a test: what the lift-and-coast measure finds in a recording, to
    /// check it on real sessions (`--record`). From `bridge`:
    ///
    /// `$env:PWREC = "race.pwrec"; cargo test lico_cost_of_a_recording -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn lico_cost_of_a_recording() {
        let Ok(path) = std::env::var("PWREC") else {
            eprintln!("Set PWREC to a .pwrec file.");
            return;
        };
        let rec = crate::recording::Recording::load(Path::new(&path)).expect("a readable recording");
        let mut e = Engine::new(StrategyConfig::default(), PitLossStore::default());
        let mut last = None;
        for f in &rec.frames {
            if e.on_message(&f.msg).0 {
                // A lap was completed: keep the latest figure, the session
                // may end with laps that do not measure.
                if let Some(c) = e.report().lico_cost {
                    eprintln!(
                        "lap {:>3}: {:.2} s/lap for +{:.1} s coasting, laps {}-{}, {} in traffic",
                        e.laps.last().map(|l| l.lap).unwrap_or(0),
                        c.s_per_lap,
                        c.extra_coast_s,
                        c.first_lap,
                        c.last_lap,
                        c.traffic_zones
                    );
                    last = Some(c);
                }
            }
        }
        eprintln!("{last:#?}");
    }

    /// Laps of 5000 m at 50 m/s, telemetry at 20 Hz. From the fourth lap the
    /// driver coasts from 1000 to 1200 m and is at 42 m/s until 1300 m; on the
    /// fifth another car runs 30 m ahead all lap.
    #[test]
    fn lift_and_coast_cost_comes_from_the_telemetry() {
        let mut e = Engine::new(StrategyConfig::default(), PitLossStore::default());
        e.on_message(&session_info());
        let (mut t, mut fuel, dt) = (0.0, 90.0, 0.05);
        for lap in 0..7i64 {
            let (lico, traffic, start) = (lap >= 3, lap == 4, t);
            let (mut s, mut step) = (0.0, 0);
            while s < 5000.0 {
                let coast = lico && (1000.0..1200.0).contains(&s);
                let speed = if lico && (1000.0..1300.0).contains(&s) { 42.0 } else { 50.0 };
                if step % 4 == 0 {
                    let other = s + if traffic { 30.0 } else { 2500.0 };
                    e.on_message(&v(serde_json::json!({
                        "type": "ScoringUpdate", "session_type": "Race", "session_time": t, "player_vehicle_id": 3,
                        "vehicles": [
                            {"id": 3, "total_laps": lap, "lap_dist": s, "in_pits": false, "pit_state": 0, "control": 0,
                             "last_lap_time": if lap > 0 { 100.0 } else { -1.0 }, "virtual_energy": 0.0,
                             "driver_name": "Alice", "lap_start_et": start},
                            {"id": 4, "lap_dist": other % 5000.0, "in_pits": false}]
                    })));
                }
                e.on_message(&v(serde_json::json!({
                    "type": "TelemetryUpdate", "current_et": t, "lap_start_et": start, "speed_ms": speed,
                    "throttle": if coast { 0.0 } else { 1.0 }, "brake": 0.0, "fuel": fuel, "fuel_capacity": 100.0
                })));
                if !coast {
                    fuel -= 0.001 * speed * dt; // 1 L/km at throttle
                }
                s += speed * dt;
                t += dt;
                step += 1;
            }
        }
        // The first sample of the next lap closes the last one.
        e.on_message(&v(serde_json::json!({
            "type": "TelemetryUpdate", "current_et": t, "lap_start_et": t, "speed_ms": 50.0,
            "throttle": 1.0, "brake": 0.0, "fuel": fuel, "fuel_capacity": 100.0
        })));
        let c = e.report().lico_cost.expect("a cost from the zones");
        // Lap 5 had its only zone in traffic: left out, and counted.
        assert_eq!((c.laps, c.first_lap, c.last_lap, c.traffic_zones), (3, 4, 7, 1));
        assert_eq!(c.ref_laps, 3);
        let expected = 300.0 / 42.0 - 300.0 / 50.0;
        assert!((c.s_per_lap - expected).abs() < 0.03, "{} vs {}", c.s_per_lap, expected);
        assert!((c.fuel_saved_per_lap.unwrap() - 0.2).abs() < 0.01, "{:?}", c.fuel_saved_per_lap);
        assert_eq!(c.corners.len(), 1);
        assert!((c.corners[0].at_m - 1000.0).abs() <= 10.0);
    }

    /// Five laps, then a stop whose lane loss works out at 30 s. Returns
    /// (lap completed, pit loss recorded) for the out-lap, and the stop time.
    fn pit_stop_in(e: &mut Engine, stype: &str) -> ((bool, bool), f64) {
        let sc = |t, laps, dist, in_pits, ps, last, ve| scoring_in(stype, t, laps, dist, in_pits, ps, last, ve, 0);
        e.on_message(&session_info());
        e.on_message(&sc(0.0, 0, 10.0, false, 0, -1.0, 1.0));
        let mut t = 0.0;
        for lap in 0..5 {
            e.on_message(&sc(t + 45.0, lap, 2500.0, false, 0, if lap > 0 { 90.0 } else { -1.0 }, 1.0));
            e.on_message(&sc(t + 90.0, lap + 1, 10.0, false, 0, 90.0, 1.0));
            t += 90.0;
        }
        // Lap 6: into the pits at 4000 m. Stationary 30 s. Lap time 140 s.
        e.on_message(&sc(t + 70.0, 5, 4000.0, true, 2, 90.0, 0.8));
        e.on_message(&sc(t + 80.0, 5, 4500.0, true, 3, 90.0, 0.8));
        e.on_message(&sc(t + 110.0, 5, 4600.0, true, 4, 90.0, 1.0));
        e.on_message(&sc(t + 120.0, 6, 50.0, true, 4, 140.0, 1.0));
        e.on_message(&sc(t + 125.0, 6, 300.0, false, 0, 140.0, 1.0));
        // Out-lap completes in 100 s: (140 + 100) − 2 × 90 − 30 = 30 s.
        (e.on_message(&sc(t + 220.0, 7, 10.0, false, 0, 100.0, 0.96)), t)
    }

    #[test]
    fn a_pit_stop_measures_the_lane_loss() {
        let mut e = Engine::new(StrategyConfig::default(), PitLossStore::default());
        let ((done, new_loss), t) = pit_stop_in(&mut e, "Race");
        assert!(done && new_loss);
        let r = e.report();
        assert_eq!(r.pit_loss.source, "measured");
        assert!((r.pit_loss.lane_loss_s.unwrap() - 30.0).abs() < 1e-6, "{:?}", r.pit_loss);
        assert_eq!(r.stint.started_at, Some(t + 125.0));
        // A manual figure wins.
        e.cfg.pit_lane_loss_s = Some(25.0);
        assert_eq!(e.report().pit_loss.source, "manual");
    }

    #[test]
    fn a_practice_stop_is_not_kept() {
        let mut e = Engine::new(StrategyConfig::default(), PitLossStore::default());
        let ((done, new_loss), _) = pit_stop_in(&mut e, "Practice");
        assert!(done && !new_loss);
        assert_eq!(e.pit_loss_store().get("Test Ring"), None);
        assert_eq!(e.report().pit_loss.source, "none");
    }

    #[test]
    fn coasting_is_measured_from_the_pedals() {
        let mut e = Engine::new(StrategyConfig::default(), PitLossStore::default());
        e.on_message(&session_info());
        let pedals = |throttle: f64, brake: f64, speed: f64| {
            v(serde_json::json!({"type": "TelemetryUpdate", "throttle": throttle, "brake": brake, "speed_ms": speed}))
        };
        let mut t = 0.0;
        e.on_message(&scoring(t, 0, 10.0, false, 0, -1.0, 1.0, 2));
        for lap in 0..3 {
            // 100 samples: 10 coasting at speed, 5 off the pedals but slow
            // (a hairpin), the rest on a pedal.
            for i in 0..100 {
                let m = if i < 10 { pedals(0.0, 0.0, 60.0) } else if i < 15 { pedals(0.0, 0.0, 12.0) } else { pedals(1.0, 0.0, 70.0) };
                e.on_message(&m);
            }
            t += 90.0;
            e.on_message(&scoring(t, lap + 1, 10.0, false, 0, 90.0, 1.0 - 0.04 * (lap + 1) as f64, 2));
        }
        // The first lap was not watched from its start; then 10 % of 90 s.
        // Measured from the engineer seat too (control 2).
        let coast: Vec<Option<f64>> = e.laps.iter().map(|l| l.coast_s).collect();
        assert_eq!(coast[0], None);
        assert!(coast[1..].iter().all(|c| (c.unwrap() - 9.0).abs() < 1e-9), "{coast:?}");
    }

    #[test]
    fn a_target_gives_the_plan_the_team_intends() {
        // 60-minute race at 4 % VE a lap: one stop. At 2 %/lap the 76 %
        // left covers the ~34 laps to go and the reserve, and so do 72 L at 2 L/lap:
        // none. (An energy without a target keeps its measured stops.)
        let cfg = StrategyConfig { ve_target_per_lap: Some(0.02), fuel_target_per_lap: Some(2.0), ..Default::default() };
        let mut e = Engine::new(cfg, PitLossStore::default());
        e.on_message(&session_info());
        e.on_message(&scoring(0.0, 0, 10.0, false, 0, -1.0, 1.0, 0));
        drive(&mut e, 0, 6, 0.0, 90.0, 1.0);
        let r = e.report();
        assert_eq!(r.plan.as_ref().unwrap().stops, 1);
        assert_eq!(r.plan_at_target.as_ref().unwrap().stops, 0);
        // Without a target there is no second plan.
        e.cfg.ve_target_per_lap = None;
        e.cfg.fuel_target_per_lap = None;
        assert!(e.report().plan_at_target.is_none());
    }

    #[test]
    fn a_stint_time_limit_can_force_a_stop() {
        let cfg = StrategyConfig { max_stint_minutes: Some(20.0), ..Default::default() };
        let mut e = Engine::new(cfg, PitLossStore::default());
        e.on_message(&session_info());
        e.on_message(&scoring(0.0, 0, 10.0, false, 0, -1.0, 0.0, 0)); // no VE
        drive(&mut e, 0, 4, 0.0, 300.0, 0.0);
        let r = e.report();
        // 300 L is enough for the race; the 20-minute stint is not.
        assert_eq!(r.fuel.as_ref().unwrap().stops, Some(0));
        let plan = r.plan.unwrap();
        assert_eq!(plan.limiting, "stint time");
        assert!(plan.stops >= 1);
    }

    #[test]
    fn a_new_session_starts_over() {
        let mut e = Engine::new(StrategyConfig::default(), PitLossStore::default());
        e.on_message(&session_info());
        e.on_message(&scoring(0.0, 0, 10.0, false, 0, -1.0, 1.0, 0));
        drive(&mut e, 0, 3, 0.0, 90.0, 1.0);
        assert_eq!(e.laps.len(), 3);
        e.on_message(&scoring(1.0, 0, 10.0, false, 0, -1.0, 1.0, 0));
        assert!(e.laps.is_empty());
    }

    #[test]
    fn pit_loss_store_keeps_a_median_per_track() {
        let mut s = PitLossStore::default();
        assert_eq!(s.get("Monza"), None);
        for x in [30.0, 28.0, 90.0] {
            s.add("Monza", x);
        }
        assert_eq!(s.get("Monza"), Some((30.0, 3)));
        for i in 0..20 {
            s.add("Spa", i as f64);
        }
        assert_eq!(s.get("Spa").unwrap().1, PIT_LOSS_KEEP as u32);
    }

    #[test]
    fn settings_are_clamped() {
        let c = StrategyConfig { reserve_laps: 99.0, tyre_wear_limit: -1.0, max_stint_minutes: Some(0.0), pit_lane_loss_s: Some(-5.0), change_tyres: true, ve_target_per_lap: Some(3.2), fuel_target_per_lap: Some(2.9), ve_target_box_lap: Some(-3), fuel_target_box_lap: Some(170) }.sanitized();
        assert_eq!((c.reserve_laps, c.tyre_wear_limit, c.max_stint_minutes, c.pit_lane_loss_s), (5.0, 0.0, None, None));
        // A VE target is a fraction: 3.2 (meant as %) is out of range; litres are fine.
        assert_eq!((c.ve_target_per_lap, c.fuel_target_per_lap), (None, Some(2.9)));
        assert_eq!((c.ve_target_box_lap, c.fuel_target_box_lap), (None, Some(170)));
    }
}
