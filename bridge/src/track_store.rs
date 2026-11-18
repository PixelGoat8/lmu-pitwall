//! Track outlines kept on this Pitwall, one file per track.
//!
//! The dashboard records a track's outline from a lap of position samples
//! (see `dashboard/src/utils/trackOutline.ts`). Kept only in the browser, every
//! dashboard had to record its own: the tablet, the hub's page, the same page
//! under another address. Stored here, the first dashboard to finish a lap
//! gives the map to all the others. The record is the dashboard's own JSON,
//! checked for shape and size but otherwise passed through.

use std::path::{Path, PathBuf};

use serde_json::Value;
use tracing::{info, warn};

use crate::protocol::messages::{ClientCommand, ServerMessage};

/// A larger record is refused (a long track at 5 m spacing is ~300 KB).
const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
const MIN_POINTS: usize = 50;
const MAX_POINTS: usize = 50_000;
const MAX_NAME_CHARS: usize = 100;
/// A whole lap closes on itself: its two ends are this close (m)…
const MAX_CLOSE_M: f64 = 200.0;
/// …and no stretch of it longer than this (m of lap distance) lacks a point.
const MAX_HOLE_M: f64 = 150.0;

fn data_dir() -> PathBuf {
    crate::paths::root().join("tracks")
}

/// File name for a track: letters and digits, lower case, everything else `_`.
fn file_name(track: &str) -> String {
    let stem: String = track
        .chars()
        .take(MAX_NAME_CHARS)
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect();
    format!("{stem}.json")
}

pub fn is_track_command(cmd: &ClientCommand) -> bool {
    matches!(
        cmd,
        ClientCommand::TrackOutlineGet { .. } | ClientCommand::TrackOutlineSave { .. } | ClientCommand::TrackOutlineDelete { .. }
    )
}

pub fn handle_command(cmd: ClientCommand) -> ServerMessage {
    let dir = data_dir();
    match cmd {
        ClientCommand::TrackOutlineGet { track } => {
            let json = get_in(&dir, &track);
            ServerMessage::TrackOutline { track, json }
        }
        ClientCommand::TrackOutlineSave { track, json } => {
            match save_in(&dir, &track, &json) {
                Ok(true) => info!("Track map: outline of {} stored", track),
                Ok(false) => {}
                Err(e) => warn!("Track map: outline of {} not stored: {}", track, e),
            }
            // Either way, what is stored now.
            let json = get_in(&dir, &track);
            ServerMessage::TrackOutline { track, json }
        }
        ClientCommand::TrackOutlineDelete { track } => {
            if delete_in(&dir, &track) {
                info!("Track map: outline of {} deleted", track);
            }
            ServerMessage::TrackOutline { track, json: None }
        }
        _ => ServerMessage::TrackOutline { track: String::new(), json: None },
    }
}

/// The stored record, if there is one and it is a whole lap. A record that
/// is not (older dashboards could save a fragment as complete) counts as
/// none, so the next good lap replaces it.
fn get_in(dir: &Path, track: &str) -> Option<String> {
    if track.trim().is_empty() {
        return None;
    }
    let json = std::fs::read_to_string(dir.join(file_name(track))).ok()?;
    match summarise(track, &json) {
        Ok(s) if s.complete => Some(json),
        Ok(_) => None,
        Err(e) => {
            warn!("Track map: stored outline of {} ignored: {}", track, e);
            None
        }
    }
}

/// Remove the stored record of `track`; whether there was one.
fn delete_in(dir: &Path, track: &str) -> bool {
    !track.trim().is_empty() && std::fs::remove_file(dir.join(file_name(track))).is_ok()
}

/// What the dashboard's record says about itself, after a shape check.
struct Summary {
    version: i64,
    complete: bool,
    has_sectors: bool,
}

fn summarise(track: &str, json: &str) -> anyhow::Result<Summary> {
    anyhow::ensure!(json.len() <= MAX_JSON_BYTES, "too large ({} bytes)", json.len());
    let v: Value = serde_json::from_str(json)?;
    anyhow::ensure!(v.get("trackName").and_then(Value::as_str) == Some(track), "for another track");
    let points = v.get("points").and_then(Value::as_array).ok_or_else(|| anyhow::anyhow!("no points"))?;
    anyhow::ensure!((MIN_POINTS..=MAX_POINTS).contains(&points.len()), "{} points", points.len());
    let xz: Vec<(f64, f64, f64)> = points
        .iter()
        .map(|p| {
            let g = |k: &str| p.get(k).and_then(Value::as_f64);
            Some((g("x")?, g("z")?, g("d").unwrap_or(-1.0)))
        })
        .collect::<Option<_>>()
        .ok_or_else(|| anyhow::anyhow!("a point without coordinates"))?;
    let (a, b) = (xz[0], xz[xz.len() - 1]);
    let gap = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    anyhow::ensure!(gap <= MAX_CLOSE_M, "not a whole lap: its ends are {gap:.0} m apart");
    let hole = xz.windows(2).filter(|w| w[0].2 >= 0.0 && w[1].2 >= 0.0).map(|w| w[1].2 - w[0].2).fold(0.0, f64::max);
    anyhow::ensure!(hole <= MAX_HOLE_M, "a {hole:.0} m stretch without points");
    let num = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(-1.0);
    Ok(Summary {
        version: v.get("version").and_then(Value::as_i64).unwrap_or(1),
        complete: v.get("complete").and_then(Value::as_bool).unwrap_or(false),
        has_sectors: num("sector1Dist") > 0.0 && num("sector2Dist") > num("sector1Dist"),
    })
}

/// Keep `json` for `track` if it is a complete outline and better than what
/// is stored: nothing stored, an incomplete or older record, or the same
/// version without the sector lines. Returns whether it was written.
fn save_in(dir: &Path, track: &str, json: &str) -> anyhow::Result<bool> {
    anyhow::ensure!(!track.trim().is_empty(), "no track name");
    let new = summarise(track, json)?;
    anyhow::ensure!(new.complete, "not a complete lap");
    if let Some(old) = get_in(dir, track).and_then(|j| summarise(track, &j).ok()) {
        let better = !old.complete
            || new.version > old.version
            || (new.version == old.version && new.has_sectors && !old.has_sectors);
        if !better {
            return Ok(false);
        }
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(file_name(track));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outline(track: &str, version: i64, points: usize, sectors: bool) -> String {
        // A circle of `points` points, 10 m apart: a closed lap.
        let r = points as f64 * 10.0 / std::f64::consts::TAU;
        let pts: Vec<Value> = (0..points)
            .map(|i| {
                let a = i as f64 / points as f64 * std::f64::consts::TAU;
                serde_json::json!({"x": r * a.cos(), "y": 0.0, "z": r * a.sin(), "d": i as f64 * 10.0})
            })
            .collect();
        serde_json::json!({
            "version": version, "trackName": track, "points": pts, "complete": true,
            "bounds": {"minX": 0, "maxX": 1, "minZ": 0, "maxZ": 1},
            "sector1Dist": if sectors { 100.0 } else { -1.0 }, "sector2Dist": if sectors { 200.0 } else { -1.0 },
        })
        .to_string()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pitwall-tracks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_first_good_outline_is_kept_and_a_better_one_replaces_it() {
        let dir = temp_dir("keep");
        let t = "Bahrain International Circuit";
        assert_eq!(get_in(&dir, t), None);
        assert!(save_in(&dir, t, &outline(t, 3, 300, false)).unwrap());
        // Same version, no better: kept as it is.
        assert!(!save_in(&dir, t, &outline(t, 3, 400, false)).unwrap());
        // With the sector lines: replaces it.
        assert!(save_in(&dir, t, &outline(t, 3, 280, true)).unwrap());
        let stored: Value = serde_json::from_str(&get_in(&dir, t).unwrap()).unwrap();
        assert_eq!(stored["points"].as_array().unwrap().len(), 280);
        // A newer version replaces it too.
        assert!(save_in(&dir, t, &outline(t, 4, 300, false)).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_records_are_refused() {
        let dir = temp_dir("bad");
        let t = "Spa";
        assert!(save_in(&dir, t, "not json").is_err());
        assert!(save_in(&dir, t, &outline("Monza", 3, 300, true)).is_err());
        assert!(save_in(&dir, t, &outline(t, 3, 10, true)).is_err());
        let mut v: Value = serde_json::from_str(&outline(t, 3, 300, true)).unwrap();
        v["complete"] = Value::Bool(false);
        assert!(save_in(&dir, t, &v.to_string()).is_err());
        assert!(save_in(&dir, "  ", &outline("  ", 3, 300, true)).is_err());
        assert_eq!(get_in(&dir, t), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fragment_saved_as_complete_is_refused_and_ignored() {
        let dir = temp_dir("fragment");
        let t = "Bahrain";
        // The first 150 points of a 500-point lap: complete by its own flag,
        // but its ends are far apart (what an older dashboard could store).
        let mut v: Value = serde_json::from_str(&outline(t, 2, 500, true)).unwrap();
        v["points"] = Value::Array(v["points"].as_array().unwrap()[..150].to_vec());
        let fragment = v.to_string();
        assert!(save_in(&dir, t, &fragment).is_err());
        // Already on disk from before: not handed out, and a good lap replaces it.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(file_name(t)), &fragment).unwrap();
        assert_eq!(get_in(&dir, t), None);
        assert!(save_in(&dir, t, &outline(t, 2, 300, false)).unwrap());
        assert!(get_in(&dir, t).is_some());
        // A hole in the middle of a closed lap.
        let mut v: Value = serde_json::from_str(&outline(t, 2, 300, true)).unwrap();
        let pts = v["points"].as_array_mut().unwrap();
        pts.drain(100..120);
        assert!(save_in(&dir, t, &v.to_string()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_outline_can_be_deleted() {
        let dir = temp_dir("delete");
        let t = "Monza";
        assert!(save_in(&dir, t, &outline(t, 3, 300, true)).unwrap());
        assert!(delete_in(&dir, t));
        assert_eq!(get_in(&dir, t), None);
        assert!(!delete_in(&dir, t));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("Circuit de la Sarthe"), "circuit_de_la_sarthe.json");
        assert_eq!(file_name("../../etc/passwd"), "______etc_passwd.json");
        assert!(file_name(&"x".repeat(500)).len() <= MAX_NAME_CHARS + 5);
    }
}
