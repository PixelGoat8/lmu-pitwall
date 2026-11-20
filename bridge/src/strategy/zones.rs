//! Lift-and-coast corner by corner: what it costs in time and saves in fuel,
//! measured from the race laps themselves.
//!
//! The lap time is a poor measure of that cost: traffic, a mistake or the
//! tyres move it by a few tenths, as much as lift-and-coast itself costs. But
//! lift-and-coast only costs time in a few places: from where the driver lifts
//! before a braking zone to where the car is back at the speed of a normal lap,
//! around the apex. So each lap is laid on a grid of short stretches by
//! distance, and only the stretches where the driver coasted more than on the
//! normal laps are compared with them. A mistake or a car in the way elsewhere
//! on the lap does not enter the figure, and zones with a car close by are
//! left out.
//!
//! Distance comes from where the car is: the first good lap of the session
//! gives the path, and every lap after it is placed on that path by its
//! position. Speed integrated over the lap would do without positions, and is
//! what the first lap uses, but it drifts by a few metres with the line (a
//! hairpin taken a metre wide is three metres longer), and under braking a
//! few metres are worth hundredths of a second per stretch. Fuel per stretch
//! is there only when the PC that is driving sends a level that moves during
//! the lap.
//!
//! Like `model`, these are pure functions over samples; `ZoneTracker` only
//! keeps the laps.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use super::model::{CLEAN_LAP_FACTOR, LICO_STEP_S};

/// Length of one stretch of the grid, metres.
pub const GRID_M: f64 = 10.0;
/// A lap with a gap in the telemetry longer than this is not used, seconds.
const MAX_GAP_S: f64 = 1.0;
/// The distance integrated from the speed must be within this fraction of
/// the track length, or the lap is not used (garage, telemetry glitch).
const DISTANCE_TOLERANCE: f64 = 0.10;
/// Normal laps: coasting within this many seconds of the driver's least.
const REF_COAST_MARGIN_S: f64 = 0.5;
/// Normal laps compared against, per driver: the latest ones…
const REF_LAPS: usize = 5;
/// …and at least these.
const MIN_REF_LAPS: usize = 2;
/// Recent laps kept to look for lift-and-coast in.
const KEEP: usize = 20;
/// The figure is the median over this many of the latest laps with LiCo.
const COST_LAPS: usize = 3;
/// A zone starts with at least this much coasting where the normal laps do
/// not coast, metres: a blip of the pedal is not lift-and-coast.
const MIN_RUN_M: f64 = 20.0;
/// Speeds this close to the normal laps' count as the same, m/s…
const SPEED_TOL_MS: f64 = 1.0;
/// …and this many metres in a row of the same speed end the zone. One
/// stretch is not enough: braking later than on a normal lap, the car crosses
/// the normal speed on its way down to the apex.
const CONVERGED_M: f64 = 30.0;
/// A zone ends at most this far after the coasting, metres.
const MAX_TAIL_M: f64 = 500.0;
/// A lap whose zones clear of traffic hold less than this share of its extra
/// coasting is not used.
const MIN_CLEAR_SHARE: f64 = 0.5;
/// Zones starting this close together are the same corner, metres.
const CORNER_MERGE_M: f64 = 150.0;
/// The fuel level must take at least this many values over a lap to be read
/// stretch by stretch; a level that only moves at the line says nothing.
const MIN_FUEL_VALUES: usize = 20;
/// The end of a zone: the corner exit, where the normal laps' speed is this
/// much above its lowest. A zone that ended at the apex, where the car is
/// slowest, would be the most sensitive to a metre of difference between laps.
const EXIT_RISE_MS: f64 = 5.0;
/// A sample farther than this from the path is off it (the pit lane, an
/// excursion): the lap is not used, metres.
const MAX_OFF_PATH_M: f64 = 25.0;
/// Fastest a car goes, to bound where the next sample can be, m/s.
const MAX_SPEED_MS: f64 = 110.0;
/// Clean laps in a row off the path before the path is taken again.
const PATH_MISSES: u32 = 3;

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// One telemetry sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Session time, seconds (the game's clock, so a replay at any speed works).
    pub et: f64,
    pub speed: f64,
    /// At speed with neither throttle nor brake.
    pub coasting: bool,
    /// In the pit lane or under caution: the lap is not compared.
    pub excluded: bool,
    /// Another car close on track.
    pub traffic: bool,
    /// Litres; `None` unless read on the PC that is driving.
    pub fuel: Option<f64>,
    /// Where the car is, on the ground plane (x, z), metres.
    pub pos: Option<(f64, f64)>,
}

// ---------------------------------------------------------------------------
// One lap on the distance grid
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    /// Laps completed at its end.
    pub lap: i64,
    pub driver: String,
    pub lap_time: f64,
    pub step_m: f64,
    /// Per stretch: seconds spent, mean speed, coasting, a car close by.
    pub dt: Vec<f64>,
    pub v: Vec<f64>,
    pub coast: Vec<bool>,
    pub traffic: Vec<bool>,
    /// Litres used per stretch, when the level could be read along the lap.
    pub fuel: Option<Vec<f64>>,
    /// No pit lane, no caution.
    pub clean: bool,
    /// Seconds coasting over the lap.
    pub coast_s: f64,
}

/// The grid for a track: number of stretches and their length.
fn grid(track_length: f64) -> (usize, f64) {
    let n = ((track_length / GRID_M).round() as usize).max(1);
    (n, track_length / n as f64)
}

/// The samples cover the lap, `start_et` to `end_et`, without a long gap.
fn watched_whole(samples: &[Sample], start_et: f64, end_et: f64) -> bool {
    let mut prev = start_et;
    for s in samples {
        if s.et < prev - 1e-6 || s.et - prev > MAX_GAP_S {
            return false;
        }
        prev = s.et;
    }
    !samples.is_empty() && end_et >= prev && end_et - prev <= MAX_GAP_S
}

/// Distance of each sample from the line by the speed, trapezoids between
/// samples, scaled to the track length. `None` if it is too far off it.
fn integrated(samples: &[Sample], start_et: f64, end_et: f64, track_length: f64) -> Option<Vec<f64>> {
    let speed = |s: &Sample| s.speed.max(0.0);
    let first = samples.first()?;
    let last = samples.last()?;
    let mut d = Vec::with_capacity(samples.len());
    d.push(speed(first) * (first.et - start_et));
    for w in samples.windows(2) {
        let prev_d = d[d.len() - 1];
        d.push(prev_d + 0.5 * (speed(&w[0]) + speed(&w[1])) * (w[1].et - w[0].et));
    }
    let total = d[d.len() - 1] + speed(last) * (end_et - last.et);
    if total <= 0.0 || (total / track_length - 1.0).abs() > DISTANCE_TOLERANCE {
        return None;
    }
    let scale = track_length / total;
    Some(d.into_iter().map(|x| x * scale).collect())
}

/// The track as driven on one lap: where the car was at each grid point, to
/// place the other laps by where the car is rather than by how far it went.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pts: Vec<(f64, f64)>,
    step: f64,
}

/// The path of a lap watched whole, clean and with positions.
pub fn make_path(samples: &[Sample], start_et: f64, end_et: f64, track_length: f64) -> Option<Path> {
    if track_length <= 0.0 || !watched_whole(samples, start_et, end_et) || samples.iter().any(|s| s.excluded) {
        return None;
    }
    let pos: Vec<(f64, f64)> = samples.iter().map(|s| s.pos).collect::<Option<_>>()?;
    let d = integrated(samples, start_et, end_et, track_length)?;
    let (n, step) = grid(track_length);
    let mut pts = Vec::with_capacity(n + 1);
    let mut j = 0;
    for k in 0..=n {
        let s = k as f64 * step;
        while j + 1 < d.len() && d[j + 1] <= s {
            j += 1;
        }
        pts.push(if s <= d[0] || j + 1 >= d.len() {
            pos[j]
        } else {
            let f = if d[j + 1] - d[j] > 1e-9 { (s - d[j]) / (d[j + 1] - d[j]) } else { 0.0 };
            (pos[j].0 + f * (pos[j + 1].0 - pos[j].0), pos[j].1 + f * (pos[j + 1].1 - pos[j].1))
        });
    }
    Some(Path { pts, step })
}

/// Distance along `path` of each sample: the nearest point of the path, a
/// little behind or ahead of the previous sample's. `None` if a sample has no
/// position or is off the path, or the lap does not reach the line.
fn projected(samples: &[Sample], start_et: f64, end_et: f64, path: &Path) -> Option<Vec<f64>> {
    let segments = path.pts.len().checked_sub(1).filter(|s| *s > 0)?;
    let (mut k, mut prev_s, mut prev_et) = (0usize, 0.0f64, start_et);
    let mut out = Vec::with_capacity(samples.len());
    for s in samples {
        let (x, z) = s.pos?;
        let reach = ((MAX_SPEED_MS * (s.et - prev_et) + 2.0 * path.step) / path.step).ceil() as usize;
        let mut best = (f64::INFINITY, k, 0.0);
        for m in k.saturating_sub(1)..=(k + reach).min(segments - 1) {
            let (a, b) = (path.pts[m], path.pts[m + 1]);
            let (dx, dz) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dz * dz;
            let t = if len2 > 1e-12 { (((x - a.0) * dx + (z - a.1) * dz) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let d2 = (x - a.0 - t * dx).powi(2) + (z - a.1 - t * dz).powi(2);
            if d2 < best.0 {
                best = (d2, m, t);
            }
        }
        if best.0.sqrt() > MAX_OFF_PATH_M {
            return None;
        }
        k = best.1;
        prev_s = ((best.1 as f64 + best.2) * path.step).max(prev_s);
        prev_et = s.et;
        out.push(prev_s);
    }
    let length = segments as f64 * path.step;
    (length - prev_s <= MAX_SPEED_MS * (end_et - prev_et) + 2.0 * path.step).then_some(out)
}

/// A lap from its samples, `start_et` to `end_et`, placed on `path` when
/// there is one and the samples have positions, by the speed otherwise.
/// `None` unless the lap was watched whole and its distance fits the track.
pub fn build_trace(lap: i64, driver: &str, samples: &[Sample], start_et: f64, end_et: f64, track_length: f64, path: Option<&Path>) -> Option<Trace> {
    let lap_time = end_et - start_et;
    if track_length <= 0.0 || lap_time <= 0.0 || !watched_whole(samples, start_et, end_et) {
        return None;
    }
    let d = match path {
        Some(p) if samples.iter().all(|s| s.pos.is_some()) && p.pts.len() == grid(track_length).0 + 1 => projected(samples, start_et, end_et, p)?,
        _ => integrated(samples, start_et, end_et, track_length)?,
    };
    let first = samples[0];
    let last = samples[samples.len() - 1];

    // Knots: (distance, time into the lap, speed, fuel), with the line at
    // both ends.
    let fuel_read = samples.iter().all(|s| s.fuel.is_some()) && {
        let mut levels: Vec<f64> = samples.iter().filter_map(|s| s.fuel).collect();
        levels.sort_by(f64::total_cmp);
        levels.dedup();
        levels.len() >= MIN_FUEL_VALUES
    };
    let fuel_of = |s: &Sample| if fuel_read { s.fuel.unwrap_or(0.0) } else { 0.0 };
    let mut knots: Vec<(f64, f64, f64, f64)> = Vec::with_capacity(samples.len() + 2);
    knots.push((0.0, 0.0, first.speed, fuel_of(&first)));
    for (s, dist) in samples.iter().zip(&d) {
        knots.push((*dist, s.et - start_et, s.speed, fuel_of(s)));
    }
    knots.push((track_length, lap_time, last.speed, fuel_of(&last)));

    // Grid points: time, speed and fuel interpolated along the distance.
    let (n, step) = grid(track_length);
    let (mut tau, mut vel, mut lvl) = (Vec::with_capacity(n + 1), Vec::with_capacity(n + 1), Vec::with_capacity(n + 1));
    let mut j = 0;
    for k in 0..=n {
        let s = k as f64 * step;
        while j + 2 < knots.len() && knots[j + 1].0 <= s {
            j += 1;
        }
        let (a, b) = (knots[j], knots[j + 1]);
        let f = if b.0 - a.0 > 1e-9 { ((s - a.0) / (b.0 - a.0)).clamp(0.0, 1.0) } else { 1.0 };
        tau.push(a.1 + f * (b.1 - a.1));
        vel.push(a.2 + f * (b.2 - a.2));
        lvl.push(a.3 + f * (b.3 - a.3));
    }

    // Per stretch: the state of the last sample before its middle.
    let (mut coast, mut traffic) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut q = 0;
    for i in 0..n {
        let mid = (i as f64 + 0.5) * step;
        while q + 1 < samples.len() && d[q + 1] <= mid {
            q += 1;
        }
        coast.push(samples[q].coasting);
        traffic.push(samples[q].traffic);
    }
    let dt: Vec<f64> = tau.windows(2).map(|w| w[1] - w[0]).collect();
    let v: Vec<f64> = vel.windows(2).map(|w| 0.5 * (w[0] + w[1])).collect();
    let fuel = fuel_read.then(|| lvl.windows(2).map(|w| w[0] - w[1]).collect());
    let coast_s: f64 = dt.iter().zip(&coast).filter(|(_, c)| **c).map(|(t, _)| t).sum();
    Some(Trace {
        lap,
        driver: driver.to_string(),
        lap_time,
        step_m: step,
        dt,
        v,
        coast,
        traffic,
        fuel,
        clean: !samples.iter().any(|s| s.excluded),
        coast_s,
    })
}

// ---------------------------------------------------------------------------
// The normal laps
// ---------------------------------------------------------------------------

/// The normal laps, stretch by stretch: median time and speed, coasting on
/// most of them, median fuel.
#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    pub dt: Vec<f64>,
    pub v: Vec<f64>,
    pub coast: Vec<bool>,
    pub fuel: Option<Vec<f64>>,
    pub laps: u32,
    pub coast_s: f64,
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 { v[n / 2] } else { 0.5 * (v[n / 2 - 1] + v[n / 2]) }
}

pub fn reference(laps: &[&Trace]) -> Option<Reference> {
    let n = laps.first()?.dt.len();
    if laps.iter().any(|l| l.dt.len() != n) {
        return None;
    }
    let col = |pick: &dyn Fn(&Trace) -> f64| -> f64 { median(laps.iter().map(|l| pick(l)).collect()) };
    let fuel = laps.iter().all(|l| l.fuel.is_some()).then(|| {
        (0..n).map(|i| median(laps.iter().filter_map(|l| l.fuel.as_ref().map(|f| f[i])).collect())).collect()
    });
    Some(Reference {
        dt: (0..n).map(|i| col(&|l: &Trace| l.dt[i])).collect(),
        v: (0..n).map(|i| col(&|l: &Trace| l.v[i])).collect(),
        coast: (0..n).map(|i| laps.iter().filter(|l| l.coast[i]).count() * 2 > laps.len()).collect(),
        fuel,
        laps: laps.len() as u32,
        coast_s: col(&|l: &Trace| l.coast_s),
    })
}

// ---------------------------------------------------------------------------
// One lap against the normal laps
// ---------------------------------------------------------------------------

/// A stretch of track where the driver coasted more than on the normal laps,
/// up to where the speed is back to theirs.
#[derive(Debug, Clone, PartialEq)]
pub struct Zone {
    pub start_m: f64,
    pub end_m: f64,
    /// Seconds lost against the normal laps.
    pub cost_s: f64,
    /// Seconds coasting where the normal laps do not.
    pub extra_coast_s: f64,
    /// Litres saved against the normal laps.
    pub fuel_saved: Option<f64>,
    /// A car was close: the time lost may be traffic, not lift-and-coast.
    pub traffic: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LapLico {
    pub lap: i64,
    /// Seconds lost to lift-and-coast on the lap: the zones clear of traffic,
    /// scaled up to all the lap's extra coasting.
    pub cost_s: f64,
    pub extra_coast_s: f64,
    pub fuel_saved: Option<f64>,
    pub zones: Vec<Zone>,
    /// Enough of the extra coasting was clear of traffic to measure the lap.
    pub usable: bool,
}

/// A zone as stretches: `core` from the lift to where the speed is back to
/// the normal laps', `all` on to the corner exit.
struct Span {
    core: std::ops::Range<usize>,
    all: std::ops::Range<usize>,
}

/// The zones of a lap, against the normal laps driven at `pace` (their times
/// multiplied by it).
fn spans(lap: &Trace, r: &Reference, pace: f64) -> Vec<Span> {
    let n = lap.dt.len();
    let step = lap.step_m;
    let extra = |i: usize| lap.coast[i] && !r.coast[i];
    let min_run = (MIN_RUN_M / step).ceil() as usize;
    let converged = ((CONVERGED_M / step).ceil() as usize).max(1);
    let max_tail = (MAX_TAIL_M / step).ceil() as usize;
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if !extra(i) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n && extra(j) {
            j += 1;
        }
        if j - i < min_run {
            i = j;
            continue;
        }
        // The tail: until the speed has been back to the normal laps' for a
        // few stretches in a row.
        let (mut end, mut same, mut k) = (j, 0, j);
        while k < n && k - j < max_tail {
            if !extra(k) && (lap.v[k] - r.v[k] / pace).abs() <= SPEED_TOL_MS {
                same += 1;
                if same == converged {
                    // Keep the stretches it took to converge: within the
                    // tolerance is not the same speed yet.
                    end = k + 1;
                    break;
                }
            } else {
                same = 0;
                end = k + 1;
            }
            k += 1;
        }
        // On to the corner exit (see `EXIT_RISE_MS`).
        let core = i..end;
        let mut lowest = r.v[i..end].iter().copied().fold(f64::INFINITY, f64::min);
        let mut exit = end;
        while exit < n && exit - j < max_tail && !extra(exit) {
            lowest = lowest.min(r.v[exit]);
            if r.v[exit] >= lowest + EXIT_RISE_MS {
                end = exit;
                break;
            }
            exit += 1;
        }
        out.push(Span { core, all: i..end });
        i = end;
    }
    out
}

pub fn analyse(lap: &Trace, r: &Reference) -> Option<LapLico> {
    let n = lap.dt.len();
    if r.dt.len() != n || n == 0 {
        return None;
    }
    // The lap's pace against the normal laps where lift-and-coast does not
    // reach (the straights, the corner exits): tyres and fuel load change it
    // everywhere, the zones included. A median of the stretches, so a mistake
    // somewhere does not move it.
    let mut away = vec![true; n];
    for z in spans(lap, r, 1.0) {
        z.core.for_each(|m| away[m] = false);
    }
    let ratios: Vec<f64> = (0..n).filter(|i| away[*i] && !lap.traffic[*i] && r.dt[*i] > 0.0).map(|i| lap.dt[i] / r.dt[i]).collect();
    // Too little of the lap to tell (traffic all round): the lap cannot be
    // measured, as worn tyres would pass for lift-and-coast.
    let measured_pace = (ratios.len() * 4 >= n).then(|| median(ratios));
    let pace = measured_pace.unwrap_or(1.0);

    let extra = |i: usize| lap.coast[i] && !r.coast[i];
    let fuel = lap.fuel.as_ref().zip(r.fuel.as_ref());
    let zones: Vec<Zone> = spans(lap, r, pace)
        .into_iter()
        .map(|span| span.all)
        .map(|range| Zone {
            start_m: range.start as f64 * lap.step_m,
            end_m: range.end as f64 * lap.step_m,
            cost_s: range.clone().map(|m| lap.dt[m] - pace * r.dt[m]).sum(),
            extra_coast_s: range.clone().filter(|m| extra(*m)).map(|m| lap.dt[m]).sum(),
            fuel_saved: fuel.map(|(lf, rf)| range.clone().map(|m| rf[m] - lf[m]).sum()),
            traffic: range.clone().any(|m| lap.traffic[m]),
        })
        .collect();

    let total_extra: f64 = zones.iter().map(|z| z.extra_coast_s).sum();
    let clear: Vec<&Zone> = zones.iter().filter(|z| !z.traffic).collect();
    let clear_extra: f64 = clear.iter().map(|z| z.extra_coast_s).sum();
    let usable = measured_pace.is_some() && total_extra > 0.0 && clear_extra >= MIN_CLEAR_SHARE * total_extra;
    let scale = if usable { total_extra / clear_extra } else { 0.0 };
    let fuel_saved = clear
        .iter()
        .map(|z| z.fuel_saved)
        .sum::<Option<f64>>()
        .filter(|_| fuel.is_some())
        .map(|f| f * scale);
    Some(LapLico {
        lap: lap.lap,
        cost_s: clear.iter().map(|z| z.cost_s).sum::<f64>() * scale,
        extra_coast_s: total_extra,
        fuel_saved,
        zones,
        usable,
    })
}

// ---------------------------------------------------------------------------
// The figure the plan shows
// ---------------------------------------------------------------------------

/// What lift-and-coast costs, from the latest laps the driver did it on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LicoCost {
    /// Seconds lift-and-coast costs per lap: median over the laps below.
    pub s_per_lap: f64,
    /// Coasting per lap beyond the normal laps, seconds.
    pub extra_coast_s: f64,
    /// Litres the zones save per lap; `None` when the level cannot be read
    /// along the lap.
    pub fuel_saved_per_lap: Option<f64>,
    /// Seconds lost per litre saved.
    pub s_per_litre: Option<f64>,
    /// Laps measured, and the first and last of them.
    pub laps: u32,
    pub first_lap: i64,
    pub last_lap: i64,
    /// The normal laps compared against: how many, and their coasting.
    pub ref_laps: u32,
    pub ref_coast_s: f64,
    /// Zones left out because a car was close.
    pub traffic_zones: u32,
    /// Where the time and fuel go, corner by corner, in track order.
    pub corners: Vec<Corner>,
    /// The driver it was measured on.
    pub driver: String,
}

/// One corner's share, averaged over the laps measured.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Corner {
    /// Where the driver lifts, metres from the line.
    pub at_m: f64,
    pub cost_s: f64,
    pub extra_coast_s: f64,
    pub fuel_saved: Option<f64>,
    /// Laps this corner was measured on.
    pub laps: u32,
}

fn corners(laps: &[LapLico]) -> Vec<Corner> {
    let mut zones: Vec<&Zone> = laps.iter().flat_map(|l| l.zones.iter()).filter(|z| !z.traffic).collect();
    zones.sort_by(|a, b| a.start_m.total_cmp(&b.start_m));
    let mut groups: Vec<Vec<&Zone>> = Vec::new();
    for z in zones {
        match groups.last_mut() {
            Some(g) if z.start_m - g[0].start_m <= CORNER_MERGE_M => g.push(z),
            _ => groups.push(vec![z]),
        }
    }
    groups
        .into_iter()
        .map(|g| {
            let k = g.len() as f64;
            Corner {
                at_m: median(g.iter().map(|z| z.start_m).collect()),
                cost_s: g.iter().map(|z| z.cost_s).sum::<f64>() / k,
                extra_coast_s: g.iter().map(|z| z.extra_coast_s).sum::<f64>() / k,
                fuel_saved: g.iter().map(|z| z.fuel_saved).sum::<Option<f64>>().map(|f| f / k),
                laps: g.len() as u32,
            }
        })
        .collect()
}

/// The laps of one session, and the figure from them.
#[derive(Debug, Default)]
pub struct ZoneTracker {
    start_et: Option<f64>,
    samples: Vec<Sample>,
    /// Latest clean laps, any coasting.
    recent: VecDeque<Trace>,
    /// Latest normal laps per driver. Kept apart from `recent` so that a long
    /// spell of lift-and-coast does not push them out.
    normal: HashMap<String, VecDeque<Trace>>,
    /// Least coasting seen per driver on a clean lap.
    least: HashMap<String, f64>,
    /// Where the laps are placed: the first clean lap with positions.
    path: Option<Path>,
    /// Clean laps in a row that did not fit the path.
    off_path: u32,
}

impl ZoneTracker {
    /// One sample of the lap that started at `lap_start_et`. A new lap closes
    /// the one before: `closed_lap` is its number (laps completed at its end),
    /// `driver` who drove it.
    pub fn on_sample(&mut self, lap_start_et: f64, s: Sample, track_length: f64, closed_lap: i64, driver: &str) {
        match self.start_et {
            Some(st) if (lap_start_et - st).abs() < 1e-6 => {}
            Some(st) if lap_start_et > st => {
                if self.path.is_none() {
                    self.path = make_path(&self.samples, st, lap_start_et, track_length);
                }
                match build_trace(closed_lap, driver, &self.samples, st, lap_start_et, track_length, self.path.as_ref()) {
                    Some(t) => {
                        self.off_path = 0;
                        self.add(t);
                    }
                    // A clean lap that only fails on the path: the path may
                    // be the odd one (an excursion on that lap). After a few,
                    // a new one is taken.
                    None if self.path.is_some()
                        && !self.samples.iter().any(|s| s.excluded)
                        && build_trace(closed_lap, driver, &self.samples, st, lap_start_et, track_length, None).is_some() =>
                    {
                        self.off_path += 1;
                        if self.off_path >= PATH_MISSES {
                            self.path = None;
                            self.off_path = 0;
                        }
                    }
                    None => {}
                }
                self.samples.clear();
                self.start_et = Some(lap_start_et);
            }
            _ => {
                self.samples.clear();
                self.start_et = Some(lap_start_et);
            }
        }
        if s.et >= lap_start_et {
            self.samples.push(s);
        }
    }

    fn add(&mut self, t: Trace) {
        if !t.clean {
            return;
        }
        // Pace: a lap well off the recent ones (a spin, a slow zone) is left out.
        let recent: Vec<f64> = self.recent.iter().filter(|r| r.driver == t.driver).map(|r| r.lap_time).collect();
        if !recent.is_empty() && t.lap_time > CLEAN_LAP_FACTOR * median(recent) {
            return;
        }
        let least = self.least.entry(t.driver.clone()).or_insert(f64::INFINITY);
        *least = least.min(t.coast_s);
        let limit = *least + REF_COAST_MARGIN_S;
        let normal = self.normal.entry(t.driver.clone()).or_default();
        if t.coast_s <= limit {
            normal.push_back(t.clone());
        }
        normal.retain(|r| r.coast_s <= limit);
        while normal.len() > REF_LAPS {
            normal.pop_front();
        }
        self.recent.push_back(t);
        while self.recent.len() > KEEP {
            self.recent.pop_front();
        }
    }

    /// What lift-and-coast costs the driver of the latest lap: `None` until
    /// there are normal laps to compare with and laps with lift-and-coast.
    pub fn summary(&self) -> Option<LicoCost> {
        let driver = &self.recent.back()?.driver;
        let normal = self.normal.get(driver)?;
        if normal.len() < MIN_REF_LAPS {
            return None;
        }
        // The normal laps lined up on the latest, then each lap on them.
        let r = reference(&normal.iter().collect::<Vec<_>>())?;
        let limit = self.least.get(driver)? + REF_COAST_MARGIN_S;
        let lico: Vec<LapLico> = self
            .recent
            .iter()
            .filter(|t| &t.driver == driver && t.coast_s > limit)
            .filter_map(|t| analyse(t, &r))
            .filter(|l| l.extra_coast_s >= LICO_STEP_S)
            .collect();
        let usable: Vec<LapLico> = lico.iter().filter(|l| l.usable).cloned().collect();
        let used = &usable[usable.len().saturating_sub(COST_LAPS)..];
        if used.is_empty() {
            return None;
        }
        // Zones in traffic over the laps measured, those left out whole included.
        let traffic_zones = lico
            .iter()
            .filter(|l| l.lap >= used[0].lap)
            .map(|l| l.zones.iter().filter(|z| z.traffic).count() as u32)
            .sum();
        let s_per_lap = median(used.iter().map(|l| l.cost_s).collect());
        let fuel_saved_per_lap = used
            .iter()
            .map(|l| l.fuel_saved)
            .collect::<Option<Vec<f64>>>()
            .map(median);
        Some(LicoCost {
            s_per_lap,
            extra_coast_s: median(used.iter().map(|l| l.extra_coast_s).collect()),
            fuel_saved_per_lap,
            s_per_litre: fuel_saved_per_lap.filter(|f| *f > 0.05).map(|f| s_per_lap / f),
            laps: used.len() as u32,
            first_lap: used[0].lap,
            last_lap: used[used.len() - 1].lap,
            ref_laps: r.laps,
            ref_coast_s: r.coast_s,
            traffic_zones,
            corners: corners(used),
            driver: driver.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const L: f64 = 3000.0;
    const HZ: f64 = 20.0;

    /// Where a car is on the test track, a circle of length `L`, `off` metres
    /// outside the line.
    fn place(s: f64, off: f64) -> (f64, f64) {
        let r = L / (2.0 * std::f64::consts::PI) + off;
        let a = 2.0 * std::f64::consts::PI * s / L;
        (r * a.cos(), r * a.sin())
    }

    /// A lap driven over a speed profile `v(s)`: samples at `HZ`, the car
    /// moved by its speed. `coast(s)` and `traffic(s)` say what the driver
    /// does where. Returns the samples and the time the lap ends.
    fn drive(t0: f64, v: impl Fn(f64) -> f64, coast: impl Fn(f64) -> bool, traffic: impl Fn(f64) -> bool, fuel0: Option<f64>) -> (Vec<Sample>, f64) {
        drive_line(t0, v, coast, traffic, fuel0, |_| 0.0)
    }

    /// The same on a line `off(s)` metres outside the normal one: the car
    /// covers more ground, so its speed reads higher than its progress along
    /// the track.
    fn drive_line(
        t0: f64,
        v: impl Fn(f64) -> f64,
        coast: impl Fn(f64) -> bool,
        traffic: impl Fn(f64) -> bool,
        fuel0: Option<f64>,
        off: impl Fn(f64) -> f64,
    ) -> (Vec<Sample>, f64) {
        let radius = L / (2.0 * std::f64::consts::PI);
        let dt = 1.0 / HZ;
        let (mut s, mut t, mut out) = (0.0, t0, Vec::new());
        // Fine steps for the motion, a sample every `dt`.
        let sub = 20;
        let mut next = t0;
        let mut fuel = fuel0;
        while s < L {
            if t >= next - 1e-9 {
                out.push(Sample {
                    et: t,
                    speed: v(s) * (radius + off(s)) / radius,
                    coasting: coast(s),
                    excluded: false,
                    traffic: traffic(s),
                    fuel,
                    pos: Some(place(s, off(s))),
                });
                next += dt;
            }
            let h = dt / sub as f64;
            let speed = v(s);
            s += speed * h;
            t += h;
            if let Some(f) = fuel.as_mut() {
                if !coast(s) {
                    *f -= 0.001 * h * speed; // 1 L per km at throttle
                }
            }
        }
        // Back off to the moment the line was crossed.
        let over = (s - L) / v(L);
        (out, t - over)
    }

    /// 60 m/s, with a corner at 1000 m: down to 30 m/s at the apex, rounded
    /// as on track, from 700 to 1300 m.
    fn normal(s: f64) -> f64 {
        let x = (s - 1000.0).abs();
        if x < 300.0 { 30.0 + 30.0 * (x / 300.0).powi(2) } else { 60.0 }
    }
    /// The same with lift-and-coast before the corner: from 500 m the car
    /// loses speed, up to 8 m/s at 700 m where the normal lap brakes, and is
    /// back to normal at the apex.
    fn lico(s: f64) -> f64 {
        let base = normal(s);
        let deficit = if (500.0..700.0).contains(&s) {
            8.0 * (s - 500.0) / 200.0
        } else if (700.0..1000.0).contains(&s) {
            8.0 * (1000.0 - s) / 300.0
        } else {
            0.0
        };
        base - deficit
    }
    fn coast_normal(s: f64) -> bool {
        (680.0..700.0).contains(&s)
    }
    fn coast_lico(s: f64) -> bool {
        (500.0..700.0).contains(&s)
    }
    fn none(_: f64) -> bool {
        false
    }
    /// Time to drive the lap at `v`, finely.
    fn lap_time(v: impl Fn(f64) -> f64) -> f64 {
        let k = 300_000;
        (0..k).map(|i| (L / k as f64) / v((i as f64 + 0.5) * L / k as f64)).sum()
    }

    fn trace(lap: i64, t0: f64, v: impl Fn(f64) -> f64, coast: impl Fn(f64) -> bool, traffic: impl Fn(f64) -> bool) -> Trace {
        let (samples, end) = drive(t0, v, coast, traffic, Some(80.0));
        build_trace(lap, "Alice", &samples, t0, end, L, None).unwrap()
    }

    #[test]
    fn a_lap_on_the_grid_keeps_its_time() {
        let (samples, end) = drive(100.0, |_| 50.0, none, none, None);
        let t = build_trace(1, "Alice", &samples, 100.0, end, L, None).unwrap();
        assert_eq!(t.dt.len(), 300);
        assert!((t.dt.iter().sum::<f64>() - 60.0).abs() < 1e-6);
        assert!(t.dt.iter().all(|d| (d - 0.2).abs() < 1e-3));
        assert!(t.fuel.is_none());
        assert!(t.clean);
    }

    #[test]
    fn a_gap_or_a_wrong_distance_drops_the_lap() {
        let (mut samples, end) = drive(0.0, |_| 50.0, none, none, None);
        assert!(build_trace(1, "A", &samples, 0.0, end, 2000.0, None).is_none(), "distance off by half");
        samples.retain(|s| !(10.0..12.0).contains(&s.et));
        assert!(build_trace(1, "A", &samples, 0.0, end, L, None).is_none(), "2 s without telemetry");
    }

    #[test]
    fn the_cost_is_the_time_lost_in_the_zone() {
        let refs: Vec<Trace> = (0..3).map(|i| trace(i, 100.0 * i as f64, normal, coast_normal, none)).collect();
        let r = reference(&refs.iter().collect::<Vec<_>>()).unwrap();
        let lap = trace(3, 400.0, lico, coast_lico, none);
        let a = analyse(&lap, &r).unwrap();
        let expected = lap_time(lico) - lap_time(normal);
        assert!(expected > 0.2, "the profiles differ by a measurable amount: {expected}");
        assert_eq!(a.zones.len(), 1);
        let z = &a.zones[0];
        assert!((z.start_m - 500.0).abs() <= GRID_M, "zone starts where the driver lifts: {}", z.start_m);
        assert!((1100.0..=1140.0).contains(&z.end_m), "and ends at the corner exit: {}", z.end_m);
        assert!((a.cost_s - expected).abs() < 0.02, "cost {} expected {}", a.cost_s, expected);
        assert!((a.extra_coast_s - lap_time_between(lico, 500.0, 680.0)).abs() < 0.1);
        // 180 m less at throttle, at 1 L/km.
        assert!((a.fuel_saved.unwrap() - 0.18).abs() < 0.01, "fuel {:?}", a.fuel_saved);
        assert!(a.usable);
    }

    fn lap_time_between(v: impl Fn(f64) -> f64, a: f64, b: f64) -> f64 {
        let k = 10_000;
        (0..k).map(|i| ((b - a) / k as f64) / v(a + (i as f64 + 0.5) * (b - a) / k as f64)).sum()
    }

    #[test]
    fn a_slow_moment_elsewhere_does_not_count() {
        let refs: Vec<Trace> = (0..3).map(|i| trace(i, 100.0 * i as f64, normal, coast_normal, none)).collect();
        let r = reference(&refs.iter().collect::<Vec<_>>()).unwrap();
        // Same lift-and-coast, plus a mistake at 2000-2300 m costing time.
        let mistake = |s: f64| if (2000.0..2300.0).contains(&s) { lico(s) - 15.0 } else { lico(s) };
        let lap = trace(3, 400.0, mistake, coast_lico, none);
        let a = analyse(&lap, &r).unwrap();
        let expected = lap_time(lico) - lap_time(normal);
        assert!(lap.lap_time - refs[0].lap_time > expected + 1.0, "the lap is much slower");
        assert!((a.cost_s - expected).abs() < 0.03, "but the cost is the zone's: {} vs {}", a.cost_s, expected);
    }

    #[test]
    fn laps_are_placed_by_where_the_car_is() {
        // The path from a normal lap, then normal laps placed on it.
        let (s0, e0) = drive(0.0, normal, coast_normal, none, None);
        let path = make_path(&s0, 0.0, e0, L).unwrap();
        let refs: Vec<Trace> = (1..4)
            .map(|i| {
                let t0 = 100.0 * i as f64;
                let (s, e) = drive(t0, normal, coast_normal, none, Some(80.0));
                build_trace(i, "Alice", &s, t0, e, L, Some(&path)).unwrap()
            })
            .collect();
        let r = reference(&refs.iter().collect::<Vec<_>>()).unwrap();
        // Lift-and-coast on a line 3 m wide up to the corner: the car covers
        // 6 m more ground, which the speed counts as track.
        let wide = |s: f64| if s < 1000.0 { 3.0 } else { 0.0 };
        let (s1, e1) = drive_line(400.0, lico, coast_lico, none, Some(80.0), wide);
        let expected = lap_time(lico) - lap_time(normal);
        let off = |path: Option<&Path>| (analyse(&build_trace(4, "Alice", &s1, 400.0, e1, L, path).unwrap(), &r).unwrap().cost_s - expected).abs();
        assert!(off(None) > 0.05, "by the speed the line counts: off by {}", off(None));
        assert!(off(Some(&path)) < 0.02, "by position it does not: off by {}", off(Some(&path)));
        // Away from the path (the pit lane, an excursion): not used.
        let (s2, e2) = drive_line(600.0, normal, coast_normal, none, None, |s| if (2000.0..2100.0).contains(&s) { 40.0 } else { 0.0 });
        assert!(build_trace(5, "Alice", &s2, 600.0, e2, L, Some(&path)).is_none());
        // Without positions, by the speed.
        let mut s3 = s2.clone();
        s3.iter_mut().for_each(|s| s.pos = None);
        assert!(build_trace(5, "Alice", &s3, 600.0, e2, L, Some(&path)).is_some());
    }

    #[test]
    fn a_lap_slower_everywhere_is_compared_at_its_own_pace() {
        let refs: Vec<Trace> = (0..3).map(|i| trace(i, 100.0 * i as f64, normal, coast_normal, none)).collect();
        let r = reference(&refs.iter().collect::<Vec<_>>()).unwrap();
        // Worn tyres: 1.5 % slower all lap, on top of the lift-and-coast.
        let worn = |s: f64| lico(s) / 1.015;
        let lap = trace(3, 400.0, worn, coast_lico, none);
        let a = analyse(&lap, &r).unwrap();
        let expected = (lap_time(lico) - lap_time(normal)) * 1.015;
        let uncorrected: f64 = {
            let z = &a.zones[0];
            let (i, j) = ((z.start_m / lap.step_m) as usize, (z.end_m / lap.step_m) as usize);
            (i..j).map(|m| lap.dt[m] - r.dt[m]).sum()
        };
        assert!(uncorrected - expected > 0.1, "without the pace the tyres would count: {uncorrected} vs {expected}");
        assert!((a.cost_s - expected).abs() < 0.02, "cost {} expected {}", a.cost_s, expected);
    }

    #[test]
    fn traffic_drops_the_zone() {
        let refs: Vec<Trace> = (0..3).map(|i| trace(i, 100.0 * i as f64, normal, coast_normal, none)).collect();
        let r = reference(&refs.iter().collect::<Vec<_>>()).unwrap();
        let lap = trace(3, 400.0, lico, coast_lico, |s| (800.0..850.0).contains(&s));
        let a = analyse(&lap, &r).unwrap();
        assert!(a.zones[0].traffic);
        assert!(!a.usable, "its only zone was in traffic");
    }

    /// How a lap is driven: speed and coasting by distance.
    type Driving = (fn(f64) -> f64, fn(f64) -> bool);

    fn tracker_with(laps: &[Driving]) -> ZoneTracker {
        let mut z = ZoneTracker::default();
        let mut t = 0.0;
        for (n, (v, c)) in laps.iter().enumerate() {
            let (samples, end) = drive(t, v, c, none, Some(80.0));
            for s in samples {
                z.on_sample(t, s, L, n as i64, "Alice");
            }
            t = end;
        }
        // The next lap's first sample closes the last one.
        let first = Sample { et: t, speed: 60.0, coasting: false, excluded: false, traffic: false, fuel: Some(70.0), pos: Some(place(0.0, 0.0)) };
        z.on_sample(t, first, L, laps.len() as i64, "Alice");
        z
    }

    #[test]
    fn the_tracker_compares_lico_laps_with_normal_ones() {
        let n: Driving = (normal, coast_normal);
        let l: Driving = (lico, coast_lico);
        assert!(tracker_with(&[n, n, n]).summary().is_none(), "no lift-and-coast yet");
        assert!(tracker_with(&[n, l, l]).summary().is_none(), "one normal lap is not enough");
        let c = tracker_with(&[n, n, n, l, l]).summary().unwrap();
        let expected = lap_time(lico) - lap_time(normal);
        assert_eq!(c.laps, 2);
        assert_eq!((c.first_lap, c.last_lap), (4, 5));
        assert_eq!(c.ref_laps, 3);
        assert!((c.s_per_lap - expected).abs() < 0.03);
        assert!((c.fuel_saved_per_lap.unwrap() - 0.18).abs() < 0.01);
        assert!((c.s_per_litre.unwrap() - expected / 0.18).abs() < 0.3);
        assert_eq!(c.corners.len(), 1);
        assert_eq!(c.corners[0].laps, 2);
        assert_eq!(c.driver, "Alice");
    }

    #[test]
    fn a_lap_in_the_pits_is_not_compared() {
        let (mut samples, end) = drive(0.0, normal, coast_normal, none, None);
        samples[10].excluded = true;
        let t = build_trace(1, "A", &samples, 0.0, end, L, None).unwrap();
        assert!(!t.clean);
        let mut z = ZoneTracker::default();
        z.add(t);
        assert!(z.recent.is_empty());
    }
}
