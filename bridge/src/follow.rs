//! The car to follow when the game gives this PC none of its own.
//!
//! In LMU's engineer seat (and as a spectator) no car is the player's:
//! `playerHasVehicle` is 0 and every car is under remote control (measured).
//! The engineer then picks the team car by hand, and the bridge treats it as
//! "our car" for timing, gaps, rivals, strategy and the AI — everything that
//! is real for another PC's car. Fuel, tyre wear and brake temperatures of
//! that car are placeholders in LMU, so the bridge marks the telemetry as
//! `spectated` and the dashboard does not show them as data.
//!
//! The choice is the car's name as the game shows it (e.g. "Team X #93"),
//! which survives from one session to the next, unlike the slot number. It is
//! kept in `follow.json`.

use std::path::PathBuf;
use std::sync::RwLock;

static FOLLOW: RwLock<Option<String>> = RwLock::new(None);
static LOADED: std::sync::Once = std::sync::Once::new();

fn path() -> PathBuf {
    crate::paths::root().join("follow.json")
}

fn load_once() {
    LOADED.call_once(|| {
        let saved = std::fs::read(path()).ok().and_then(|d| serde_json::from_slice::<Option<String>>(&d).ok()).flatten();
        if let Ok(mut f) = FOLLOW.write() {
            *f = saved;
        }
    });
}

/// The followed car's name, if one is chosen.
pub fn get() -> Option<String> {
    load_once();
    FOLLOW.read().ok().and_then(|f| f.clone())
}

/// Choose the car to follow (`None` = none), and remember it.
pub fn set(name: Option<String>) {
    load_once();
    let name = name.map(|n| n.trim().chars().take(64).collect::<String>()).filter(|n| !n.is_empty());
    if let Ok(mut f) = FOLLOW.write() {
        *f = name.clone();
    }
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, serde_json::to_vec(&name).unwrap_or_default());
}
