//! Wheel lock-up tracking.
//!
//! LMU does not publish flat spots. What it does publish, at 100 Hz, is how
//! fast each wheel turns and how fast the car moves — and a flat spot is what
//! a wheel gets from sliding instead of turning. So this module watches for
//! lock-ups and keeps a per-wheel tally for the life of each tyre: how many
//! times it locked, for how long in total, and the longest single slide. The
//! dashboard turns that into a "flat spot risk" hint. It is an inference from
//! the driver's braking, not a reading of the tyre: a long lock-up makes a
//! flat spot likely, it does not prove one.
//!
//! A wheel counts as locked when the car is braking above walking pace and the
//! wheel's rim speed is well below the car's speed. Short blips are ignored:
//! ABS, kerbs and a wheel in the air all produce a few slow samples.

use crate::shared_memory::types::rF2VehicleTelemetry;

/// Below this the car is manoeuvring in the pits or stopped: no lock-ups.
const MIN_SPEED_MS: f64 = 12.0;
/// Brake input needed for a slow wheel to count as locked rather than spun.
const MIN_BRAKE: f64 = 0.10;
/// A wheel turning slower than this fraction of the car's speed is locked.
const LOCKED_SPEED_RATIO: f64 = 0.35;
/// A slide shorter than this is not counted (ABS pulses, kerbs, airborne).
const MIN_LOCKUP_S: f64 = 0.08;
/// A jump in tread this large means a new tyre was fitted.
const NEW_TYRE_WEAR_JUMP: f64 = 0.02;
/// Used when the game reports no radius.
const DEFAULT_RADIUS_M: f64 = 0.34;

/// Lock-up tally for one wheel, since that tyre was fitted.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LockupStats {
    /// Lock-ups counted.
    pub count: u32,
    /// Total time spent locked, seconds.
    pub total_s: f64,
    /// Longest single lock-up, seconds.
    pub longest_s: f64,
    /// True while the wheel is locked right now.
    pub locked_now: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct WheelState {
    stats: LockupStats,
    /// Duration of the slide in progress, seconds; 0 when turning.
    current_s: f64,
    /// Whether the slide in progress has already been counted.
    counted: bool,
    /// Tread remaining at the previous sample, to spot a tyre change.
    last_wear: f64,
}

/// Tracks lock-ups for the four wheels (FL, FR, RL, RR) of the player's car.
#[derive(Debug, Default)]
pub struct LockupTracker {
    wheels: [WheelState; 4],
    last_et: f64,
}

impl LockupTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything (new session, different car).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn stats(&self) -> [LockupStats; 4] {
        [0, 1, 2, 3].map(|i| self.wheels[i].stats)
    }

    /// Feed one telemetry sample of the player's car.
    pub fn update(&mut self, veh: &rF2VehicleTelemetry) {
        let lv = veh.mLocalVel;
        let speed = (lv.x * lv.x + lv.y * lv.y + lv.z * lv.z).sqrt();
        let wheels = [0usize, 1, 2, 3].map(|i| {
            let w = veh.mWheels[i];
            WheelSample {
                rotation_rad_s: w.mRotation,
                radius_m: w.mStaticUndeflectedRadius as f64 / 100.0,
                wear: w.mWear,
            }
        });
        self.update_sample(veh.mElapsedTime, speed, veh.mFilteredBrake, wheels);
    }

    /// The same, on plain numbers (what the tests drive).
    pub fn update_sample(&mut self, et: f64, speed_ms: f64, brake: f64, wheels: [WheelSample; 4]) {
        let dt = et - self.last_et;
        // The clock going backwards is a new session; a long gap is a pause or
        // a teleport to the pits. Neither is driving time, so no slide spans it.
        let continuous = dt > 0.0 && dt < 0.5;
        if dt < 0.0 {
            self.reset();
        }
        self.last_et = et;

        for (state, w) in self.wheels.iter_mut().zip(wheels) {
            // A fresh tyre starts with a clean record.
            if w.wear > state.last_wear + NEW_TYRE_WEAR_JUMP && state.last_wear > 0.0 {
                *state = WheelState::default();
            }
            state.last_wear = w.wear;

            let radius = if w.radius_m > 0.1 { w.radius_m } else { DEFAULT_RADIUS_M };
            let rim_speed = w.rotation_rad_s.abs() * radius;
            let locked = speed_ms > MIN_SPEED_MS
                && brake > MIN_BRAKE
                && rim_speed < speed_ms * LOCKED_SPEED_RATIO;

            if locked && continuous {
                state.current_s += dt;
                if state.current_s >= MIN_LOCKUP_S {
                    if !state.counted {
                        state.counted = true;
                        state.stats.count += 1;
                        // The time before the threshold was part of the slide.
                        state.stats.total_s += state.current_s;
                    } else {
                        state.stats.total_s += dt;
                    }
                    state.stats.longest_s = state.stats.longest_s.max(state.current_s);
                    state.stats.locked_now = true;
                }
            } else {
                state.current_s = 0.0;
                state.counted = false;
                state.stats.locked_now = false;
            }
        }
    }
}

/// What the tracker needs from one wheel.
#[derive(Debug, Clone, Copy)]
pub struct WheelSample {
    pub rotation_rad_s: f64,
    pub radius_m: f64,
    /// Tread remaining, 1.0 = new.
    pub wear: f64,
}

/// LMU's `mOptimalTemp` in °C, or `None` when the game gives nothing usable.
///
/// The header does not state the unit. Every other temperature in the wheel
/// block is Kelvin, so a value in the Kelvin range for a racing tyre is
/// converted and one already in a plausible Celsius range is kept. Anything
/// else is discarded rather than guessed at.
pub fn optimal_temp_c(raw: f32) -> Option<f64> {
    let v = raw as f64;
    if !v.is_finite() {
        None
    } else if (300.0..=450.0).contains(&v) {
        Some(v - 273.15)
    } else if (30.0..=180.0).contains(&v) {
        Some(v)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 0.02; // 50 Hz, the bridge's polling rate
    const R: f64 = 0.34;

    /// Wheel turning at `ratio` × the car's speed.
    fn wheel(speed: f64, ratio: f64, wear: f64) -> WheelSample {
        WheelSample { rotation_rad_s: speed * ratio / R, radius_m: R, wear }
    }

    /// Run `seconds` of samples with the front-left at `fl_ratio`, others rolling.
    fn run(t: &mut LockupTracker, et: &mut f64, seconds: f64, speed: f64, brake: f64, fl_ratio: f64) {
        let n = (seconds / DT).round() as usize;
        for _ in 0..n {
            *et += DT;
            let rolling = wheel(speed, 1.0, 0.9);
            t.update_sample(*et, speed, brake, [wheel(speed, fl_ratio, 0.9), rolling, rolling, rolling]);
        }
    }

    #[test]
    fn normal_braking_is_not_a_lockup() {
        let (mut t, mut et) = (LockupTracker::new(), 100.0);
        run(&mut t, &mut et, 2.0, 60.0, 0.9, 0.92); // 8 % slip: hard, healthy braking
        assert_eq!(t.stats(), [LockupStats::default(); 4]);
    }

    #[test]
    fn a_locked_front_is_counted_once_with_its_duration() {
        let (mut t, mut et) = (LockupTracker::new(), 100.0);
        run(&mut t, &mut et, 0.5, 60.0, 0.9, 1.0);
        run(&mut t, &mut et, 0.4, 60.0, 0.9, 0.0); // locked for 0.4 s
        assert!(t.stats()[0].locked_now);
        run(&mut t, &mut et, 0.5, 60.0, 0.3, 1.0);

        let fl = t.stats()[0];
        assert_eq!(fl.count, 1);
        assert!((fl.total_s - 0.4).abs() < 0.03, "{fl:?}");
        assert!((fl.longest_s - 0.4).abs() < 0.03, "{fl:?}");
        assert!(!fl.locked_now);
        // The other three wheels kept turning.
        assert_eq!(t.stats()[1], LockupStats::default());
    }

    #[test]
    fn separate_lockups_add_up_and_the_longest_is_kept() {
        let (mut t, mut et) = (LockupTracker::new(), 100.0);
        for secs in [0.2, 0.6, 0.3] {
            run(&mut t, &mut et, secs, 50.0, 0.8, 0.1);
            run(&mut t, &mut et, 1.0, 50.0, 0.0, 1.0);
        }
        let fl = t.stats()[0];
        assert_eq!(fl.count, 3);
        assert!((fl.total_s - 1.1).abs() < 0.06, "{fl:?}");
        assert!((fl.longest_s - 0.6).abs() < 0.03, "{fl:?}");
    }

    #[test]
    fn short_blips_slow_speed_and_no_brake_are_ignored() {
        let (mut t, mut et) = (LockupTracker::new(), 100.0);
        run(&mut t, &mut et, 0.04, 60.0, 0.9, 0.0); // two samples: ABS pulse
        run(&mut t, &mut et, 0.5, 60.0, 0.9, 1.0);
        run(&mut t, &mut et, 1.0, 5.0, 1.0, 0.0); // stopped wheel in the pit box
        run(&mut t, &mut et, 1.0, 60.0, 0.0, 0.0); // slow wheel, no brake: not a lock-up
        assert_eq!(t.stats()[0], LockupStats::default());
    }

    #[test]
    fn a_new_tyre_clears_that_wheels_record() {
        let (mut t, mut et) = (LockupTracker::new(), 100.0);
        run(&mut t, &mut et, 0.3, 60.0, 0.9, 0.0);
        assert_eq!(t.stats()[0].count, 1);

        // Pit stop: front-left goes from 0.9 back to 1.0.
        et += DT;
        let fresh = wheel(0.0, 1.0, 1.0);
        let old = wheel(0.0, 1.0, 0.9);
        t.update_sample(et, 0.0, 1.0, [fresh, old, old, old]);
        assert_eq!(t.stats()[0], LockupStats::default());
    }

    #[test]
    fn a_pause_does_not_become_one_long_lockup() {
        let (mut t, mut et) = (LockupTracker::new(), 100.0);
        run(&mut t, &mut et, 0.1, 60.0, 0.9, 0.0);
        et += 30.0; // game paused with the wheel locked
        run(&mut t, &mut et, 0.02, 60.0, 0.9, 0.0);
        assert!(t.stats()[0].longest_s < 0.2, "{:?}", t.stats()[0]);
    }

    #[test]
    fn optimal_temperature_unit_is_normalised() {
        assert!((optimal_temp_c(363.15).unwrap() - 90.0).abs() < 0.01); // Kelvin
        assert_eq!(optimal_temp_c(95.0), Some(95.0)); // already Celsius
        assert_eq!(optimal_temp_c(0.0), None);
        assert_eq!(optimal_temp_c(f32::NAN), None);
        assert_eq!(optimal_temp_c(1.0e9), None);
    }
}
