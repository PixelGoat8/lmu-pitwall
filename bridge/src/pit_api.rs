//! The pit-stop side of LMU's REST API, for the strategy engine.
//!
//! Two endpoints, both measured on LMU 1.4.2 (see docs/lmu-api.md):
//!
//! * `/rest/garage/UIScreen/RepairAndRefuel`: fuel and VE in real units, the
//!   car's pit work times, the pit menu as set, the game's recommendations.
//! * `/rest/strategy/pitstop-estimate`: the game's own estimate of the
//!   stationary time for the menu as set, item by item.
//!
//! Both answer only with a session loaded. The result is broadcast as a
//! `PitInfo` message so that a hub, which has no game of its own, gets the
//! driving PC's figures too.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::strategy::model::PitTimes;

const REPAIR_AND_REFUEL_URL: &str = "http://localhost:6397/rest/garage/UIScreen/RepairAndRefuel";
const PITSTOP_ESTIMATE_URL: &str = "http://localhost:6397/rest/strategy/pitstop-estimate";

/// The game's estimate for the pit menu as currently set, seconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PitEstimate {
    pub total: f64,
    pub fuel: f64,
    pub ve: f64,
    pub tires: f64,
    pub driver_swap: f64,
    pub damage: f64,
    pub penalties: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PitInfo {
    /// Litres.
    pub fuel: Option<f64>,
    pub fuel_capacity: Option<f64>,
    /// Fraction 0–1 (`currentVirtualEnergy / maxVirtualEnergy`).
    pub ve: Option<f64>,
    /// The car's pit work times; `None` if the game did not send them.
    pub times: Option<PitTimes>,
    pub estimate: Option<PitEstimate>,
    /// Virtual energy the pit menu is set to add, fraction 0–1.
    pub menu_ve: Option<f64>,
    /// Text of the menu's fuel line as the game shows it.
    pub menu_fuel: Option<String>,
    /// The game's own recommendation, as sent (units not documented).
    pub recommended_fuel: Option<f64>,
    pub recommended_ve: Option<f64>,
}

fn f(v: &Value, path: &[&str]) -> Option<f64> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_f64()
}

fn flag(v: &Value, key: &str) -> bool {
    match v.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(n) => n.as_f64().map(|x| x != 0.0).unwrap_or(false),
        None => false,
    }
}

/// The text of the selected option of a pit menu line, e.g. "25% 7 laps".
fn menu_choice<'a>(v: &'a Value, line: &str) -> Option<&'a str> {
    let item = v
        .get("pitMenu")?
        .get("pitMenu")?
        .as_array()?
        .iter()
        .find(|i| i.get("name").and_then(|n| n.as_str()) == Some(line))?;
    let idx = item.get("currentSetting")?.as_u64()? as usize;
    item.get("settings")?.as_array()?.get(idx)?.get("text")?.as_str()
}

/// The leading percentage of a menu text: "25% 7 laps" → 0.25.
fn leading_percent(text: &str) -> Option<f64> {
    let num: String = text.trim().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let rest = text.trim()[num.len()..].trim_start();
    (rest.starts_with('%')).then(|| num.parse::<f64>().ok()).flatten().map(|p| p / 100.0)
}

pub fn parse_repair_and_refuel(v: &Value) -> PitInfo {
    let fi = v.get("fuelInfo");
    let ve = fi.and_then(|fi| {
        let cur = fi.get("currentVirtualEnergy")?.as_f64()?;
        let max = fi.get("maxVirtualEnergy")?.as_f64()?;
        (max > 0.0).then(|| (cur / max).clamp(0.0, 1.0))
    });
    let times = v.get("pitStopTimes").and_then(|p| p.get("times")).map(|t| PitTimes {
        fuel_fill_rate: f(t, &["FuelFillRate"]).unwrap_or(0.0),
        fuel_insert_s: f(t, &["FuelInsert"]).unwrap_or(0.0),
        fuel_concurrent: flag(t, "FuelTimeConcurrent"),
        ve_fill_rate: f(t, &["virtualEnergyFillRate"]).unwrap_or(0.0),
        ve_insert_s: f(t, &["virtualEnergyInsert"]).unwrap_or(0.0),
        ve_concurrent: flag(t, "virtualEnergyTimeConcurrent"),
        four_tyres_s: f(t, &["FourTireChange"]).unwrap_or(0.0),
        two_tyres_s: f(t, &["TwoTireChange"]).unwrap_or(0.0),
        tyres_concurrent: flag(t, "TireTimeConcurrent"),
        driver_change_s: f(t, &["DriverChange"]).unwrap_or(0.0),
        driver_concurrent: flag(t, "DriverConcurrent"),
    });
    PitInfo {
        fuel: fi.and_then(|x| f(x, &["currentFuel"])),
        fuel_capacity: fi.and_then(|x| f(x, &["maxFuel"])).filter(|c| *c > 0.0),
        ve,
        times,
        estimate: None,
        menu_ve: menu_choice(v, "VIRTUAL ENERGY:").and_then(leading_percent),
        menu_fuel: menu_choice(v, "FUEL:").map(str::to_string),
        recommended_fuel: f(v, &["pitRecommendations", "fuel"]),
        recommended_ve: f(v, &["pitRecommendations", "virtualEnergy"]),
    }
}

pub fn parse_estimate(v: &Value) -> Option<PitEstimate> {
    Some(PitEstimate {
        total: f(v, &["total"])?,
        fuel: f(v, &["fuel"]).unwrap_or(0.0),
        ve: f(v, &["ve"]).unwrap_or(0.0),
        tires: f(v, &["tires"]).unwrap_or(0.0),
        driver_swap: f(v, &["driverSwap"]).unwrap_or(0.0),
        damage: f(v, &["damage"]).unwrap_or(0.0),
        penalties: f(v, &["penalties"]).unwrap_or(0.0),
    })
}

fn get_json(url: &str) -> Option<Value> {
    ureq::get(url)
        .timeout(std::time::Duration::from_secs(2))
        .call()
        .ok()?
        .into_json::<Value>()
        .ok()
}

/// One RepairAndRefuel response, used for both the wearables and `PitInfo`.
pub fn fetch_repair_and_refuel() -> Option<Value> {
    get_json(REPAIR_AND_REFUEL_URL)
}

/// Both endpoints together (blocking, 2 s timeout each).
pub fn fetch_pit_info(repair_and_refuel: &Value) -> PitInfo {
    let mut info = parse_repair_and_refuel(repair_and_refuel);
    info.estimate = get_json(PITSTOP_ESTIMATE_URL).as_ref().and_then(parse_estimate);
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape and values of a real response (LMU 1.4.2, Hypercar, Monza),
    /// names removed and lists shortened.
    fn sample() -> Value {
        serde_json::json!({
            "fuelInfo": {"currentBattery": 606460.1, "currentFuel": 80.09, "currentVirtualEnergy": 867711104.0,
                         "maxBattery": 5400000.0, "maxFuel": 110.0, "maxVirtualEnergy": 900000000.0},
            "pitStopTimes": {"times": {"FourTireChange": 12.0, "TwoTireChange": 4.5, "DriverChange": 25.0,
                "DriverConcurrent": 1.0, "FuelFillRate": 4.0, "FuelInsert": 1.0, "FuelTimeConcurrent": 1.0,
                "TireTimeConcurrent": 0.0, "virtualEnergyFillRate": 0.025, "virtualEnergyInsert": 0.0,
                "virtualEnergyTimeConcurrent": 1.0}},
            "pitMenu": {"pitMenu": [
                {"PMC Value": 5, "currentSetting": 0, "default": 0, "name": "DRIVER:", "settings": [{"text": "A Driver"}]},
                {"PMC Value": 6, "currentSetting": 2, "default": 100, "name": "VIRTUAL ENERGY:",
                 "settings": [{"text": "0% 0 laps"}, {"text": "1% 0 laps"}, {"text": "25% 7 laps"}]}
            ]},
            "pitRecommendations": {"fuel": 0, "virtualEnergy": 0}
        })
    }

    #[test]
    fn a_real_response_is_parsed() {
        let p = parse_repair_and_refuel(&sample());
        assert_eq!(p.fuel, Some(80.09));
        assert_eq!(p.fuel_capacity, Some(110.0));
        assert!((p.ve.unwrap() - 0.964).abs() < 0.001);
        let t = p.times.unwrap();
        assert_eq!((t.fuel_fill_rate, t.ve_fill_rate, t.four_tyres_s), (4.0, 0.025, 12.0));
        assert!(t.fuel_concurrent && t.ve_concurrent && t.driver_concurrent && !t.tyres_concurrent);
        assert_eq!(p.menu_ve, Some(0.25));
        assert_eq!(p.menu_fuel, None);
        assert_eq!(p.recommended_ve, Some(0.0));
    }

    #[test]
    fn a_car_without_ve_has_none() {
        // LMP3 online: fuelInfo without the VE keys (measured).
        let v = serde_json::json!({"fuelInfo": {"currentBattery": 0.0, "currentFuel": 100.0, "maxBattery": 0.0, "maxFuel": 100.0}});
        let p = parse_repair_and_refuel(&v);
        assert_eq!((p.fuel, p.ve, p.times.is_some()), (Some(100.0), None, false));
    }

    #[test]
    fn the_estimate_is_parsed() {
        let v = serde_json::json!({"brakeDucts":0.0,"brakes":0.0,"damage":0.0,"driverSwap":0.0,"fuel":0.0,"penalties":10.0,"tires":0.0,"total":10.0,"ve":0.0});
        let e = parse_estimate(&v).unwrap();
        assert_eq!((e.total, e.penalties), (10.0, 10.0));
        assert!(parse_estimate(&Value::Null).is_none());
    }

    #[test]
    fn menu_percentages() {
        assert_eq!(leading_percent("25% 7 laps"), Some(0.25));
        assert_eq!(leading_percent("100% 27 laps"), Some(1.0));
        assert_eq!(leading_percent("N/A"), None);
        assert_eq!(leading_percent("12 laps"), None);
    }
}
