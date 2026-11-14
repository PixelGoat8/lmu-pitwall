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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
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

/// How often the plan is re-sent between laps (time remaining moves on).
const REFRESH: Duration = Duration::from_secs(2);
/// A telemetry reading older than this is not used for a lap sample.
const TELEMETRY_MAX_AGE: Duration = Duration::from_secs(3);
/// Plausible pit lane loss (seconds): outside this a measurement is discarded.
const PIT_LOSS_RANGE: (f64, f64) = (3.0, 120.0);
/// Pit lane losses kept per track.
const PIT_LOSS_KEEP: usize = 10;

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
        self
    }
}

fn data_dir() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")).unwrap_or_else(|_| PathBuf::from("."))
        })
        .join("LMUPitwall")
}

fn load_json<T: serde::de::DeserializeOwned + Default>(name: &str) -> T {
    std::fs::read(data_dir().join(name)).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
}

fn save_json<T: Serialize>(name: &str, value: &T) {
    let dir = data_dir();
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(value).unwrap_or_default())?;
        std::fs::rename(tmp, dir.join(name))
    };
    if let Err(e) = write() {
        warn!("Could not save {}: {}", name, e);
    }
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
    pub what_if: Vec<WhatIf>,
    pub tyres: TyrePlan,
    pub tyre_choice: Option<TyreChoice>,
    pub stint: StintInfo,
    pub pit_loss: PitLossInfo,
    pub weather: WeatherOutlook,
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
    pit_loss: PitLossStore,
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
    stint_start: Option<f64>,
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
        let Some(r) = field(msg, "vehicles")
            .and_then(|v| v.as_array())
            .and_then(|vs| vs.iter().find(|v| field(v, "id").and_then(|i| i.as_i64()) == Some(pid)))
        else {
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
        };
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
            self.laps.push(LapSample {
                lap: row.total_laps,
                session_time: t,
                lap_time: row.last_lap_time,
                fuel: if trusted { self.fuel } else { None },
                ve: (row.ve > 0.0).then_some(row.ve),
                wear: if trusted { self.wear } else { None },
                pit: self.lap_pit,
                caution: self.lap_caution,
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
        info!("Strategy: pit lane loss at {} measured {:.1} s (stationary {:.1} s)", self.track, lane, p.stationary_s);
        self.pit_loss.add(&self.track, lane);
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
        let fuel_now = self.fuel.or_else(|| self.laps.iter().rev().find_map(|l| l.fuel));
        let fuel_cap = self.pit_info.as_ref().and_then(|p| p.fuel_capacity).or(self.fuel_capacity);
        let fuel = match (fuel_per_lap, fuel_now, fuel_cap) {
            (Some(st), Some(now), Some(cap)) => energy_plan("L", now, cap, st, self.cfg.reserve_laps, to_go),
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
            let plan = st.and_then(|st| energy_plan("%", row.ve, 1.0, st, self.cfg.reserve_laps, to_go));
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
        let mut limits: Vec<(&str, f64, f64)> = Vec::new(); // (name, range from here, stint max)
        if let Some(f) = &fuel {
            limits.push(("fuel", f.laps_left, f.stint_laps_max));
        }
        if let Some(v) = &ve {
            limits.push(("ve", v.laps_left, v.stint_laps_max));
        }
        if let (Some(lim), Some(lap)) = (limit_s, ref_lap) {
            let left = (lim - elapsed.unwrap_or(0.0)).max(0.0) / lap;
            limits.push(("stint time", left, lim / lap));
        }
        let mut energy_stops = fuel.as_ref().and_then(|f| f.stops).unwrap_or(0).max(ve.as_ref().and_then(|v| v.stops).unwrap_or(0));
        let plan = (!limits.is_empty()).then(|| {
            let range = limits.iter().map(|l| l.1).fold(f64::INFINITY, f64::min);
            let (limiting, _, stint_max) = limits.iter().copied().min_by(|a, b| a.2.total_cmp(&b.2)).unwrap();
            // The stint limit can force stops the tank does not.
            if let Some(go) = to_go {
                if go > range {
                    let more = ((go - range) / stint_max.max(1.0)).ceil() as u32;
                    energy_stops = energy_stops.max(more.max(1));
                }
            }
            let window = to_go.and_then(|go| pit_window(go, p, current_lap, range, stint_max, energy_stops));
            PlanSummary {
                stops: window.as_ref().map(|w| w.stops).unwrap_or(energy_stops),
                limiting: limiting.to_string(),
                stint_laps_max: stint_max,
                range_laps: range,
                window,
            }
        });
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
pub async fn run(
    mut rx: broadcast::Receiver<Arc<ServerMessage>>,
    tx: broadcast::Sender<Arc<ServerMessage>>,
    mut config_rx: mpsc::UnboundedReceiver<StrategyConfig>,
) {
    let mut engine = Engine::new(load_json::<StrategyConfig>("strategy.json").sanitized(), load_json("pit_loss.json"));
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
                        ServerMessage::StrategyUpdate(_) => continue,
                        ServerMessage::Raw(v) => v.clone(),
                        other => match rmpv::ext::to_value(other) {
                            Ok(v) => v,
                            Err(_) => continue,
                        },
                    };
                    let (lap_done, new_loss) = engine.on_message(&value);
                    if new_loss {
                        save_json("pit_loss.json", engine.pit_loss_store());
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
                save_json("strategy.json", &engine.cfg);
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
        v(serde_json::json!({
            "type": "ScoringUpdate", "session_type": "Race", "session_time": t, "player_vehicle_id": 3,
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

    #[test]
    fn a_pit_stop_measures_the_lane_loss() {
        let mut e = Engine::new(StrategyConfig::default(), PitLossStore::default());
        e.on_message(&session_info());
        e.on_message(&scoring(0.0, 0, 10.0, false, 0, -1.0, 1.0, 0));
        let t = drive(&mut e, 0, 5, 0.0, 90.0, 1.0);
        // Lap 6: into the pits at 4000 m. Stationary 30 s. Lap time 120 s.
        e.on_message(&scoring(t + 70.0, 5, 4000.0, true, 2, 90.0, 0.8, 0));
        e.on_message(&scoring(t + 80.0, 5, 4500.0, true, 3, 90.0, 0.8, 0));
        e.on_message(&scoring(t + 110.0, 5, 4600.0, true, 4, 90.0, 1.0, 0));
        e.on_message(&scoring(t + 120.0, 6, 50.0, true, 4, 140.0, 1.0, 0));
        e.on_message(&scoring(t + 125.0, 6, 300.0, false, 0, 140.0, 1.0, 0));
        // Out-lap completes in 100 s: (140 + 100) − 2 × 90 − 30 = 30 s.
        e.on_message(&telemetry(88.0, 1.0));
        let (done, new_loss) = e.on_message(&scoring(t + 220.0, 7, 10.0, false, 0, 100.0, 0.96, 0));
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
        let c = StrategyConfig { reserve_laps: 99.0, tyre_wear_limit: -1.0, max_stint_minutes: Some(0.0), pit_lane_loss_s: Some(-5.0), change_tyres: true }.sanitized();
        assert_eq!((c.reserve_laps, c.tyre_wear_limit, c.max_stint_minutes, c.pit_lane_loss_s), (5.0, 0.0, None, None));
    }
}
