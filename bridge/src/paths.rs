//! Where Pitwall keeps its files.
//!
//! The main data folder is `%APPDATA%\LMUPitwall`: settings, the access token,
//! track outlines, the pit lane losses, logs. On a driver's bridge everything
//! lives there. On a hub each team has a folder of its own under
//! `teams\<id>` (see `hub::teams`) for what belongs to one car (strategy settings, AI settings,
//! session history); what is about the tracks or the money stays shared.

use std::path::PathBuf;

/// `%APPDATA%\LMUPitwall` (or `~/.config/LMUPitwall` elsewhere).
pub fn root() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")).unwrap_or_else(|_| PathBuf::from("."))
        })
        .join("LMUPitwall")
}
