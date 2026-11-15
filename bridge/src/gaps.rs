//! Race gap between two cars.
//!
//! The gap to the leader cannot be used: LMU sets `mTimeBehindLeader` to 0
//! for every car the leader has lapped, leaving only `mLapsBehindLeader`, so
//! two lapped cars on the same lap read 0 s apart (seen live, 2026-11-15).
//! Instead the gap is summed along the order from each car's gap to the car
//! one place ahead (`mTimeBehindNext` / `mLapsBehindNext`), which stays valid.

/// What the gap needs from one car.
#[derive(Debug, Clone, Copy)]
pub struct Link {
    /// Overall position, 1-based.
    pub place: i64,
    pub time_behind_next: f64,
    pub laps_behind_next: i64,
}

/// How far apart two cars are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gap {
    Time(f64),
    Laps(i64),
}

/// The gap between the cars at places `a` and `b`, in either order. `None`
/// when a place in between is missing from `cars`.
pub fn between(cars: &[Link], a: i64, b: i64) -> Option<Gap> {
    let (front, back) = if a <= b { (a, b) } else { (b, a) };
    let mut secs = 0.0;
    let mut laps = 0;
    for p in front + 1..=back {
        let c = cars.iter().find(|c| c.place == p)?;
        if c.laps_behind_next > 0 {
            laps += c.laps_behind_next;
        } else {
            // Slightly negative values happen at timing points.
            secs += c.time_behind_next.max(0.0);
        }
    }
    Some(if laps > 0 { Gap::Laps(laps) } else { Gap::Time(secs) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(place: i64, t: f64, laps: i64) -> Link {
        Link { place, time_behind_next: t, laps_behind_next: laps }
    }

    #[test]
    fn summed_along_the_order() {
        // Places 20–23 as read live: all lapped, time to the leader 0 for each.
        let cars = [l(20, 17.39, 0), l(21, 0.43, 0), l(22, 9.15, 0), l(23, 46.97, 0)];
        assert_eq!(between(&cars, 21, 22), Some(Gap::Time(9.15)));
        assert_eq!(between(&cars, 21, 20), Some(Gap::Time(0.43)));
        match between(&cars, 21, 23) {
            Some(Gap::Time(t)) => assert!((t - 56.12).abs() < 1e-9),
            g => panic!("{g:?}"),
        }
        assert_eq!(between(&cars, 22, 22), Some(Gap::Time(0.0)));
    }

    #[test]
    fn a_lap_in_between_makes_it_laps() {
        let cars = [l(1, 0.0, 0), l(2, 0.0, 1), l(3, 4.0, 0)];
        assert_eq!(between(&cars, 1, 3), Some(Gap::Laps(1)));
    }

    #[test]
    fn a_missing_place_gives_nothing() {
        let cars = [l(1, 0.0, 0), l(3, 4.0, 0)];
        assert_eq!(between(&cars, 1, 3), None);
    }
}
