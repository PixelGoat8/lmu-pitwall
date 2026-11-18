//! The hub's decisions, as plain functions over plain data: what a bridge's
//! messages say about its driver, which bridge to follow, and where one stint
//! ends and the next begins. No sockets, no clocks — everything here is driven
//! by the session time carried in the messages, so it runs the same on a live
//! race and on a replay, and it is tested without either.

use rmpv::Value;

use crate::recording::field;

/// A number out of a raw message, whatever width it was encoded with. JSON
/// recordings turn `30.0` into the integer `30`; live MessagePack does not.
pub fn num(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_i64().map(|i| i as f64))
        .or_else(|| v.as_u64().map(|u| u as f64))
}

fn get_num(msg: &Value, key: &str) -> Option<f64> {
    field(msg, key).and_then(num)
}
fn get_str(msg: &Value, key: &str) -> String {
    field(msg, key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}
fn get_bool(msg: &Value, key: &str) -> bool {
    field(msg, key).and_then(|v| v.as_bool()).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// What one ScoringUpdate says about the car this bridge belongs to
// ---------------------------------------------------------------------------

/// The player's row of a `ScoringUpdate`, reduced to what the hub needs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScoringSummary {
    pub session_type: String,
    pub session_time: f64,
    /// Who is playing on the bridge's PC (`mPlayerName`).
    pub pc_player: String,
    /// The slot of the player's car, and who the game says is driving it now.
    pub car_id: i64,
    /// The car as the game names it ("Iron Lynx #63").
    pub car_name: String,
    pub driver_name: String,
    /// `mControl` of that car; `None` from a bridge too old to send it.
    pub control: Option<i64>,
    pub in_pits: bool,
    pub total_laps: i64,
    pub last_lap_time: f64,
    pub virtual_energy: f64,
}

impl ScoringSummary {
    pub fn from_msg(msg: &Value) -> Option<Self> {
        let player_id = field(msg, "player_vehicle_id")?.as_i64()?;
        let row = field(msg, "vehicles")?
            .as_array()?
            .iter()
            .find(|v| field(v, "id").and_then(|i| i.as_i64()) == Some(player_id))?;
        Some(ScoringSummary {
            session_type: get_str(msg, "session_type"),
            session_time: get_num(msg, "session_time").unwrap_or(0.0),
            pc_player: get_str(msg, "player_name"),
            car_id: player_id,
            car_name: get_str(row, "car_name"),
            driver_name: get_str(row, "driver_name"),
            control: field(row, "control").and_then(|v| v.as_i64()),
            in_pits: get_bool(row, "in_pits"),
            total_laps: field(row, "total_laps").and_then(|v| v.as_i64()).unwrap_or(0),
            last_lap_time: get_num(row, "last_lap_time").unwrap_or(-1.0),
            virtual_energy: get_num(row, "virtual_energy").unwrap_or(0.0),
        })
    }

    /// True when this bridge's own PC is driving the car: `mControl == 0`.
    ///
    /// The same car seen from a teammate's PC is under remote control (2), and
    /// a car parked in the garage is held by the local AI (1, measured). Only
    /// the driver's own bridge has the full telemetry — fuel, tyres, brakes —
    /// so this is the bridge worth following.
    pub fn driving(&self) -> bool {
        self.control == Some(0)
    }
}

// ---------------------------------------------------------------------------
// Which bridge to follow
// ---------------------------------------------------------------------------

/// One bridge, as the selection sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: String,
    /// Connected and authenticated.
    pub linked: bool,
    pub session_active: bool,
    pub driving: bool,
    /// Session time at which it started driving; orders two "driving" sources.
    pub driving_since: f64,
}

/// Pick the bridge to follow.
///
/// 1. A pinned bridge, if it is linked — the engineer's explicit choice wins.
/// 2. A bridge that is driving. If the one already followed is driving, it
///    stays (no flapping); otherwise the one that took the wheel most recently.
/// 3. Nobody is driving (pit stop, driver swap in progress, everyone in the
///    garage): stay where we are while that bridge still has a session, so
///    the screen does not jump around during a stop.
/// 4. Otherwise any bridge with a session, then any linked bridge.
pub fn choose_source(cands: &[Candidate], current: Option<&str>, pinned: Option<&str>) -> Option<String> {
    let by_id = |id: &str| cands.iter().find(|c| c.id == id);

    if let Some(p) = pinned.and_then(by_id) {
        if p.linked {
            return Some(p.id.clone());
        }
    }

    let cur = current.and_then(by_id);
    if let Some(c) = cur {
        if c.linked && c.driving {
            return Some(c.id.clone());
        }
    }
    let newest_driver = cands
        .iter()
        .filter(|c| c.linked && c.driving)
        .max_by(|a, b| a.driving_since.total_cmp(&b.driving_since));
    if let Some(d) = newest_driver {
        return Some(d.id.clone());
    }

    if let Some(c) = cur {
        if c.linked && c.session_active {
            return Some(c.id.clone());
        }
    }
    cands
        .iter()
        .find(|c| c.linked && c.session_active)
        .or_else(|| cands.iter().find(|c| c.linked))
        .map(|c| c.id.clone())
}

// ---------------------------------------------------------------------------
// Stints
// ---------------------------------------------------------------------------

/// One stint: a driver's uninterrupted run between two pit stops.
#[derive(Debug, Clone, PartialEq)]
pub struct Stint {
    pub number: u32,
    pub driver: String,
    /// Label of the bridge the data came from.
    pub source: String,
    pub start_time: f64,
    /// `None` while the stint is running.
    pub end_time: Option<f64>,
    pub start_lap: i64,
    pub end_lap: i64,
    /// Times of the laps completed in this stint, in order.
    pub lap_times: Vec<f64>,
    pub fuel_start: Option<f64>,
    pub fuel_end: Option<f64>,
    pub ve_start: Option<f64>,
    pub ve_end: Option<f64>,
}

impl Stint {
    pub fn best_lap(&self) -> Option<f64> {
        self.lap_times.iter().copied().filter(|t| *t > 0.0).reduce(f64::min)
    }

    /// Average of the stint's laps, leaving out the first (the out-lap) once
    /// there are enough laps for that not to throw most of the stint away.
    pub fn avg_lap(&self) -> Option<f64> {
        let valid: Vec<f64> = self.lap_times.iter().copied().filter(|t| *t > 0.0).collect();
        let laps = if valid.len() >= 3 { &valid[1..] } else { &valid[..] };
        (!laps.is_empty()).then(|| laps.iter().sum::<f64>() / laps.len() as f64)
    }
}

/// A fuel reading this much above the previous one is a refuel, not noise.
const REFILL_MARGIN_L: f64 = 0.5;
/// The same for virtual energy (fraction of the tank).
const REFILL_MARGIN_VE: f64 = 0.005;

/// Builds the stint history of the followed car from its scoring summaries.
#[derive(Debug, Default)]
pub struct StintTracker {
    stints: Vec<Stint>,
    /// Index of the running stint in `stints`.
    open: Option<usize>,
    last: Option<ScoringSummary>,
    /// Latest trustworthy fuel reading, litres (only from a driving bridge).
    fuel: Option<f64>,
}

impl StintTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stints(&self) -> &[Stint] {
        &self.stints
    }

    /// Fuel level from the followed bridge's telemetry. Call only while that
    /// bridge is the one driving: from anywhere else the game reports a
    /// placeholder, which would poison the stint's consumption.
    pub fn on_fuel(&mut self, litres: f64) {
        self.fuel = Some(litres);
        if let Some(st) = self.open.and_then(|i| self.stints.get_mut(i)) {
            if st.fuel_start.is_none() {
                st.fuel_start = Some(litres);
            }
            // A rise is a refuel. It can show up a moment before the car is
            // flagged in the pits, and it belongs to the next stint, so it
            // must not overwrite where this one ended.
            match st.fuel_end {
                Some(end) if litres > end + REFILL_MARGIN_L => {}
                _ => st.fuel_end = Some(litres),
            }
        }
    }

    /// Feed one scoring summary of the followed car. Returns true when the
    /// stint list changed in a way worth sending to clients.
    pub fn on_scoring(&mut self, source: &str, s: &ScoringSummary) -> bool {
        let mut changed = false;

        // A new session (clock jumped back, or the session type changed)
        // starts a new history.
        if let Some(last) = &self.last {
            if s.session_time < last.session_time - 5.0 || s.session_type != last.session_type {
                *self = Self::default();
                changed = true;
            }
        }
        let was_in_pits = self.last.as_ref().map(|l| l.in_pits).unwrap_or(true);
        let prev_laps = self.last.as_ref().map(|l| l.total_laps).unwrap_or(s.total_laps);

        if let Some(i) = self.open {
            let st = &mut self.stints[i];
            let driver_changed = !s.driver_name.is_empty() && s.driver_name != st.driver;

            // Laps completed since the last summary — unless the driver
            // changed, in which case the lap count may be another car's.
            if !driver_changed && s.total_laps > prev_laps {
                if s.last_lap_time > 0.0 {
                    st.lap_times.push(s.last_lap_time);
                }
                st.end_lap = s.total_laps;
                changed = true;
            }
            // As for fuel: a rise is a refill for the next stint.
            if s.virtual_energy > 0.0 && st.ve_end.map(|e| s.virtual_energy <= e + REFILL_MARGIN_VE).unwrap_or(true) {
                st.ve_end = Some(s.virtual_energy);
            }

            let entered_pits = s.in_pits && !was_in_pits;
            if driver_changed || entered_pits {
                st.end_time = Some(s.session_time);
                self.open = None;
                changed = true;
            }
        }

        // A stint starts when the car is out of the pits and none is running:
        // the first time we see it on track, at a pit exit, or right after a
        // driver change away from the pit lane.
        if self.open.is_none() && !s.in_pits && !s.driver_name.is_empty() {
            self.stints.push(Stint {
                number: self.stints.len() as u32 + 1,
                driver: s.driver_name.clone(),
                source: source.to_string(),
                start_time: s.session_time,
                end_time: None,
                start_lap: s.total_laps,
                end_lap: s.total_laps,
                lap_times: Vec::new(),
                // The reading in hand predates this stint (it may be the
                // level before refuelling), so wait for the next one.
                fuel_start: None,
                fuel_end: None,
                ve_start: (s.virtual_energy > 0.0).then_some(s.virtual_energy),
                ve_end: (s.virtual_energy > 0.0).then_some(s.virtual_energy),
            });
            self.open = Some(self.stints.len() - 1);
            changed = true;
        }

        self.last = Some(s.clone());
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: &str, linked: bool, session: bool, driving: bool, since: f64) -> Candidate {
        Candidate { id: id.into(), linked, session_active: session, driving, driving_since: since }
    }

    #[test]
    fn the_driving_bridge_is_followed() {
        let c = [cand("a", true, true, false, 0.0), cand("b", true, true, true, 10.0)];
        assert_eq!(choose_source(&c, None, None).as_deref(), Some("b"));
        assert_eq!(choose_source(&c, Some("a"), None).as_deref(), Some("b"));
    }

    #[test]
    fn a_driver_swap_moves_to_the_new_driver() {
        // Alice was followed and has handed over; Bob took the wheel.
        let c = [cand("alice", true, true, false, 0.0), cand("bob", true, true, true, 900.0)];
        assert_eq!(choose_source(&c, Some("alice"), None).as_deref(), Some("bob"));
    }

    #[test]
    fn nobody_driving_keeps_the_current_view() {
        // Mid pit stop: nobody has control. Stay on the bridge we were on.
        let c = [cand("alice", true, true, false, 0.0), cand("bob", true, true, false, 0.0)];
        assert_eq!(choose_source(&c, Some("bob"), None).as_deref(), Some("bob"));
        // With no current choice, any bridge in a session will do.
        assert_eq!(choose_source(&c, None, None).as_deref(), Some("alice"));
    }

    #[test]
    fn two_drivers_do_not_flap() {
        // Friends each in their own car: both "driving". Keep the current one;
        // with no current one, the most recent.
        let c = [cand("a", true, true, true, 50.0), cand("b", true, true, true, 80.0)];
        assert_eq!(choose_source(&c, Some("a"), None).as_deref(), Some("a"));
        assert_eq!(choose_source(&c, None, None).as_deref(), Some("b"));
    }

    #[test]
    fn a_pin_wins_while_its_bridge_is_linked() {
        let mut c = vec![cand("a", true, true, false, 0.0), cand("b", true, true, true, 10.0)];
        assert_eq!(choose_source(&c, Some("b"), Some("a")).as_deref(), Some("a"));
        // The pinned bridge drops: fall back to the normal rules.
        c[0].linked = false;
        assert_eq!(choose_source(&c, Some("a"), Some("a")).as_deref(), Some("b"));
    }

    #[test]
    fn unlinked_bridges_are_never_chosen() {
        let c = [cand("a", false, true, true, 99.0), cand("b", true, false, false, 0.0)];
        assert_eq!(choose_source(&c, Some("a"), None).as_deref(), Some("b"));
        assert_eq!(choose_source(&[cand("a", false, false, false, 0.0)], None, None), None);
        assert_eq!(choose_source(&[], None, None), None);
    }

    // --- scoring summary ----------------------------------------------------

    fn scoring_json(control: Option<i64>) -> Value {
        let mut row = serde_json::json!({
            "id": 2, "driver_name": "Alice", "in_pits": false, "total_laps": 7,
            "last_lap_time": 93.5, "virtual_energy": 0.62
        });
        if let Some(c) = control {
            row["control"] = c.into();
        }
        rmpv::ext::to_value(serde_json::json!({
            "type": "ScoringUpdate", "session_type": "Race", "session_time": 700,
            "player_vehicle_id": 2, "player_name": "Alice PC",
            "vehicles": [{ "id": 1, "driver_name": "Other" }, row]
        }))
        .unwrap()
    }

    #[test]
    fn the_players_row_is_summarised() {
        let s = ScoringSummary::from_msg(&scoring_json(Some(0))).unwrap();
        assert_eq!(s.driver_name, "Alice");
        assert_eq!(s.pc_player, "Alice PC");
        assert_eq!(s.total_laps, 7);
        assert_eq!(s.session_time, 700.0); // an integer in JSON, still read
        assert!(s.driving());
        assert!(!ScoringSummary::from_msg(&scoring_json(Some(2))).unwrap().driving());
        // An older bridge sends no control: unknown is not "driving".
        assert!(!ScoringSummary::from_msg(&scoring_json(None)).unwrap().driving());
    }

    #[test]
    fn a_message_without_a_player_row_gives_no_summary() {
        let msg = rmpv::ext::to_value(serde_json::json!({
            "type": "ScoringUpdate", "player_vehicle_id": -1, "vehicles": [{ "id": 1 }]
        }))
        .unwrap();
        assert!(ScoringSummary::from_msg(&msg).is_none());
    }

    // --- stints -------------------------------------------------------------

    fn s(t: f64, driver: &str, laps: i64, last: f64, in_pits: bool) -> ScoringSummary {
        ScoringSummary {
            session_type: "Race".into(),
            session_time: t,
            driver_name: driver.into(),
            total_laps: laps,
            last_lap_time: last,
            in_pits,
            control: Some(0),
            ..Default::default()
        }
    }

    #[test]
    fn laps_pit_stop_and_driver_swap_make_three_stints() {
        let mut t = StintTracker::new();
        // Alice leaves the pits and does three laps.
        t.on_scoring("alice-pc", &s(0.0, "Alice", 0, -1.0, true));
        assert!(t.stints().is_empty(), "no stint while still in the pits");
        t.on_scoring("alice-pc", &s(5.0, "Alice", 0, -1.0, false));
        t.on_fuel(80.0);
        t.on_scoring("alice-pc", &s(100.0, "Alice", 1, 95.0, false));
        t.on_scoring("alice-pc", &s(192.0, "Alice", 2, 92.0, false));
        t.on_fuel(74.0);
        t.on_scoring("alice-pc", &s(283.0, "Alice", 3, 91.0, false));
        // Pit stop: fuel only, Alice stays in.
        t.on_scoring("alice-pc", &s(290.0, "Alice", 3, 91.0, true));
        t.on_fuel(90.0); // refuelled in the box: belongs to no stint
        t.on_scoring("alice-pc", &s(320.0, "Alice", 3, 91.0, false));
        t.on_fuel(89.5);
        t.on_scoring("alice-pc", &s(420.0, "Alice", 4, 130.0, false));
        // Second stop: Bob takes over, data now from his PC.
        t.on_scoring("alice-pc", &s(430.0, "Alice", 4, 130.0, true));
        t.on_scoring("bob-pc", &s(450.0, "Bob", 4, 130.0, true));
        t.on_scoring("bob-pc", &s(470.0, "Bob", 4, 130.0, false));

        let st = t.stints();
        assert_eq!(st.len(), 3);

        assert_eq!((st[0].driver.as_str(), st[0].source.as_str()), ("Alice", "alice-pc"));
        assert_eq!(st[0].lap_times, vec![95.0, 92.0, 91.0]);
        assert_eq!((st[0].start_lap, st[0].end_lap), (0, 3));
        assert_eq!(st[0].end_time, Some(290.0));
        assert_eq!((st[0].fuel_start, st[0].fuel_end), (Some(80.0), Some(74.0)));
        assert_eq!(st[0].best_lap(), Some(91.0));
        assert_eq!(st[0].avg_lap(), Some(91.5)); // out-lap left out

        assert_eq!(st[1].driver, "Alice");
        assert_eq!(st[1].lap_times, vec![130.0]);
        assert_eq!(st[1].fuel_start, Some(89.5), "the pre-stop level is not this stint's start");
        assert_eq!(st[1].end_time, Some(430.0));

        assert_eq!((st[2].driver.as_str(), st[2].source.as_str()), ("Bob", "bob-pc"));
        assert_eq!(st[2].end_time, None);
        assert_eq!(st[2].number, 3);
    }

    #[test]
    fn a_refill_seen_before_the_pit_flag_does_not_end_the_stint_full() {
        let mut t = StintTracker::new();
        let mut row = s(5.0, "Alice", 0, -1.0, false);
        row.virtual_energy = 0.9;
        t.on_scoring("a", &row);
        t.on_fuel(80.0);
        t.on_fuel(74.0);
        row.virtual_energy = 0.82;
        row.total_laps = 2;
        t.on_scoring("a", &row);
        // Refilled a moment before the car is flagged in the pits.
        t.on_fuel(90.0);
        row.virtual_energy = 1.0;
        row.session_time = 6.0;
        t.on_scoring("a", &row);
        row.in_pits = true;
        t.on_scoring("a", &row);
        let st = &t.stints()[0];
        assert_eq!((st.fuel_start, st.fuel_end), (Some(80.0), Some(74.0)));
        assert_eq!((st.ve_start, st.ve_end), (Some(0.9), Some(0.82)));
    }

    #[test]
    fn a_driver_change_on_track_splits_the_stint() {
        // Seen from a hub following two friends in different cars.
        let mut t = StintTracker::new();
        t.on_scoring("a", &s(10.0, "Alice", 2, 90.0, false));
        t.on_scoring("b", &s(20.0, "Bob", 5, 88.0, false));
        assert_eq!(t.stints().len(), 2);
        assert_eq!(t.stints()[0].end_time, Some(20.0));
        assert_eq!(t.stints()[1].driver, "Bob");
    }

    #[test]
    fn a_new_session_clears_the_history() {
        let mut t = StintTracker::new();
        t.on_scoring("a", &s(500.0, "Alice", 3, 90.0, false));
        assert_eq!(t.stints().len(), 1);
        // The clock restarts: qualifying after practice, or a new race.
        assert!(t.on_scoring("a", &s(3.0, "Alice", 0, -1.0, false)));
        assert_eq!(t.stints().len(), 1);
        assert_eq!(t.stints()[0].start_time, 3.0);
    }

    #[test]
    fn averages_handle_short_stints() {
        let mut st = Stint {
            number: 1, driver: "A".into(), source: "a".into(), start_time: 0.0, end_time: None,
            start_lap: 0, end_lap: 0, lap_times: vec![], fuel_start: None, fuel_end: None,
            ve_start: None, ve_end: None,
        };
        assert_eq!((st.best_lap(), st.avg_lap()), (None, None));
        st.lap_times = vec![100.0, 90.0];
        assert_eq!(st.avg_lap(), Some(95.0)); // too few laps to drop the out-lap
    }
}
