//! Where Pitwall keeps its files.
//!
//! The main data folder is `%APPDATA%\LMUPitwall`: settings, the access token,
//! track outlines, the pit lane losses, logs. On a driver's bridge everything
//! lives there. On a hub each team has a folder of its own under
//! `teams\<id>` (see `hub::teams`) for what belongs to one car (strategy settings, AI settings,
//! session history); what is about the tracks or the money stays shared.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Set by `--data-dir`, before anything reads or writes a file.
static OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Use this folder instead of the default one, for the whole run.
pub fn set_root(dir: PathBuf) {
    let _ = OVERRIDE.set(dir);
}

/// `%APPDATA%\LMUPitwall` (or `~/.config/LMUPitwall` elsewhere), unless
/// `--data-dir` named another folder.
pub fn root() -> PathBuf {
    if let Some(dir) = OVERRIDE.get() {
        return dir.clone();
    }
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")).unwrap_or_else(|_| PathBuf::from("."))
        })
        .join("LMUPitwall")
}
