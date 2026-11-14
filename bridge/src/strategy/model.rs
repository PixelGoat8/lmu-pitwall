//! The strategy maths, as pure functions over per-lap samples.
//!
//! Nothing here reads the game or a clock: inputs are the laps completed so
//! far and the state "now", outputs are numbers the dashboard shows as they
//! are. Every figure is computed from measured laps or from the game's own
//! data (pit times, forecast); where the data is missing the result is `None`
//! rather than a guess. That matters doubly because these numbers are what the
//! AI layer will later comment on, and it must not have anything invented to
//! repeat.

use serde::{Deserialize, Serialize};

/// Consumption and pace statistics use at most this many recent laps.
pub const STATS_WINDOW: usize = 10;
/// A trend needs at least this many samples to mean anything.
const MIN_TREND_SAMPLES: usize = 4;
/// Laps slower than this multiple of the median are not "clean".
const CLEAN_LAP_FACTOR: f64 = 1.07;
/// Rows in the "pit in N laps" table.
const MAX_WHATIF_ROWS: usize = 8;

// ---------------------------------------------------------------------------
// Input: one completed lap
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct LapSample {
    /// Laps completed after this one (`mTotalLaps` at the line).
    pub lap: i64,
    pub session_time: f64,
    /// Lap time in seconds; `<= 0` when the game has none.
    pub lap_time: f64,
    /// Litres at the line. `None` unless read on the PC that was driving.
    pub fuel: Option<f64>,
    /// Virtual energy fraction at the line; `None` for cars without it.
    pub ve: Option<f64>,
    /// Tread remaining per tyre (1.0 = new). Same trust rule as fuel.
    pub wear: Option<[f64; 4]>,
    /// The lap went through the pit lane (in-lap or out-lap).
    pub pit: bool,
    /// Full-course yellow or safety car at some point during the lap.
    pub caution: bool,
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stat {
    /// Samples used (most recent `STATS_WINDOW` at most).
    pub n: u32,
    pub mean: f64,
    pub median: f64,
    pub last: f64,
    pub min: f64,
    pub max: f64,
    /// Least-squares slope per lap over the samples used; `None` with fewer
    /// than four. Positive = rising.
    pub trend_per_lap: Option<f64>,
}

impl Stat {
    /// Statistics over `(x, y)` points, x being the lap number. `None` if empty.
    pub fn of(points: &[(f64, f64)]) -> Option<Stat> {
        let pts = &points[points.len().saturating_sub(STATS_WINDOW)..];
        if pts.is_empty() {
            return None;
        }
        let ys: Vec<f64> = pts.iter().map(|p| p.1).collect();
        let n = ys.len();
        let mean = ys.iter().sum::<f64>() / n as f64;
        let mut sorted = ys.clone();
        sorted.sort_by(f64::total_cmp);
        let median = if n % 2 == 1 { sorted[n / 2] } else { (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0 };
        Some(Stat {
            n: n as u32,
            mean,
            median,
            last: ys[n - 1],
            min: sorted[0],
            max: sorted[n - 1],
            trend_per_lap: (n >= MIN_TREND_SAMPLES).then(|| slope(pts)).flatten(),
        })
    }
}

/// Least-squares slope of y over x. `None` if all x are equal.
pub fn slope(points: &[(f64, f64)]) -> Option<f64> {
    let n = points.len() as f64;
    if n < 2.0 {
        return None;
    }
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
    if sxx <= f64::EPSILON {
        return None;
    }
    let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    Some(sxy / sxx)
}

/// Per-lap consumption of one quantity (fuel or VE), from consecutive laps.
///
/// Laps through the pit lane are left out (a refuel makes the "drop"
/// negative, and the pit lane is driven slowly), and so are laps under
/// caution, which consume far less than racing laps and would make the plan
/// optimistic.
///
/// Each sample is the average over the run of green laps it belongs to, up
/// to that lap: (level at the start of the run − level now) / laps. Reading
/// the level at the line is never exactly on the line — the last reading can
/// be a few hundredths of a second either side — and a lap-by-lap difference
/// turns that jitter into noise of the same size as the signal on a short
/// lap. Over a run the jitter is divided by the number of laps.
pub fn consumption(laps: &[LapSample], pick: impl Fn(&LapSample) -> Option<f64>) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut run_start: Option<(i64, f64)> = None;
    let mut prev: Option<&LapSample> = None;
    for l in laps {
        let level = pick(l);
        let continues = prev.map(|p| l.lap == p.lap + 1).unwrap_or(false) && !l.pit && !l.caution;
        match (continues, run_start, level) {
            (true, Some((lap0, level0)), Some(level)) => {
                let used = (level0 - level) / (l.lap - lap0) as f64;
                if used > 0.0 {
                    out.push((l.lap as f64, used));
                } else {
                    // A level that went up without a pit stop: start over.
                    run_start = Some((l.lap, level));
                }
            }
            _ => run_start = level.map(|v| (l.lap, v)),
        }
        // A pit or caution lap ends the run; the next green lap starts from it.
        if l.pit || l.caution {
            run_start = level.map(|v| (l.lap, v));
        }
        prev = Some(l);
    }
    out
}

/// Lap times worth judging pace on: not through the pits, not under caution,
/// not the first lap of the session, and within 107 % of the median.
pub fn clean_laps(laps: &[LapSample]) -> Vec<(f64, f64)> {
    let candidates: Vec<(f64, f64)> = laps
        .iter()
        .filter(|l| l.lap_time > 0.0 && !l.pit && !l.caution && l.lap > 1)
        .map(|l| (l.lap as f64, l.lap_time))
        .collect();
    let Some(stat) = Stat::of(&candidates) else { return Vec::new() };
    let cutoff = stat.median * CLEAN_LAP_FACTOR;
    candidates.into_iter().filter(|p| p.1 <= cutoff).collect()
}

// ---------------------------------------------------------------------------
// How much race is left
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Horizon {
    /// Distance still to drive, in laps, from where the car is now.
    pub laps_to_go: f64,
    /// "time" for a timed race, "laps" for a lap-limited one.
    pub basis: String,
    pub time_remaining_s: Option<f64>,
}

/// Laps still to drive, counted from the car's position on the current lap.
///
/// Timed race: the current lap, then whole laps until the clock runs out,
/// then the lap that is running when it does (LMU, like rF2, ends a timed
/// race at the line after time expires). It uses the car's own pace, so for a
/// car several laps down on the leader it can be one lap high: the safe side
/// for fuel.
pub fn horizon(
    time_remaining_s: Option<f64>,
    laps_remaining: Option<f64>,
    lap_progress: f64,
    lap_time: Option<f64>,
) -> Option<Horizon> {
    let p = lap_progress.clamp(0.0, 0.999);
    if let Some(left) = laps_remaining.filter(|l| *l > 0.0) {
        return Some(Horizon { laps_to_go: (left - p).max(0.0), basis: "laps".into(), time_remaining_s });
    }
    let t = time_remaining_s.filter(|t| *t >= 0.0)?;
    let l = lap_time.filter(|l| *l > 0.0)?;
    let this_lap = (1.0 - p) * l;
    let laps_to_go = if t <= this_lap { 1.0 - p } else { (1.0 - p) + ((t - this_lap) / l).ceil() };
    Some(Horizon { laps_to_go, basis: "time".into(), time_remaining_s: Some(t) })
}

// ---------------------------------------------------------------------------
// Energy plan (fuel or VE)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnergyPlan {
    /// "L" for fuel, "%" for virtual energy (values are fractions 0–1).
    pub unit: String,
    pub now: f64,
    pub capacity: f64,
    pub per_lap: Stat,
    /// The figure used for planning: the median, robust to one odd lap.
    pub planning_per_lap: f64,
    /// Kept in the tank at all times, in the same unit.
    pub reserve: f64,
    /// Laps until only the reserve is left.
    pub laps_left: f64,
    pub needed_to_finish: Option<f64>,
    pub to_add: Option<f64>,
    /// Longest stint a full tank allows, in laps.
    pub stint_laps_max: f64,
    pub stops: Option<u32>,
}

pub fn energy_plan(
    unit: &str,
    now: f64,
    capacity: f64,
    per_lap: Stat,
    reserve_laps: f64,
    laps_to_go: Option<f64>,
) -> Option<EnergyPlan> {
    let use_per_lap = per_lap.median;
    if use_per_lap <= 0.0 || capacity <= 0.0 {
        return None;
    }
    let reserve = reserve_laps.max(0.0) * use_per_lap;
    let usable = (capacity - reserve).max(use_per_lap);
    let needed = laps_to_go.map(|l| l * use_per_lap + reserve);
    let to_add = needed.map(|n| (n - now).max(0.0));
    Some(EnergyPlan {
        unit: unit.into(),
        now,
        capacity,
        planning_per_lap: use_per_lap,
        reserve,
        laps_left: ((now - reserve) / use_per_lap).max(0.0),
        needed_to_finish: needed,
        to_add,
        stint_laps_max: usable / use_per_lap,
        stops: to_add.map(|a| if a <= 0.0 { 0 } else { (a / usable).ceil() as u32 }),
        per_lap,
    })
}

// ---------------------------------------------------------------------------
// Pit window
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PitWindow {
    /// Laps still to complete before boxing: 0 = box at the end of this lap.
    pub earliest_in: i64,
    pub latest_in: i64,
    /// The same as lap numbers: box at the end of lap N.
    pub earliest_lap: i64,
    pub latest_lap: i64,
    /// Stops needed from here to the finish, this one included.
    pub stops: u32,
}

/// Where the next stop can go with the fewest stops to the finish.
///
/// `range_laps` is how far the car can go from here (the tightest of fuel,
/// VE and the stint time limit), `stint_laps` the longest stint after a stop.
/// The latest stop is the last lap the tank allows; the earliest is the first
/// lap from which `stops` full stints still cover the rest of the race. If the
/// two cross, one more stop is needed and the window is recomputed for it.
pub fn pit_window(
    laps_to_go: f64,
    lap_progress: f64,
    current_lap: i64,
    range_laps: f64,
    stint_laps: f64,
    stops: u32,
) -> Option<PitWindow> {
    if stops == 0 || stint_laps <= 0.0 {
        return None;
    }
    let rest_of_lap = 1.0 - lap_progress.clamp(0.0, 0.999);
    let latest_in = (range_laps - rest_of_lap).floor() as i64;
    if latest_in < 0 {
        // Not enough to finish this lap: box now, the window is "this lap".
        return Some(PitWindow { earliest_in: 0, latest_in: 0, earliest_lap: current_lap, latest_lap: current_lap, stops });
    }
    let stint_laps = stint_laps.floor().max(1.0);
    let mut k = stops;
    // A handful of iterations at most: each extra stop adds a full stint.
    for _ in 0..8 {
        let after = laps_to_go - rest_of_lap - k as f64 * stint_laps;
        let earliest_in = after.ceil().max(0.0) as i64;
        if earliest_in <= latest_in {
            return Some(PitWindow {
                earliest_in,
                latest_in,
                earliest_lap: current_lap + earliest_in,
                latest_lap: current_lap + latest_in,
                stops: k,
            });
        }
        k += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Pit stop time model
// ---------------------------------------------------------------------------

/// The game's own pit work times (`/rest/garage/UIScreen/RepairAndRefuel`,
/// `pitStopTimes.times`), as far as the plan uses them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PitTimes {
    /// Litres per second.
    pub fuel_fill_rate: f64,
    pub fuel_insert_s: f64,
    pub fuel_concurrent: bool,
    /// Fraction of a full VE tank per second.
    pub ve_fill_rate: f64,
    pub ve_insert_s: f64,
    pub ve_concurrent: bool,
    pub four_tyres_s: f64,
    pub two_tyres_s: f64,
    pub tyres_concurrent: bool,
    pub driver_change_s: f64,
    pub driver_concurrent: bool,
}

/// Stationary time for one stop, from the game's work times.
///
/// Reading of the game's `*Concurrent` flags (an interpretation, not
/// documented): work flagged concurrent overlaps with other concurrent work,
/// the rest is done one after the other. So the stop lasts the sum of the
/// sequential items plus the longest concurrent one. The game's own estimate
/// for the menu currently set is shown next to this, so the two can be
/// compared at every stop.
pub fn stationary_time(t: &PitTimes, fuel_add: f64, ve_add: f64, tyres: u8, driver_change: bool) -> f64 {
    let fuel = if fuel_add > 0.0 && t.fuel_fill_rate > 0.0 { t.fuel_insert_s + fuel_add / t.fuel_fill_rate } else { 0.0 };
    let ve = if ve_add > 0.0 && t.ve_fill_rate > 0.0 { t.ve_insert_s + ve_add / t.ve_fill_rate } else { 0.0 };
    let tyre = match tyres {
        4 => t.four_tyres_s,
        2 => t.two_tyres_s,
        _ => 0.0,
    };
    let driver = if driver_change { t.driver_change_s } else { 0.0 };
    let mut sequential = 0.0;
    let mut parallel: f64 = 0.0;
    for (secs, concurrent) in [(fuel, t.fuel_concurrent), (ve, t.ve_concurrent), (tyre, t.tyres_concurrent), (driver, t.driver_concurrent)] {
        if concurrent {
            parallel = parallel.max(secs);
        } else {
            sequential += secs;
        }
    }
    sequential + parallel
}

// ---------------------------------------------------------------------------
// What if: box in N laps
// ---------------------------------------------------------------------------

/// What one quantity looks like for a stop N laps from now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tank {
    pub now: f64,
    pub capacity: f64,
    pub per_lap: f64,
    pub reserve: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhatIf {
    pub in_laps: i64,
    pub pit_lap: i64,
    /// The car reaches the pits on what is in the tank.
    pub reachable: bool,
    pub fuel_add: Option<f64>,
    pub ve_add: Option<f64>,
    /// Further stops still needed after this one.
    pub stops_after: u32,
    /// Stationary time from the game's pit times; `None` without them.
    pub stationary_s: Option<f64>,
    /// Stationary time plus the pit lane loss, when the loss is known.
    pub stop_cost_s: Option<f64>,
}

/// How much to add at a stop `distance_to_stop` laps away so that the car
/// either finishes or leaves full, and how many stops are still needed then.
fn at_stop(t: &Tank, distance_to_stop: f64, distance_after: f64) -> (f64, u32, bool) {
    let level = t.now - distance_to_stop * t.per_lap;
    let reachable = level >= -1e-9;
    let level = level.max(0.0);
    let need_after = distance_after.max(0.0) * t.per_lap + t.reserve;
    let add = (need_after - level).clamp(0.0, (t.capacity - level).max(0.0));
    let usable = (t.capacity - t.reserve).max(t.per_lap);
    let short = need_after - (level + add);
    let stops_after = if short <= 1e-9 { 0 } else { (short / usable).ceil() as u32 };
    (add, stops_after, reachable)
}

/// The "box in 0, 1, 2 … laps" table: every lap of the window, and "box now"
/// even when it is before the window (it then shows the extra stop it costs).
#[allow(clippy::too_many_arguments)]
pub fn what_if(
    window: &PitWindow,
    current_lap: i64,
    lap_progress: f64,
    laps_to_go: f64,
    fuel: Option<Tank>,
    ve: Option<Tank>,
    pit_times: Option<&PitTimes>,
    tyres: u8,
    lane_loss_s: Option<f64>,
) -> Vec<WhatIf> {
    let rest = 1.0 - lap_progress.clamp(0.0, 0.999);
    let mut ns: Vec<i64> = vec![0];
    ns.extend((window.earliest_in.max(1)..=window.latest_in).take(MAX_WHATIF_ROWS - 1));
    ns.dedup();
    ns.into_iter()
        .map(|n| {
            let to_stop = rest + n as f64;
            let after = laps_to_go - to_stop;
            let f = fuel.map(|t| at_stop(&t, to_stop, after));
            let v = ve.map(|t| at_stop(&t, to_stop, after));
            let stops_after = f.map(|x| x.1).unwrap_or(0).max(v.map(|x| x.1).unwrap_or(0));
            let reachable = f.map(|x| x.2).unwrap_or(true) && v.map(|x| x.2).unwrap_or(true);
            let stationary = pit_times.map(|pt| {
                stationary_time(pt, f.map(|x| x.0).unwrap_or(0.0), v.map(|x| x.0).unwrap_or(0.0), tyres, false)
            });
            WhatIf {
                in_laps: n,
                pit_lap: current_lap + n,
                reachable,
                fuel_add: f.map(|x| x.0),
                ve_add: v.map(|x| x.0),
                stops_after,
                stationary_s: stationary,
                stop_cost_s: stationary.zip(lane_loss_s).map(|(s, l)| s + l),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tyres
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TyrePlan {
    /// Laps completed on the tyres now fitted (as far as the samples show).
    pub set_laps: u32,
    pub wear_now: Option<[f64; 4]>,
    /// Tread lost per lap, per tyre; `None` until a few laps on this set.
    pub wear_per_lap: Option<[f64; 4]>,
    /// Laps until the most worn tyre reaches the limit.
    pub laps_to_limit: Option<f64>,
    pub limit: f64,
    /// Pace change per lap on this set, from clean laps (includes the fuel
    /// burning off, which makes the car faster: a flat trend can hide wear).
    pub pace_trend_s_per_lap: Option<f64>,
}

/// The laps on the current tyre set: everything after the last lap on which
/// some tyre's tread went up (a tyre change).
pub fn current_set(laps: &[LapSample]) -> &[LapSample] {
    let with_wear: Vec<usize> = laps.iter().enumerate().filter(|(_, l)| l.wear.is_some()).map(|(i, _)| i).collect();
    let mut start = 0;
    for w in with_wear.windows(2) {
        let (a, b) = (laps[w[0]].wear.unwrap(), laps[w[1]].wear.unwrap());
        if (0..4).any(|i| b[i] > a[i] + 0.02) {
            start = w[1];
        }
    }
    &laps[start..]
}

pub fn tyre_plan(laps: &[LapSample], limit: f64) -> TyrePlan {
    let set = current_set(laps);
    let worn: Vec<(f64, [f64; 4])> = set.iter().filter_map(|l| l.wear.map(|w| (l.lap as f64, w))).collect();
    let wear_now = worn.last().map(|w| w.1);
    let wear_per_lap = (worn.len() >= 3).then(|| {
        let mut rate = [0.0; 4];
        for (i, r) in rate.iter_mut().enumerate() {
            let pts: Vec<(f64, f64)> = worn.iter().map(|(x, w)| (*x, w[i])).collect();
            *r = -slope(&pts).unwrap_or(0.0);
        }
        rate
    });
    let laps_to_limit = wear_now.zip(wear_per_lap).and_then(|(now, rate)| {
        (0..4)
            .filter(|&i| rate[i] > 1e-6)
            .map(|i| ((now[i] - limit) / rate[i]).max(0.0))
            .reduce(f64::min)
    });
    let clean = clean_laps(set);
    TyrePlan {
        set_laps: set.iter().filter(|l| !l.pit).count() as u32,
        wear_now,
        wear_per_lap,
        laps_to_limit,
        limit,
        pace_trend_s_per_lap: (clean.len() >= MIN_TREND_SAMPLES).then(|| slope(&clean)).flatten(),
    }
}

/// Fresh tyres or not at the next stop, as the two numbers to weigh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TyreChoice {
    /// Seconds the tyre change adds to the stop.
    pub change_cost_s: f64,
    /// Seconds new tyres would gain over the next stint at the observed
    /// pace trend (`None` if no trend, or it shows no loss).
    pub expected_gain_s: Option<f64>,
    /// The current tyres would pass the wear limit during the next stint.
    pub must_change: bool,
}

pub fn tyre_choice(
    plan: &TyrePlan,
    pit_times: &PitTimes,
    stop_in_laps: f64,
    next_stint_laps: f64,
    fuel_add: f64,
    ve_add: f64,
) -> TyreChoice {
    let with = stationary_time(pit_times, fuel_add, ve_add, 4, false);
    let without = stationary_time(pit_times, fuel_add, ve_add, 0, false);
    let age_at_stop = plan.set_laps as f64 + stop_in_laps;
    // Old tyres keep losing `trend` per lap of age; new ones start from zero.
    // Over the next stint the difference is the age at the stop times the
    // trend, on every lap.
    let expected_gain_s = plan
        .pace_trend_s_per_lap
        .filter(|t| *t > 0.0)
        .map(|t| t * age_at_stop * next_stint_laps);
    let must_change = plan
        .laps_to_limit
        .map(|l| l < stop_in_laps + next_stint_laps)
        .unwrap_or(false);
    TyreChoice { change_cost_s: with - without, expected_gain_s, must_change }
}

// ---------------------------------------------------------------------------
// Weather
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct ForecastNode {
    /// Fraction of the session (0, 0.25, 0.5, 0.75, 1).
    pub at: f64,
    /// 0 clear … 10 storm; 6 and above is rain.
    pub sky: i32,
    /// 0–1.
    pub rain_chance: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WeatherOutlook {
    pub raining_now: bool,
    /// Average water on the racing line, 0–1.
    pub wetness: f64,
    /// Next forecast point with rain likely (chance >= 50 % or a rain sky).
    pub rain_in_s: Option<f64>,
    pub rain_chance: Option<f64>,
    pub rain_in_laps: Option<f64>,
    /// While wet: next forecast point that looks dry again.
    pub dry_in_s: Option<f64>,
    pub dry_in_laps: Option<f64>,
}

const RAIN_SKY: i32 = 6;

pub fn weather_outlook(
    nodes: &[ForecastNode],
    session_length_s: Option<f64>,
    session_time: f64,
    raining_now: f64,
    wetness: f64,
    lap_time: Option<f64>,
) -> WeatherOutlook {
    let wet_now = raining_now > 0.05 || wetness > 0.1;
    let mut out = WeatherOutlook { raining_now: raining_now > 0.05, wetness, ..Default::default() };
    let Some(len) = session_length_s.filter(|l| *l > 0.0) else { return out };
    let laps = |s: f64| lap_time.filter(|l| *l > 0.0).map(|l| s / l);
    for n in nodes {
        let t = n.at * len - session_time;
        if t <= 0.0 {
            continue;
        }
        let rainy = n.rain_chance >= 0.5 || n.sky >= RAIN_SKY;
        if rainy && out.rain_in_s.is_none() && !wet_now {
            out.rain_in_s = Some(t);
            out.rain_chance = Some(n.rain_chance);
            out.rain_in_laps = laps(t);
        }
        if !rainy && n.rain_chance < 0.2 && wet_now && out.dry_in_s.is_none() {
            out.dry_in_s = Some(t);
            out.dry_in_laps = laps(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lap(n: i64, t: f64, fuel: f64, ve: f64) -> LapSample {
        LapSample {
            lap: n,
            session_time: n as f64 * t,
            lap_time: t,
            fuel: Some(fuel),
            ve: Some(ve),
            wear: Some([1.0 - 0.01 * n as f64; 4]),
            pit: false,
            caution: false,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn stats_median_mean_and_trend() {
        let s = Stat::of(&[(1.0, 3.0), (2.0, 3.1), (3.0, 3.2), (4.0, 3.3), (5.0, 9.0)]).unwrap();
        assert_eq!(s.n, 5);
        assert!(close(s.median, 3.2));
        assert!(close(s.mean, 4.32));
        assert_eq!((s.last, s.min, s.max), (9.0, 3.0, 9.0));
        assert!(s.trend_per_lap.unwrap() > 0.0);
        // Only the last ten count.
        let many: Vec<(f64, f64)> = (0..30).map(|i| (i as f64, if i < 20 { 100.0 } else { 1.0 })).collect();
        assert!(close(Stat::of(&many).unwrap().mean, 1.0));
        // Too few for a trend.
        assert_eq!(Stat::of(&[(1.0, 1.0), (2.0, 2.0)]).unwrap().trend_per_lap, None);
        assert!(Stat::of(&[]).is_none());
    }

    #[test]
    fn consumption_skips_pit_caution_and_gaps() {
        let mut laps = vec![lap(1, 90.0, 80.0, 0.9), lap(2, 90.0, 77.0, 0.85), lap(3, 90.0, 74.1, 0.80)];
        let mut l4 = lap(4, 120.0, 90.0, 1.0); // refuelled: through the pits
        l4.pit = true;
        laps.push(l4);
        laps.push(lap(5, 90.0, 87.0, 0.95));
        let mut l6 = lap(6, 130.0, 86.0, 0.94); // caution
        l6.caution = true;
        laps.push(l6);
        laps.push(lap(8, 90.0, 80.0, 0.88)); // lap 7 missing
        let fuel = consumption(&laps, |l| l.fuel);
        let used: Vec<f64> = fuel.iter().map(|p| (p.1 * 100.0).round() / 100.0).collect();
        // Lap 3 is the average of laps 2-3; after the stop, lap 5 starts a new run.
        assert_eq!(fuel.iter().map(|p| p.0 as i64).collect::<Vec<_>>(), vec![2, 3, 5]);
        assert_eq!(used, vec![3.0, 2.95, 3.0]);
        assert_eq!(consumption(&laps, |l| l.ve).len(), 3);
        // A missing reading breaks the run.
        laps[1].fuel = None;
        assert_eq!(consumption(&laps, |l| l.fuel).len(), 1);
    }

    #[test]
    fn reading_jitter_at_the_line_averages_out() {
        // 3 L a lap, but the level is read alternately just before and just
        // after the line (±0.5 L): lap by lap that is 2 and 4 L.
        let laps: Vec<LapSample> = (1..=9)
            .map(|n| lap(n, 90.0, 90.0 - 3.0 * n as f64 + if n % 2 == 0 { 0.5 } else { -0.5 }, 0.0))
            .collect();
        let st = Stat::of(&consumption(&laps, |l| l.fuel)).unwrap();
        assert!((st.median - 3.0).abs() < 0.15, "{st:?}");
        assert!((st.last - 3.0).abs() < 0.1, "{st:?}");
    }

    #[test]
    fn clean_laps_drop_slow_ones() {
        let mut laps: Vec<LapSample> = (1..=8).map(|n| lap(n, 90.0 + n as f64 * 0.1, 0.0, 0.0)).collect();
        laps[4].lap_time = 140.0; // a spin
        laps[6].pit = true;
        let clean = clean_laps(&laps);
        // lap 1 (session start), lap 5 (spin) and lap 7 (pit) are out.
        assert_eq!(clean.iter().map(|p| p.0 as i64).collect::<Vec<_>>(), vec![2, 3, 4, 6, 8]);
    }

    #[test]
    fn horizon_for_timed_and_lap_races() {
        // 10 minutes left, 90 s laps, half way round: finish this lap (45 s),
        // then 555 s = 6.17 laps → 7 more laps.
        let h = horizon(Some(600.0), None, 0.5, Some(90.0)).unwrap();
        assert!(close(h.laps_to_go, 7.5));
        assert_eq!(h.basis, "time");
        // The clock runs out during this lap: only its remainder is left.
        assert!(close(horizon(Some(20.0), None, 0.5, Some(90.0)).unwrap().laps_to_go, 0.5));
        // Lap race: 12 laps left including the current one.
        let h = horizon(Some(999.0), Some(12.0), 0.25, Some(90.0)).unwrap();
        assert!(close(h.laps_to_go, 11.75));
        assert_eq!(h.basis, "laps");
        // Timed race without a lap time: unknown, not a guess.
        assert!(horizon(Some(600.0), None, 0.5, None).is_none());
        assert!(horizon(None, None, 0.5, Some(90.0)).is_none());
    }

    fn stat(per_lap: f64) -> Stat {
        Stat::of(&[(1.0, per_lap), (2.0, per_lap), (3.0, per_lap)]).unwrap()
    }

    #[test]
    fn energy_plan_counts_stops() {
        // 100 L tank, 3 L/lap, 0.5 lap reserve (1.5 L), 50 L in, 60 laps to go.
        let p = energy_plan("L", 50.0, 100.0, stat(3.0), 0.5, Some(60.0)).unwrap();
        assert!(close(p.reserve, 1.5));
        assert!(close(p.laps_left, 48.5 / 3.0));
        assert!(close(p.needed_to_finish.unwrap(), 181.5));
        assert!(close(p.to_add.unwrap(), 131.5));
        assert!(close(p.stint_laps_max, 98.5 / 3.0));
        assert_eq!(p.stops, Some(2)); // 131.5 / 98.5 → 2
        // Enough on board: no stop.
        let p = energy_plan("L", 50.0, 100.0, stat(3.0), 0.5, Some(10.0)).unwrap();
        assert_eq!((p.to_add, p.stops), (Some(0.0), Some(0)));
        // Unknown race length: no stop count, but laps left still known.
        let p = energy_plan("L", 50.0, 100.0, stat(3.0), 0.5, None).unwrap();
        assert_eq!(p.stops, None);
        assert!(p.laps_left > 16.0);
        // No consumption yet: no plan.
        assert!(energy_plan("L", 50.0, 100.0, Stat::default(), 0.5, Some(10.0)).is_none());
    }

    #[test]
    fn pit_window_from_range_and_stints() {
        // Just past the line (the whole lap 21 still to drive), 40 laps to go,
        // 15 laps of range, 30-lap stints, 1 stop: latest = 15 − 1 = 14 more
        // laps after this one, earliest = 40 − 1 − 30 = 9.
        let w = pit_window(40.0, 0.0, 21, 15.0, 30.0, 1).unwrap();
        assert_eq!((w.earliest_in, w.latest_in, w.stops), (9, 14, 1));
        assert_eq!((w.earliest_lap, w.latest_lap), (30, 35));
        // Mid-lap: 0.4 of this lap still to drive before the line.
        let w = pit_window(40.0, 0.6, 21, 15.0, 30.0, 1).unwrap();
        assert_eq!((w.earliest_in, w.latest_in), (10, 14));
        // Window would close: one more stop is needed.
        let w = pit_window(80.0, 0.0, 1, 15.0, 30.0, 2).unwrap();
        assert_eq!(w.stops, 3);
        assert!(w.earliest_in <= w.latest_in);
        // Not enough to finish this lap.
        let w = pit_window(40.0, 0.5, 21, 0.2, 30.0, 1).unwrap();
        assert_eq!((w.earliest_in, w.latest_in), (0, 0));
        assert!(pit_window(40.0, 0.0, 1, 15.0, 30.0, 0).is_none());
    }

    fn times() -> PitTimes {
        // The values LMU reported for a Hypercar at Monza (1.4.2).
        PitTimes {
            fuel_fill_rate: 4.0,
            fuel_insert_s: 1.0,
            fuel_concurrent: true,
            ve_fill_rate: 0.025,
            ve_insert_s: 0.0,
            ve_concurrent: true,
            four_tyres_s: 12.0,
            two_tyres_s: 4.5,
            tyres_concurrent: false,
            driver_change_s: 25.0,
            driver_concurrent: true,
        }
    }

    #[test]
    fn stationary_time_overlaps_concurrent_work() {
        let t = times();
        // 40 L at 4 L/s = 11 s with the insert; 50 % VE at 2.5 %/s = 20 s: overlap → 20.
        assert!(close(stationary_time(&t, 40.0, 0.5, 0, false), 20.0));
        // Tyres are sequential: +12.
        assert!(close(stationary_time(&t, 40.0, 0.5, 4, false), 32.0));
        // A driver change (25 s) overlaps with refuelling.
        assert!(close(stationary_time(&t, 40.0, 0.5, 4, true), 37.0));
        assert!(close(stationary_time(&t, 0.0, 0.0, 0, false), 0.0));
    }

    #[test]
    fn what_if_rows_cover_now_and_the_window() {
        let w = PitWindow { earliest_in: 3, latest_in: 6, earliest_lap: 13, latest_lap: 16, stops: 1 };
        let fuel = Tank { now: 30.0, capacity: 100.0, per_lap: 3.0, reserve: 1.5 };
        let rows = what_if(&w, 10, 0.0, 30.0, Some(fuel), None, Some(&times()), 0, Some(25.0));
        assert_eq!(rows.iter().map(|r| r.in_laps).collect::<Vec<_>>(), vec![0, 3, 4, 5, 6]);
        // Box at the end of lap 10 (one lap to the line, 27 L left): need
        // 29 laps × 3 + 1.5 = 88.5, add 61.5.
        let now = &rows[0];
        assert!(now.reachable);
        assert!(close(now.fuel_add.unwrap(), 61.5));
        assert_eq!(now.stops_after, 0);
        assert!(close(now.stationary_s.unwrap(), 1.0 + 61.5 / 4.0));
        assert!(close(now.stop_cost_s.unwrap(), now.stationary_s.unwrap() + 25.0));
        // The fuel still needed to the flag does not depend on when the stop
        // is, as long as it fits in the tank: only the stop's lap changes.
        assert!(close(rows[4].fuel_add.unwrap(), rows[1].fuel_add.unwrap()));
        // Without pit times: amounts, but no time.
        let rows = what_if(&w, 10, 0.0, 30.0, Some(fuel), None, None, 0, Some(25.0));
        assert_eq!((rows[0].stationary_s, rows[0].stop_cost_s), (None, None));
    }

    #[test]
    fn boxing_too_early_shows_the_extra_stop() {
        // 40 laps to go, 33 L in a 100 L tank at 3 L/lap: one stop is enough
        // from 7 laps after this one (a full tank lasts 32.8 laps); boxing at
        // the end of this lap leaves 39 laps, more than a tank.
        let w = PitWindow { earliest_in: 7, latest_in: 9, earliest_lap: 7, latest_lap: 9, stops: 1 };
        let fuel = Tank { now: 33.0, capacity: 100.0, per_lap: 3.0, reserve: 1.5 };
        let rows = what_if(&w, 0, 0.0, 40.0, Some(fuel), None, None, 0, None);
        assert_eq!(rows.iter().map(|r| r.in_laps).collect::<Vec<_>>(), vec![0, 7, 8, 9]);
        assert_eq!(rows[0].stops_after, 1);
        assert_eq!(rows[1].stops_after, 0);
    }

    #[test]
    fn unreachable_stops_are_flagged() {
        let w = PitWindow { earliest_in: 0, latest_in: 3, earliest_lap: 1, latest_lap: 4, stops: 1 };
        let fuel = Tank { now: 5.0, capacity: 100.0, per_lap: 3.0, reserve: 1.5 };
        let rows = what_if(&w, 1, 0.0, 20.0, Some(fuel), None, None, 0, None);
        assert!(rows[0].reachable);
        assert!(!rows.last().unwrap().reachable);
    }

    #[test]
    fn tyre_set_wear_rate_and_pace_trend() {
        let mut laps: Vec<LapSample> = (1..=12)
            .map(|n| {
                let mut l = lap(n, 90.0, 0.0, 0.0);
                l.wear = Some([1.0 - 0.02 * n as f64, 1.0 - 0.01 * n as f64, 1.0, 1.0]);
                l
            })
            .collect();
        // New tyres from lap 7, losing 0.05 s per lap of age.
        for (age, l) in laps[6..].iter_mut().enumerate() {
            l.wear = Some([1.0 - 0.02 * age as f64, 1.0 - 0.01 * age as f64, 1.0, 1.0]);
            l.lap_time = 90.0 + 0.05 * age as f64;
        }
        let p = tyre_plan(&laps, 0.3);
        assert_eq!(p.set_laps, 6);
        let rate = p.wear_per_lap.unwrap();
        assert!(close(rate[0], 0.02) && close(rate[1], 0.01) && close(rate[2], 0.0));
        // FL at 0.90, limit 0.30, 0.02/lap → 30 laps.
        assert!(close(p.laps_to_limit.unwrap(), 30.0));
        assert!(close(p.pace_trend_s_per_lap.unwrap(), 0.05));

        let c = tyre_choice(&p, &times(), 4.0, 20.0, 40.0, 0.0);
        assert!(close(c.change_cost_s, 12.0));
        // Age at the stop 10 laps × 0.05 s × 20 laps = 10 s.
        assert!(close(c.expected_gain_s.unwrap(), 10.0));
        assert!(!c.must_change);
        assert!(tyre_choice(&p, &times(), 4.0, 40.0, 40.0, 0.0).must_change);
    }

    #[test]
    fn weather_outlook_from_the_forecast() {
        let nodes = [
            ForecastNode { at: 0.0, sky: 1, rain_chance: 0.0 },
            ForecastNode { at: 0.25, sky: 2, rain_chance: 0.1 },
            ForecastNode { at: 0.5, sky: 6, rain_chance: 0.6 },
            ForecastNode { at: 0.75, sky: 4, rain_chance: 0.1 },
            ForecastNode { at: 1.0, sky: 1, rain_chance: 0.0 },
        ];
        // 1 h session, 20 min in: rain likely at 30 min → 600 s, 6.7 laps of 90 s.
        let w = weather_outlook(&nodes, Some(3600.0), 1200.0, 0.0, 0.0, Some(90.0));
        assert!(close(w.rain_in_s.unwrap(), 600.0));
        assert!(close(w.rain_chance.unwrap(), 0.6));
        assert!(close(w.rain_in_laps.unwrap(), 600.0 / 90.0));
        assert_eq!(w.dry_in_s, None);
        // Wet now at 35 min: dry again at 45 min.
        let w = weather_outlook(&nodes, Some(3600.0), 2100.0, 0.3, 0.4, Some(90.0));
        assert!(w.raining_now);
        assert_eq!(w.rain_in_s, None);
        assert!(close(w.dry_in_s.unwrap(), 600.0));
        // Unknown session length: nothing from the forecast.
        assert_eq!(weather_outlook(&nodes, None, 0.0, 0.0, 0.0, None).rain_in_s, None);
    }
}
