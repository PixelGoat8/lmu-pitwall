//! The teams of a hub, kept in `teams.json` in the main data folder.
//!
//! A team is one car: its drivers' bridges, the bridge it is pinned to, a
//! name and a colour. Each team has a folder of its own (`teams\<id>`) for its
//! strategy and AI settings and its history. The list stays from one race to
//! the next: before a race the engineer moves drivers where they belong.
//!
//! A hub from before teams kept one list in `hub.json`: it becomes the first
//! team, and the strategy and AI settings it used are copied into that team's
//! folder so nothing has to be set again.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::{HubConfig, SourceConfig};

/// Colours offered to new teams, in order (the dashboard's chart palette).
pub const PALETTE: [&str; 4] = ["#3987e5", "#d95926", "#199e70", "#c98500"];
const MAX_NAME_CHARS: usize = 40;
/// Settings a team inherits from a hub that predates teams.
const INHERITED: [&str; 2] = ["strategy.json", "ai.json"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamConfig {
    pub id: String,
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    /// Id of the bridge the engineer pinned; `None` = follow the driver.
    #[serde(default)]
    pub follow: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamsFile {
    #[serde(default)]
    pub teams: Vec<TeamConfig>,
}

/// The team list and where it is kept. Every change is written at once.
pub struct TeamsStore {
    root: PathBuf,
    file: Mutex<TeamsFile>,
}

/// A team id from its name: lower case letters, digits and dashes.
fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "team".into() } else { s }
}

fn clean_name(name: &str) -> Result<String> {
    let n: String = name.trim().chars().take(MAX_NAME_CHARS).collect();
    if n.is_empty() {
        return Err(anyhow!("a team needs a name"));
    }
    Ok(n)
}

fn clean_color(color: &str) -> Option<String> {
    let c = color.trim();
    let hex = c.strip_prefix('#')?;
    (hex.len() == 6 && hex.chars().all(|ch| ch.is_ascii_hexdigit())).then(|| c.to_lowercase())
}

impl TeamsStore {
    /// The teams of the hub whose main folder is `root`: `teams.json`, or the
    /// old `hub.json` as the first team, or one empty team.
    pub fn open(root: &Path) -> Self {
        let path = root.join("teams.json");
        let existing: Option<TeamsFile> = std::fs::read(&path).ok().and_then(|d| serde_json::from_slice(&d).ok());
        let store = TeamsStore { root: root.to_path_buf(), file: Mutex::new(TeamsFile::default()) };
        let mut file = match existing {
            Some(f) if !f.teams.is_empty() => f,
            _ => {
                let old: Option<HubConfig> = std::fs::read(root.join("hub.json")).ok().and_then(|d| serde_json::from_slice(&d).ok());
                let first = TeamConfig {
                    id: "squadra".into(),
                    name: "Squadra".into(),
                    color: PALETTE[0].into(),
                    sources: old.as_ref().map(|o| o.sources.clone()).unwrap_or_default(),
                    follow: old.and_then(|o| o.follow),
                };
                info!("Hub: teams start with \"{}\" ({} bridge(s) from hub.json)", first.name, first.sources.len());
                store.inherit_settings(&first.id);
                TeamsFile { teams: vec![first] }
            }
        };
        // Ids must be unique and usable as folder names.
        let mut seen = std::collections::HashSet::new();
        for t in &mut file.teams {
            let mut id = slug(&t.id);
            while !seen.insert(id.clone()) {
                id.push('2');
            }
            t.id = id;
            if clean_color(&t.color).is_none() {
                t.color = PALETTE[0].into();
            }
        }
        store.write(&file);
        *store.file.lock().unwrap() = file;
        store
    }

    /// Copy the settings a hub used before teams into a team's folder, unless
    /// the team already has its own.
    fn inherit_settings(&self, id: &str) {
        let dir = self.team_dir(id);
        let _ = std::fs::create_dir_all(&dir);
        for name in INHERITED {
            let (from, to) = (self.root.join(name), dir.join(name));
            if from.exists() && !to.exists() {
                if let Err(e) = std::fs::copy(&from, &to) {
                    warn!("Hub: could not copy {} into the team's folder: {}", name, e);
                }
            }
        }
    }

    pub fn team_dir(&self, id: &str) -> PathBuf {
        self.root.join("teams").join(id)
    }

    pub fn snapshot(&self) -> TeamsFile {
        self.file.lock().map(|f| f.clone()).unwrap_or_default()
    }

    fn write(&self, file: &TeamsFile) {
        let path = self.root.join("teams.json");
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(&self.root)?;
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(file).unwrap_or_default())?;
            std::fs::rename(&tmp, &path)
        };
        if let Err(e) = write() {
            warn!("Could not save teams.json: {}", e);
        }
    }

    fn change<R>(&self, f: impl FnOnce(&mut TeamsFile) -> Result<R>) -> Result<R> {
        let mut file = self.file.lock().map_err(|_| anyhow!("teams busy"))?;
        let r = f(&mut file)?;
        self.write(&file);
        Ok(r)
    }

    /// A team's bridges and pin, as its hub keeps them.
    pub fn set_sources(&self, id: &str, sources: Vec<SourceConfig>, follow: Option<String>) {
        let _ = self.change(|file| {
            let t = file.teams.iter_mut().find(|t| t.id == id).ok_or_else(|| anyhow!("no team {id}"))?;
            t.sources = sources;
            t.follow = follow;
            Ok(())
        });
    }

    pub fn create(&self, name: &str, color: Option<&str>) -> Result<TeamConfig> {
        let name = clean_name(name)?;
        let team = self.change(|file| {
            let mut id = slug(&name);
            let base = id.clone();
            let mut n = 2;
            while file.teams.iter().any(|t| t.id == id) {
                id = format!("{base}-{n}");
                n += 1;
            }
            let color = color
                .and_then(clean_color)
                .unwrap_or_else(|| PALETTE[file.teams.len() % PALETTE.len()].to_string());
            let team = TeamConfig { id, name, color, sources: Vec::new(), follow: None };
            file.teams.push(team.clone());
            Ok(team)
        })?;
        let _ = std::fs::create_dir_all(self.team_dir(&team.id));
        Ok(team)
    }

    pub fn update(&self, id: &str, name: Option<&str>, color: Option<&str>) -> Result<()> {
        let name = name.map(clean_name).transpose()?;
        let color = match color {
            Some(c) => Some(clean_color(c).ok_or_else(|| anyhow!("that is not a colour"))?),
            None => None,
        };
        self.change(|file| {
            let t = file.teams.iter_mut().find(|t| t.id == id).ok_or_else(|| anyhow!("no such team"))?;
            if let Some(n) = name {
                t.name = n;
            }
            if let Some(c) = color {
                t.color = c;
            }
            Ok(())
        })
    }

    /// Remove a team. Its drivers must have been moved first, and the last
    /// team stays. Its folder (settings, history) is left on disk.
    pub fn delete(&self, id: &str) -> Result<()> {
        self.change(|file| {
            let i = file.teams.iter().position(|t| t.id == id).ok_or_else(|| anyhow!("no such team"))?;
            if file.teams.len() == 1 {
                return Err(anyhow!("the last team cannot be removed"));
            }
            if !file.teams[i].sources.is_empty() {
                return Err(anyhow!("move this team's drivers to another team first"));
            }
            file.teams.remove(i);
            Ok(())
        })
    }

    /// The team a bridge belongs to, and its configuration.
    pub fn find_source(&self, source_id: &str) -> Option<(String, SourceConfig)> {
        let file = self.file.lock().ok()?;
        file.teams.iter().find_map(|t| t.sources.iter().find(|s| s.id == source_id).map(|s| (t.id.clone(), s.clone())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pitwall-teams-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn source(id: &str) -> SourceConfig {
        SourceConfig { id: id.into(), label: format!("Driver {id}"), url: format!("ws://host-{id}:9000/?token=t") }
    }

    #[test]
    fn a_hub_from_before_teams_becomes_the_first_team() {
        let root = temp_root("migrate");
        let old = HubConfig { sources: vec![source("a"), source("b")], follow: Some("b".into()) };
        std::fs::write(root.join("hub.json"), serde_json::to_vec(&old).unwrap()).unwrap();
        std::fs::write(root.join("strategy.json"), br#"{"reserve_laps":0.0}"#).unwrap();
        let store = TeamsStore::open(&root);
        let teams = store.snapshot().teams;
        assert_eq!(teams.len(), 1);
        assert_eq!((teams[0].id.as_str(), teams[0].sources.len(), teams[0].follow.as_deref()), ("squadra", 2, Some("b")));
        // Its settings came along, and teams.json now exists.
        assert!(store.team_dir("squadra").join("strategy.json").exists());
        assert!(root.join("teams.json").exists());
        // Opened again: the same list, not a second migration.
        assert_eq!(TeamsStore::open(&root).snapshot().teams, teams);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn teams_are_created_renamed_and_removed() {
        let root = temp_root("crud");
        let store = TeamsStore::open(&root);
        let clienti = store.create("Clienti", None).unwrap();
        assert_eq!(clienti.id, "clienti");
        assert_eq!(clienti.color, PALETTE[1]);
        // The same name again gets a different id.
        assert_eq!(store.create("Clienti", Some("#ABCDEF")).unwrap().id, "clienti-2");
        assert!(store.create("   ", None).is_err());
        store.update("clienti", Some("Clienti AF"), Some("#112233")).unwrap();
        assert!(store.update("clienti", None, Some("red")).is_err());
        let t = store.snapshot().teams.into_iter().find(|t| t.id == "clienti").unwrap();
        assert_eq!((t.name.as_str(), t.color.as_str()), ("Clienti AF", "#112233"));
        // A team with drivers stays until they are moved; the last team stays.
        store.set_sources("clienti", vec![source("x")], None);
        assert!(store.delete("clienti").is_err());
        assert_eq!(store.find_source("x").map(|(t, _)| t), Some("clienti".into()));
        store.set_sources("clienti", vec![], None);
        store.delete("clienti").unwrap();
        store.delete("clienti-2").unwrap();
        assert!(store.delete("squadra").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ids_are_safe_folder_names() {
        assert_eq!(slug("Ferrari Fabbrica!"), "ferrari-fabbrica");
        assert_eq!(slug("../../x"), "x");
        assert_eq!(slug("  "), "team");
    }
}
