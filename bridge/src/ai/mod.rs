//! AI engineer comments.
//!
//! At every lap (or every few laps) and after notable race events, this builds
//! a compact snapshot of the team car — from the strategy engine's plan, the
//! race log and the latest timing, never from the 100 Hz telemetry — and asks
//! Claude for a short comment for the engineer: trends, anomalies, strategy
//! suggestions. The engineer reads it and decides what to pass on by radio.
//!
//! # Numbers
//!
//! The model is told to use only numbers from the snapshot, and [`verify`]
//! checks that it did: every number in a comment must be one of the snapshot's,
//! possibly rounded. A comment with any other number is withheld by default and
//! the dashboard says which numbers failed.
//!
//! # Cost
//!
//! Off by default. When on: one request in flight at a time, a minimum interval
//! between automatic comments, a cap on requests per hour and a daily budget in
//! dollars, computed from the token counts each response reports. The API key
//! comes only from the `ANTHROPIC_API_KEY` environment variable of the process
//! that runs this (on a team, the engineer's hub), and is never sent to clients.

pub mod verify;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rmpv::Value;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

use crate::hub::logic::num;
use crate::protocol::messages::ServerMessage;
use crate::race::RaceEvent;
use crate::recording::{field, message_type};
use crate::strategy::StrategyReport;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
/// Overrides `API_URL` (tests point it at a local mock server).
const BASE_URL_ENV: &str = "LMU_PITWALL_AI_BASE_URL";
const KEY_ENV: &str = "ANTHROPIC_API_KEY";
const API_VERSION: &str = "2026-10-21";
/// Server-side refusal fallback: if the chosen model declines, another one
/// answers in the same call. Recommended default for current models.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
/// Room for the (always-on) thinking plus a short comment.
const MAX_TOKENS: u32 = 4000;
/// Comments kept and shown.
const KEEP_COMMENTS: usize = 30;
/// After an event, wait this long for related events before asking.
const EVENT_DEBOUNCE: Duration = Duration::from_secs(4);
/// A question asked by hand may come this soon after the previous request.
const MANUAL_MIN_INTERVAL: Duration = Duration::from_secs(10);
const STATUS_REFRESH: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Settings and pricing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_model")]
    pub model: String,
    /// "low", "medium", "high" … (`output_config.effort`).
    #[serde(default = "default_effort")]
    pub effort: String,
    /// Comment every N completed laps (0 = only on events and by hand).
    #[serde(default = "default_every_laps")]
    pub every_laps: u32,
    /// Also comment after pit stops, full course yellows, penalties, driver changes.
    #[serde(default = "default_true")]
    pub on_events: bool,
    /// Minimum seconds between automatic comments.
    #[serde(default = "default_min_interval")]
    pub min_interval_s: f64,
    #[serde(default = "default_max_per_hour")]
    pub max_calls_per_hour: u32,
    /// Spending cap per calendar day (local time of the PC), US dollars.
    #[serde(default = "default_budget")]
    pub daily_budget_usd: f64,
    /// Language of the comments.
    #[serde(default = "default_language")]
    pub language: String,
    /// Show comments that failed the number check (flagged) instead of withholding them.
    #[serde(default)]
    pub show_unverified: bool,
}

fn default_model() -> String {
    "claude-opus-5-5".into()
}
fn default_effort() -> String {
    "low".into()
}
fn default_every_laps() -> u32 {
    2
}
fn default_true() -> bool {
    true
}
fn default_min_interval() -> f64 {
    60.0
}
fn default_max_per_hour() -> u32 {
    40
}
fn default_budget() -> f64 {
    3.0
}
fn default_language() -> String {
    "Italian".into()
}

impl Default for AiConfig {
    fn default() -> Self {
        serde_json::from_str("{}").expect("defaults")
    }
}

impl AiConfig {
    pub fn sanitized(mut self) -> Self {
        self.min_interval_s = self.min_interval_s.clamp(15.0, 3600.0);
        self.max_calls_per_hour = self.max_calls_per_hour.clamp(1, 600);
        self.daily_budget_usd = self.daily_budget_usd.clamp(0.0, 1000.0);
        self.every_laps = self.every_laps.min(100);
        if !["low", "medium", "high", "xhigh", "max"].contains(&self.effort.as_str()) {
            self.effort = default_effort();
        }
        self.model = self.model.trim().to_string();
        if self.model.is_empty() || self.model.len() > 64 || !self.model.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') {
            self.model = default_model();
        }
        self.language = self.language.chars().filter(|c| c.is_alphabetic() || *c == ' ').take(30).collect();
        if self.language.trim().is_empty() {
            self.language = default_language();
        }
        self
    }
}

/// US dollars per million tokens: (input, output, cache read).
///
/// From Anthropic's published prices (November 2026). An unknown model is
/// priced as the most expensive one listed here, so the budget errs on the
/// safe side. Cache writes are charged at 1.25 × input, the 5-minute rate.
fn price(model: &str) -> (f64, f64, f64) {
    match model {
        "claude-opus-5-5" => (4.0, 20.0, 0.20),
        "claude-sonnet-5-5" => (2.0, 10.0, 0.10),
        "claude-haiku-5-5" => (0.10, 0.50, 0.10),
        "claude-fable-5-2" => (8.0, 40.0, 0.20),
        _ => (8.0, 40.0, 8.0),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

pub fn cost_usd(model: &str, u: &Usage) -> f64 {
    let (i, o, cr) = price(model);
    (u.input_tokens as f64 * i
        + u.output_tokens as f64 * o
        + u.cache_read_input_tokens as f64 * cr
        + u.cache_creation_input_tokens as f64 * i * 1.25)
        / 1_000_000.0
}

/// Spending so far today, kept across restarts (ai_usage.json).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DayUsage {
    day: String,
    calls: u32,
    cost_usd: f64,
}

/// Days since the Unix epoch, in local time as well as the OS offset allows;
/// only used to reset the daily budget, so UTC is close enough.
fn today() -> String {
    let days = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0);
    format!("day-{days}")
}

fn load<T: serde::de::DeserializeOwned + Default>(dir: &Path, name: &str) -> T {
    std::fs::read(dir.join(name)).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
}

fn save<T: Serialize>(dir: &Path, name: &str, v: &T) {
    let _ = std::fs::create_dir_all(dir);
    let tmp = dir.join(format!("{name}.tmp"));
    if std::fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(tmp, dir.join(name));
    }
}

/// Today's spending, one count for the whole process: on a hub every team's
/// AI draws on the same money. Kept in `ai_usage.json` in the main folder.
fn shared_spend() -> &'static Mutex<DayUsage> {
    static SPEND: OnceLock<Mutex<DayUsage>> = OnceLock::new();
    SPEND.get_or_init(|| Mutex::new(load(&crate::paths::root(), "ai_usage.json")))
}

// ---------------------------------------------------------------------------
// Output messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AiComment {
    pub id: u64,
    /// Session time and laps completed when the snapshot was taken.
    pub session_time: f64,
    pub lap: i64,
    /// Why it was asked: "lap", an event kind, or "question".
    pub trigger: String,
    /// The engineer's question, for comments asked by hand.
    pub question: Option<String>,
    /// The comment; empty when withheld.
    pub text: String,
    /// Numbers in the reply that are not in the snapshot.
    pub unverified: Vec<String>,
    /// Hidden because of `unverified` (the default).
    pub withheld: bool,
    /// The model declined (`stop_reason: refusal`) even after the fallback.
    pub refused: bool,
    /// Model that actually answered (differs from the configured one after a fallback).
    pub model: String,
    pub usage: Usage,
    pub cost_usd: f64,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AiStatus {
    pub enabled: bool,
    /// Whether the process has an API key at all.
    pub key_present: bool,
    pub busy: bool,
    pub calls_today: u32,
    pub cost_today_usd: f64,
    pub calls_last_hour: u32,
    /// Why no comment is coming, when that is the case.
    pub blocked: Option<String>,
    pub last_error: Option<String>,
    pub config: Option<AiConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AiFeed {
    pub status: AiStatus,
    /// Newest last.
    pub comments: Vec<AiComment>,
}

/// From a dashboard: change the settings, or ask now (optionally a question).
#[derive(Debug, Clone)]
pub enum AiCommand {
    Config(AiConfig),
    Ask(Option<String>),
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

fn r1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}
fn lap_str(t: f64) -> Option<String> {
    (t > 0.0).then(|| {
        let m = (t / 60.0).floor();
        format!("{}:{:06.3}", m as i64, t - m * 60.0)
    })
}
fn mmss(s: f64) -> String {
    let s = s.max(0.0).round() as i64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// What the latest messages say, kept for the next snapshot.
#[derive(Default)]
struct World {
    strategy: Option<StrategyReport>,
    scoring: Option<Value>,
    telemetry: Option<Value>,
    session: Option<Value>,
    events: Vec<RaceEvent>,
    /// Events already shown to the model.
    events_seen: usize,
}

const NOTABLE: [&str; 7] = ["fcy", "fcy_end", "pit_out", "penalty", "driver", "impact", "position"];

impl World {
    fn reset(&mut self) {
        *self = World::default();
    }

    fn own_new_notable(&self) -> Option<String> {
        self.events[self.events_seen.min(self.events.len())..]
            .iter()
            .filter(|e| NOTABLE.contains(&e.kind.as_str()) && (e.own || e.car_id.is_none()))
            .map(|e| e.kind.clone())
            .next_back()
    }

    /// The JSON the model sees. Units are in the key names; everything is
    /// rounded to what is worth saying on the radio.
    fn snapshot(&self, trigger: &str, question: Option<&str>) -> serde_json::Value {
        let s = self.strategy.as_ref();
        let mut snap = json!({ "trigger": trigger });
        if let Some(q) = question {
            snap["engineer_question"] = json!(q);
        }

        if let Some(st) = s {
            snap["session"] = json!({
                "track": st.track,
                "type": st.session_type,
                "time_left": st.horizon.as_ref().and_then(|h| h.time_remaining_s).map(mmss),
                "laps_to_go": st.horizon.as_ref().map(|h| r1(h.laps_to_go)),
            });
            snap["car"] = json!({
                "driver": st.driver,
                "laps_completed": st.laps_completed,
                "reference_lap": st.pace.ref_lap_s.and_then(lap_str),
                "stint_time": st.stint.elapsed_s.map(mmss),
                "stint_limit": st.stint.limit_s.map(mmss),
            });
            let energy = |e: &crate::strategy::model::EnergyPlan, pct: bool| {
                let k = if pct { 100.0 } else { 1.0 };
                json!({
                    "now": r1(e.now * k),
                    "use_per_lap": if pct { r2(e.planning_per_lap * k) } else { r2(e.planning_per_lap) },
                    "use_trend_per_lap": e.per_lap.trend_per_lap.map(|t| r3(t * k)),
                    "laps_left": r1(e.laps_left),
                    "needed_to_finish": e.needed_to_finish.map(|v| r1(v * k)),
                    "to_add": e.to_add.map(|v| r1(v * k)),
                    "full_tank_laps": r1(e.stint_laps_max),
                    "use_measured_over": format!("{} laps ({})", e.rate.laps, match e.rate.basis.as_str() {
                        "lico" => "since the driver changed lift-and-coast",
                        "previous stint" => "previous stint",
                        _ => "this stint",
                    }),
                    "use_last_lap": e.rate.last_lap.map(|v| r2(v * k)),
                    "lift_and_coast_s_per_lap": e.rate.coast_s.map(r1),
                    "lift_and_coast_before_change_s_per_lap": e.rate.coast_before_s.map(r1),
                    "use_per_lap_before_change": e.rate.per_lap_before.map(|v| r2(v * k)),
                    "one_stop_fewer": e.save_stop.as_ref().map(|sv| json!({
                        "stops": sv.stops,
                        "max_use_per_lap": r2(sv.per_lap * k),
                        "saving_percent": r1(sv.saving * 100.0),
                        "extra_lift_and_coast_s_per_lap": sv.extra_coast_s.map(r1),
                    })),
                    "engineer_target": e.target.as_ref().map(|t| json!({
                        "box_at_end_of_lap": t.box_lap,
                        "laps_to_that_stop": t.laps_to_box.map(r1),
                        "use_per_lap": r2(t.per_lap * k),
                        "now_over_target_percent": r1(t.off_target * 100.0),
                        "laps_left_at_target": r1(t.laps_left),
                        "full_tank_laps_at_target": r1(t.stint_laps_max),
                        "stops_at_target": t.stops,
                        "extra_lift_and_coast_s_per_lap": t.extra_coast_s.map(r1),
                    })),
                })
            };
            if let Some(f) = &st.fuel {
                snap["fuel_litres"] = energy(f, false);
            }
            if let Some(v) = &st.ve {
                snap["virtual_energy_percent"] = energy(v, true);
            }
            if let Some(p) = &st.plan {
                snap["plan"] = json!({
                    "stops_to_finish": p.stops,
                    "limited_by": p.limiting,
                    "pit_window_laps": p.window.as_ref().map(|w| format!("{}-{}", w.earliest_lap, w.latest_lap)),
                });
            }
            if let Some(p) = &st.plan_at_target {
                snap["plan_at_engineer_target"] = json!({
                    "stops_to_finish": p.stops,
                    "pit_window_laps": p.window.as_ref().map(|w| format!("{}-{}", w.earliest_lap, w.latest_lap)),
                });
            }
            let rows: Vec<serde_json::Value> = st.what_if.iter().take(4).map(|w| json!({
                "box_in_laps": w.in_laps,
                "on_lap": w.pit_lap,
                "reachable": w.reachable,
                "add_fuel_litres": w.fuel_add.map(r1),
                "add_ve_percent": w.ve_add.map(|v| r1(v * 100.0)),
                "stops_after": w.stops_after,
                "stationary_s": w.stationary_s.map(r1),
                "total_stop_cost_s": w.stop_cost_s.map(r1),
            })).collect();
            if !rows.is_empty() {
                snap["box_options"] = json!(rows);
            }
            let t = &st.tyres;
            snap["tyres"] = json!({
                "laps_on_set": t.set_laps,
                "tread_left_percent": t.wear_now.map(|w| w.map(|x| r1(x * 100.0))),
                "tread_lost_per_lap_percent": t.wear_per_lap.map(|w| w.map(|x| r2(x * 100.0))),
                "laps_to_wear_limit": t.laps_to_limit.map(r1),
                "pace_change_s_per_lap": t.pace_trend_s_per_lap.map(r3),
                "new_tyres_cost_s": st.tyre_choice.as_ref().map(|c| r1(c.change_cost_s)),
                "new_tyres_gain_next_stint_s": st.tyre_choice.as_ref().and_then(|c| c.expected_gain_s).map(r1),
                "must_change_next_stop": st.tyre_choice.as_ref().map(|c| c.must_change),
            });
            snap["pit_lane_loss_s"] = json!(st.pit_loss.lane_loss_s.map(r1));
            let w = &st.weather;
            snap["weather"] = json!({
                "raining": w.raining_now,
                "track_water_percent": r1(w.wetness * 100.0),
                "rain_expected_in": w.rain_in_s.map(mmss),
                "rain_expected_in_laps": w.rain_in_laps.map(r1),
                "rain_chance_percent": w.rain_chance.map(|c| (c * 100.0).round()),
                "dry_again_in": w.dry_in_s.map(mmss),
            });
            if !st.notes.is_empty() {
                snap["missing_data"] = json!(st.notes);
            }
        }

        // Class rivals and our lap times, from the latest scoring.
        if let Some(sc) = &self.scoring {
            if let Some(rv) = rivals(sc) {
                snap["class"] = rv;
            }
        }

        // Tyre lock-ups (flat-spot risk), from telemetry.
        if let Some(tel) = &self.telemetry {
            if let Some(tyres) = field(tel, "tires").and_then(|t| t.as_array()) {
                let names = ["FL", "FR", "RL", "RR"];
                let locks: Vec<serde_json::Value> = tyres
                    .iter()
                    .enumerate()
                    .filter_map(|(i, t)| {
                        let n = field(t, "lockups").and_then(num).unwrap_or(0.0);
                        (n > 0.0).then(|| json!({
                            "tyre": names.get(i).copied().unwrap_or("?"),
                            "lockups": n as i64,
                            "longest_s": field(t, "lockup_longest_s").and_then(num).map(r1),
                        }))
                    })
                    .collect();
                if !locks.is_empty() {
                    snap["lockups_this_set"] = json!(locks);
                }
                if tyres.iter().any(|t| field(t, "flat").and_then(|v| v.as_bool()).unwrap_or(false)) {
                    snap["puncture"] = json!(true);
                }
            }
        }

        let recent: Vec<String> = self.events[self.events_seen.min(self.events.len())..]
            .iter()
            .rev()
            .take(8)
            .rev()
            .map(|e| format!("lap {}: {}", e.lap, e.text))
            .collect();
        if !recent.is_empty() {
            snap["events_since_last_comment"] = json!(recent);
        }
        snap
    }
}

/// Our class position, the cars just ahead and behind in class, and the gaps.
fn rivals(sc: &Value) -> Option<serde_json::Value> {
    let pid = field(sc, "player_vehicle_id")?.as_i64()?;
    let rows = field(sc, "vehicles")?.as_array()?;
    let n = |r: &Value, k: &str| field(r, k).and_then(num).unwrap_or(-1.0);
    let me = rows.iter().find(|r| field(r, "id").and_then(|v| v.as_i64()) == Some(pid))?;
    let cls = field(me, "vehicle_class")?.as_str()?.to_string();
    let mut same: Vec<&Value> = rows.iter().filter(|r| field(r, "vehicle_class").and_then(|v| v.as_str()) == Some(cls.as_str())).collect();
    same.sort_by(|a, b| n(a, "position").total_cmp(&n(b, "position")));
    let idx = same.iter().position(|r| field(r, "id").and_then(|v| v.as_i64()) == Some(pid))?;
    let race = field(sc, "session_type").and_then(|v| v.as_str()).unwrap_or("").to_lowercase().contains("race");
    let links: Vec<crate::gaps::Link> = rows
        .iter()
        .map(|r| crate::gaps::Link {
            place: n(r, "position") as i64,
            time_behind_next: n(r, "time_behind_next"),
            laps_behind_next: n(r, "laps_behind_next").max(0.0) as i64,
        })
        .collect();
    let car = |r: &Value| {
        let gap = match crate::gaps::between(&links, n(me, "position") as i64, n(r, "position") as i64) {
            Some(crate::gaps::Gap::Laps(l)) if race => json!(format!("{l} laps")),
            Some(crate::gaps::Gap::Time(t)) if race => json!(r1(t)),
            _ => json!(null),
        };
        json!({
            "driver": field(r, "driver_name").and_then(|v| v.as_str()).unwrap_or(""),
            "gap_s": gap,
            "last_lap": lap_str(n(r, "last_lap_time")),
            "best_lap": lap_str(n(r, "best_lap_time")),
            "in_pits": field(r, "in_pits").and_then(|v| v.as_bool()).unwrap_or(false),
            "pit_stops": n(r, "num_pitstops").max(0.0) as i64,
        })
    };
    Some(json!({
        "class": cls,
        "position": format!("P{}/{}", idx + 1, same.len()),
        "our_last_lap": lap_str(n(me, "last_lap_time")),
        "our_best_lap": lap_str(n(me, "best_lap_time")),
        "ahead": (idx > 0).then(|| car(same[idx - 1])),
        "behind": same.get(idx + 1).map(|r| car(r)),
    }))
}

// ---------------------------------------------------------------------------
// The request
// ---------------------------------------------------------------------------

fn system_prompt(language: &str) -> String {
    format!(
        "You assist the race engineer of an endurance team in the sim racing game Le Mans Ultimate. \
The engineer follows the team car from home and talks to the driver by radio; your comment is for the engineer only.\n\n\
Each message is a JSON snapshot computed by the team's strategy software: plan, energy use, tyres, rivals, weather, and the race events since your last comment. \
Write a short comment in {language}: at most four lines, about 60 words. Say what changed or matters now: trends, anomalies, and concrete strategy suggestions (when to box, tyres, saving energy, a rival to watch). \
If nothing notable happened, say so in one line. If the snapshot has an engineer_question, answer it first.\n\n\
Numbers: use only numbers that appear in the snapshot, copied as written or rounded to fewer decimals. \
Do not add, subtract, multiply, project or estimate anything, and do not convert units: every number you write is checked against the snapshot and the comment is discarded if one is not there. \
When something matters but the snapshot has no number for it, say it in words. Fields listed in missing_data are unknown: do not guess them.\n\n\
Plain text, no markdown headings, no preamble."
    )
}

enum Outcome {
    Ok { text: String, refused: bool, model: String, usage: Usage },
    Err { message: String, disable: bool, retry_after: Option<Duration> },
}

fn call_api(key: &str, cfg: &AiConfig, snapshot: &serde_json::Value, previous: &[String]) -> Outcome {
    let url = std::env::var(BASE_URL_ENV).map(|b| format!("{}/v1/messages", b.trim_end_matches('/'))).unwrap_or_else(|_| API_URL.into());
    let mut user = String::new();
    if !previous.is_empty() {
        user.push_str("Your previous comments, oldest first (do not repeat them):\n");
        for p in previous {
            user.push_str("- ");
            user.push_str(p);
            user.push('\n');
        }
        user.push('\n');
    }
    user.push_str("Snapshot:\n");
    user.push_str(&serde_json::to_string_pretty(snapshot).unwrap_or_default());

    let body = json!({
        "model": cfg.model,
        "max_tokens": MAX_TOKENS,
        "system": [{ "type": "text", "text": system_prompt(&cfg.language), "cache_control": { "type": "ephemeral" } }],
        "output_config": { "effort": cfg.effort },
        "fallbacks": "default",
        "messages": [{ "role": "user", "content": user }],
    });
    let resp = ureq::post(&url)
        .timeout(REQUEST_TIMEOUT)
        .set("content-type", "application/json")
        .set("x-api-key", key)
        .set("anthropic-version", API_VERSION)
        .set("anthropic-beta", FALLBACK_BETA)
        .send_json(body);

    match resp {
        Ok(r) => {
            let v: serde_json::Value = match r.into_json() {
                Ok(v) => v,
                Err(e) => return Outcome::Err { message: format!("unreadable response: {e}"), disable: false, retry_after: None },
            };
            parse_response(&v, &cfg.model)
        }
        Err(ureq::Error::Status(code, r)) => {
            let retry_after = r.header("retry-after").and_then(|s| s.parse::<u64>().ok()).map(Duration::from_secs);
            let detail = r
                .into_json::<serde_json::Value>()
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).map(str::to_string))
                .unwrap_or_default();
            let (message, disable) = match code {
                401 | 403 => (format!("API key rejected ({code}): {detail}"), true),
                400 | 404 => (format!("request rejected ({code}): {detail}"), true),
                429 => (format!("rate limited by the API: {detail}"), false),
                _ => (format!("API error {code}: {detail}"), false),
            };
            Outcome::Err { message, disable, retry_after }
        }
        Err(e) => Outcome::Err { message: format!("network error: {e}"), disable: false, retry_after: None },
    }
}

fn parse_response(v: &serde_json::Value, requested: &str) -> Outcome {
    let u = v.get("usage");
    let g = |k: &str| u.and_then(|u| u.get(k)).and_then(|x| x.as_u64()).unwrap_or(0);
    let usage = Usage {
        input_tokens: g("input_tokens"),
        output_tokens: g("output_tokens"),
        cache_read_input_tokens: g("cache_read_input_tokens"),
        cache_creation_input_tokens: g("cache_creation_input_tokens"),
    };
    let refused = v.get("stop_reason").and_then(|s| s.as_str()) == Some("refusal");
    let text = v
        .get("content")
        .and_then(|c| c.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
                .trim()
                .to_string()
        })
        .unwrap_or_default();
    let model = v.get("model").and_then(|m| m.as_str()).unwrap_or(requested).to_string();
    Outcome::Ok { text, refused, model, usage }
}

// ---------------------------------------------------------------------------
// The task
// ---------------------------------------------------------------------------

struct Ai {
    cfg: AiConfig,
    key: Option<String>,
    world: World,
    comments: VecDeque<AiComment>,
    next_id: u64,
    day: DayUsage,
    recent_calls: VecDeque<Instant>,
    last_auto: Option<Instant>,
    last_any: Option<Instant>,
    busy: bool,
    last_error: Option<String>,
    pause_until: Option<Instant>,
    last_lap_commented: i64,
    pending_event: Option<(String, Instant)>,
    /// A question asked while another request was running: sent next.
    queued_question: Option<Option<String>>,
    /// The session the plan is about: a change starts the AI over.
    session_type: String,
    session_time: f64,
    /// Bumped at each new session. A reply to a request sent in the previous
    /// session is paid for but not shown.
    session_gen: u64,
}

impl Ai {
    fn new(cfg: AiConfig, key: Option<String>, day: DayUsage) -> Self {
        Ai {
            cfg,
            key,
            world: World::default(),
            comments: VecDeque::new(),
            next_id: 1,
            day,
            recent_calls: VecDeque::new(),
            last_auto: None,
            last_any: None,
            busy: false,
            last_error: None,
            pause_until: None,
            last_lap_commented: 0,
            pending_event: None,
            queued_question: None,
            session_type: String::new(),
            session_time: 0.0,
            session_gen: 0,
        }
    }

    fn in_practice(&self) -> bool {
        crate::session_kind::is_practice(&self.session_type)
    }

    fn on_strategy(&mut self, s: StrategyReport) {
        let new_session = !self.session_type.is_empty()
            && (s.session_type != self.session_type || s.session_time < self.session_time - 5.0);
        if new_session {
            self.new_session();
        }
        self.session_type = s.session_type.clone();
        self.session_time = s.session_time;
        if self.last_lap_commented == 0 && s.laps_completed > 0 {
            // First plan seen: count from here, do not comment on laps that
            // went by before.
            self.last_lap_commented = s.laps_completed;
        }
        if s.laps_completed < self.last_lap_commented {
            self.last_lap_commented = s.laps_completed;
        }
        self.world.strategy = Some(s);
    }

    /// Nothing from the previous session (the practice before the race, say)
    /// is shown or sent to the model again.
    fn new_session(&mut self) {
        info!("AI: new session, previous comments cleared");
        self.world.reset();
        self.comments.clear();
        self.last_lap_commented = 0;
        self.pending_event = None;
        self.queued_question = None;
        self.session_gen += 1;
    }

    fn status(&self) -> AiStatus {
        AiStatus {
            enabled: self.cfg.enabled,
            key_present: self.key.is_some(),
            busy: self.busy || self.queued_question.is_some(),
            calls_today: self.day.calls,
            cost_today_usd: self.day.cost_usd,
            calls_last_hour: self.recent_calls.len() as u32,
            blocked: self.blocked().or_else(|| {
                (self.cfg.enabled && self.in_practice())
                    .then(|| "Practice: no automatic comments until qualifying or the race. Ask still works.".to_string())
            }),
            last_error: self.last_error.clone(),
            config: Some(self.cfg.clone()),
        }
    }

    fn feed(&self) -> ServerMessage {
        ServerMessage::AiFeed(Box::new(AiFeed { status: self.status(), comments: self.comments.iter().cloned().collect() }))
    }

    fn roll_day(&mut self) {
        let d = today();
        if self.day.day != d {
            self.day = DayUsage { day: d, calls: 0, cost_usd: 0.0 };
        }
        while self.recent_calls.front().map(|t| t.elapsed() > Duration::from_secs(3600)).unwrap_or(false) {
            self.recent_calls.pop_front();
        }
    }

    /// Why a request cannot go out now, if it cannot.
    fn blocked(&self) -> Option<String> {
        if self.key.is_none() {
            return Some(format!("No API key: set {KEY_ENV} for this Pitwall and restart it."));
        }
        if !self.cfg.enabled {
            return Some("AI comments are off.".into());
        }
        if self.day.cost_usd >= self.cfg.daily_budget_usd {
            return Some(format!("Daily budget of ${:.2} reached.", self.cfg.daily_budget_usd));
        }
        if self.recent_calls.len() as u32 >= self.cfg.max_calls_per_hour {
            return Some(format!("Hourly limit of {} requests reached.", self.cfg.max_calls_per_hour));
        }
        if self.pause_until.map(|t| Instant::now() < t).unwrap_or(false) {
            return Some("Paused after an API rate limit.".into());
        }
        None
    }

    /// The trigger for an automatic comment due now, if any.
    fn due(&mut self) -> Option<String> {
        let st = self.world.strategy.as_ref()?;
        // Nothing useful to say before the first lap: no pace, no consumption.
        if self.busy || self.blocked().is_some() || st.laps_completed < 1 {
            return None;
        }
        // No spending on the practice before the race.
        if crate::session_kind::is_practice(&st.session_type) {
            self.pending_event = None;
            return None;
        }
        let spaced = self.last_auto.map(|t| t.elapsed().as_secs_f64() >= self.cfg.min_interval_s).unwrap_or(true);
        if let Some((kind, at)) = &self.pending_event {
            if at.elapsed() >= EVENT_DEBOUNCE && spaced {
                let k = kind.clone();
                self.pending_event = None;
                return Some(k);
            }
        }
        let every = self.cfg.every_laps as i64;
        if every > 0 && st.laps_completed >= self.last_lap_commented + every && st.laps_completed > 0 && spaced {
            return Some("lap".into());
        }
        None
    }
}

/// Run on a broadcast stream until it closes; send `AiFeed` on the same channel.
/// `dir` holds the AI settings (`ai.json`): the main data folder on a
/// driver's bridge, the team's folder on a hub.
pub async fn run(
    mut rx: broadcast::Receiver<Arc<ServerMessage>>,
    tx: broadcast::Sender<Arc<ServerMessage>>,
    mut commands: mpsc::UnboundedReceiver<AiCommand>,
    dir: PathBuf,
) {
    let key = std::env::var(KEY_ENV).ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    let spent = shared_spend().lock().map(|d| d.clone()).unwrap_or_default();
    let mut ai = Ai::new(load::<AiConfig>(&dir, "ai.json").sanitized(), key, spent);
    info!("AI comments: {} (API key {})", if ai.cfg.enabled { "on" } else { "off" }, if ai.key.is_some() { "present" } else { "absent" });

    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<(String, Option<String>, f64, i64, Instant, Outcome, String, u64)>();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut last_status = Instant::now() - STATUS_REFRESH;

    let send = |ai: &Ai| {
        let _ = tx.send(Arc::new(ai.feed()));
    };

    loop {
        let mut want: Option<(String, Option<String>)> = None;
        tokio::select! {
            r = rx.recv() => match r {
                Ok(msg) => {
                    match &*msg {
                        ServerMessage::StrategyUpdate(s) => ai.on_strategy((**s).clone()),
                        ServerMessage::RaceTiming(t) => {
                            if t.events.len() < ai.world.events_seen {
                                ai.world.events_seen = 0;
                            }
                            ai.world.events = t.events.clone();
                            if ai.cfg.on_events {
                                if let Some(kind) = ai.world.own_new_notable() {
                                    if ai.pending_event.is_none() {
                                        ai.pending_event = Some((kind, Instant::now()));
                                    }
                                }
                            }
                        }
                        ServerMessage::AiFeed(_) => {}
                        other => {
                            let v = match other {
                                ServerMessage::Raw(_)
                                | ServerMessage::ScoringUpdate { .. }
                                | ServerMessage::TelemetryUpdate { .. }
                                | ServerMessage::ConnectionStatus { .. } => crate::recording::to_raw(other),
                                _ => None,
                            };
                            if let Some(v) = v {
                                match message_type(&v) {
                                    Some("ScoringUpdate") => ai.world.scoring = Some(v),
                                    Some("TelemetryUpdate") => ai.world.telemetry = Some(v),
                                    Some("SessionInfo") => ai.world.session = Some(v),
                                    Some("ConnectionStatus") => {
                                        let active = field(&v, "session_active").and_then(|x| x.as_bool()).unwrap_or(true);
                                        if !active && ai.world.strategy.is_some() {
                                            ai.world.reset();
                                            ai.last_lap_commented = 0;
                                            ai.pending_event = None;
                                        }
                                    }
                                    Some("StrategyUpdate") => {
                                        // On a hub, relayed: decode it like our own.
                                        if let Ok(s) = rmpv::ext::from_value::<StrategyReport>(v.clone()) {
                                            ai.on_strategy(s);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            },
            Some(cmd) = commands.recv() => match cmd {
                AiCommand::Config(c) => {
                    ai.cfg = c.sanitized();
                    save(&dir, "ai.json", &ai.cfg);
                    ai.last_error = None;
                    send(&ai);
                }
                AiCommand::Ask(q) => {
                    let q = q.map(|s| s.chars().take(300).collect::<String>()).filter(|s| !s.trim().is_empty());
                    let soon = ai.last_any.map(|t| t.elapsed() < MANUAL_MIN_INTERVAL).unwrap_or(false);
                    if let Some(reason) = ai.blocked() {
                        ai.last_error = Some(reason);
                        send(&ai);
                    } else if ai.world.strategy.is_none() {
                        ai.last_error = Some("No session data yet.".into());
                        send(&ai);
                    } else if ai.busy || soon {
                        // Another request is running or just went out: the
                        // question goes next, it is not dropped.
                        ai.queued_question = Some(q);
                        send(&ai);
                    } else {
                        want = Some(("question".into(), q));
                    }
                }
            },
            Some((trigger, question, st, lap, started, outcome, snapshot_text, gen)) = done_rx.recv() => {
                ai.busy = false;
                ai.roll_day();
                match outcome {
                    Outcome::Ok { text, refused, model, usage } => {
                        let cost = cost_usd(&model, &usage);
                        if let Ok(mut shared) = shared_spend().lock() {
                            if shared.day != ai.day.day {
                                *shared = ai.day.clone();
                            }
                            shared.calls += 1;
                            shared.cost_usd += cost;
                            save(&crate::paths::root(), "ai_usage.json", &*shared);
                            ai.day = shared.clone();
                        }
                        let unverified = if refused { Vec::new() } else { verify::unverified(&text, &snapshot_text) };
                        let withheld = !unverified.is_empty() && !ai.cfg.show_unverified;
                        if !unverified.is_empty() {
                            warn!("AI comment has numbers not in the snapshot: {:?}", unverified);
                        }
                        let c = AiComment {
                            id: ai.next_id,
                            session_time: st,
                            lap,
                            trigger,
                            question,
                            text: if withheld || refused { String::new() } else { text },
                            unverified,
                            withheld,
                            refused,
                            model,
                            usage,
                            cost_usd: cost,
                            latency_ms: started.elapsed().as_millis() as u64,
                        };
                        ai.next_id += 1;
                        if gen == ai.session_gen {
                            ai.comments.push_back(c);
                        }
                        while ai.comments.len() > KEEP_COMMENTS {
                            ai.comments.pop_front();
                        }
                        ai.last_error = None;
                    }
                    Outcome::Err { message, disable, retry_after } => {
                        warn!("AI request failed: {}", message);
                        if disable {
                            ai.cfg.enabled = false;
                            save(&dir, "ai.json", &ai.cfg);
                        }
                        if let Some(d) = retry_after {
                            ai.pause_until = Some(Instant::now() + d);
                        }
                        ai.last_error = Some(message);
                    }
                }
                send(&ai);
            }
            _ = tick.tick() => {
                // What the other teams spent counts against the same budget.
                if let Ok(shared) = shared_spend().lock() {
                    if shared.day == today() {
                        ai.day = shared.clone();
                    }
                }
                ai.roll_day();
                let ready = !ai.busy && ai.last_any.map(|t| t.elapsed() >= MANUAL_MIN_INTERVAL).unwrap_or(true);
                if ai.queued_question.is_some() && ready && ai.blocked().is_none() {
                    want = Some(("question".into(), ai.queued_question.take().flatten()));
                } else if let Some(trigger) = ai.due() {
                    ai.last_auto = Some(Instant::now());
                    want = Some((trigger, None));
                } else if last_status.elapsed() >= STATUS_REFRESH {
                    last_status = Instant::now();
                    send(&ai);
                }
            }
        }

        if let Some((trigger, question)) = want {
            let Some(key) = ai.key.clone() else { continue };
            let snapshot = ai.world.snapshot(&trigger, question.as_deref());
            let snapshot_text = snapshot.to_string();
            let mut previous: Vec<String> = ai.comments.iter().rev().filter(|c| !c.text.is_empty()).take(2).map(|c| c.text.clone()).collect();
            previous.reverse();
            let (st, lap) = ai.world.strategy.as_ref().map(|s| (s.session_time, s.laps_completed)).unwrap_or((0.0, 0));
            ai.last_lap_commented = lap;
            ai.world.events_seen = ai.world.events.len();
            ai.busy = true;
            ai.last_any = Some(Instant::now());
            ai.recent_calls.push_back(Instant::now());
            let cfg = ai.cfg.clone();
            let done = done_tx.clone();
            let gen = ai.session_gen;
            send(&ai);
            info!("AI: asking for a comment ({})", trigger);
            tokio::task::spawn_blocking(move || {
                let started = Instant::now();
                let outcome = call_api(&key, &cfg, &snapshot, &previous);
                let _ = done.send((trigger, question, st, lap, started, outcome, snapshot_text, gen));
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn practice_is_silent_and_the_race_starts_clean() {
        let cfg = AiConfig { enabled: true, ..Default::default() };
        let mut ai = Ai::new(cfg, Some("test".into()), DayUsage::default());
        let plan = |stype: &str, t: f64, laps: i64| StrategyReport {
            session_type: stype.into(), session_time: t, laps_completed: laps, ..Default::default()
        };
        ai.on_strategy(plan("Practice", 100.0, 1));
        ai.on_strategy(plan("Practice", 900.0, 9));
        assert_eq!(ai.due(), None);
        assert!(ai.status().blocked.unwrap().starts_with("Practice"));
        ai.comments.push_back(AiComment { id: 1, text: "from practice".into(), ..Default::default() });
        // The race: nothing from practice is left, and comments come again.
        ai.on_strategy(plan("Race", 50.0, 0));
        assert!(ai.comments.is_empty());
        assert_eq!(ai.session_gen, 1);
        assert_eq!(ai.status().blocked, None);
        ai.on_strategy(plan("Race", 200.0, 1));
        ai.on_strategy(plan("Race", 400.0, 3));
        assert_eq!(ai.due().as_deref(), Some("lap"));
    }

    #[test]
    fn defaults_are_off_and_cheap() {
        let c = AiConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.model, "claude-opus-5-5");
        assert_eq!(c.effort, "low");
        assert_eq!(c.daily_budget_usd, 3.0);
        assert_eq!(c.every_laps, 2);
        assert!(!c.show_unverified);
    }

    #[test]
    fn settings_are_clamped() {
        let c = AiConfig {
            min_interval_s: 1.0,
            max_calls_per_hour: 0,
            effort: "turbo".into(),
            model: "claude; rm -rf".into(),
            language: "".into(),
            daily_budget_usd: -3.0,
            ..AiConfig::default()
        }
        .sanitized();
        assert_eq!((c.min_interval_s, c.max_calls_per_hour, c.daily_budget_usd), (15.0, 1, 0.0));
        assert_eq!((c.effort.as_str(), c.model.as_str(), c.language.as_str()), ("low", "claude-opus-5-5", "Italian"));
    }

    #[test]
    fn cost_from_usage() {
        let u = Usage { input_tokens: 2000, output_tokens: 500, cache_read_input_tokens: 1000, cache_creation_input_tokens: 0 };
        // 2000 × 4 + 500 × 20 + 1000 × 0.2 = 18 200 per million.
        assert!((cost_usd("claude-opus-5-5", &u) - 0.0182).abs() < 1e-9);
        // Unknown model: priced high.
        assert!(cost_usd("some-new-model", &u) > cost_usd("claude-opus-5-5", &u));
    }

    #[test]
    fn a_response_is_parsed() {
        let v = serde_json::json!({
            "model": "claude-opus-5-5", "stop_reason": "end_turn",
            "content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": " Box at lap 30. "}],
            "usage": {"input_tokens": 1500, "output_tokens": 120, "cache_read_input_tokens": 900}
        });
        match parse_response(&v, "x") {
            Outcome::Ok { text, refused, model, usage } => {
                assert_eq!(text, "Box at lap 30.");
                assert!(!refused);
                assert_eq!(model, "claude-opus-5-5");
                assert_eq!((usage.input_tokens, usage.output_tokens, usage.cache_read_input_tokens), (1500, 120, 900));
            }
            Outcome::Err { .. } => panic!("parsed as error"),
        }
        let refusal = serde_json::json!({"model": "m", "stop_reason": "refusal", "content": [], "usage": {}});
        assert!(matches!(parse_response(&refusal, "x"), Outcome::Ok { refused: true, .. }));
    }

    #[test]
    fn the_snapshot_holds_rounded_figures_and_events() {
        let mut w = World::default();
        let mut s = StrategyReport { driver: "Alice".into(), laps_completed: 12, ..Default::default() };
        s.horizon = Some(crate::strategy::model::Horizon { laps_to_go: 23.456, basis: "time".into(), time_remaining_s: Some(2045.0) });
        w.strategy = Some(s);
        w.events = vec![RaceEvent { t: 1.0, lap: 11, kind: "pit_out".into(), severity: "info".into(), car_id: Some(1), own: true, text: "Our car left the pits".into() }];
        let snap = w.snapshot("lap", Some("box now?"));
        assert_eq!(snap["session"]["laps_to_go"], 23.5);
        assert_eq!(snap["session"]["time_left"], "34:05");
        assert_eq!(snap["engineer_question"], "box now?");
        assert_eq!(snap["events_since_last_comment"][0], "lap 11: Our car left the pits");
        w.events_seen = 1;
        assert!(w.world_has_no_new_events());
    }

    impl World {
        fn world_has_no_new_events(&self) -> bool {
            self.snapshot("lap", None).get("events_since_last_comment").is_none()
        }
    }
}
