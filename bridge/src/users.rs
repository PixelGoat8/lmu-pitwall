//! The people who may connect to a hub, kept in `users.json` in the main data
//! folder.
//!
//! Each person has a name, a role and a personal link (a token of their own),
//! so the hub knows who every connection is and can take one person's access
//! away without changing everybody's link. Roles:
//!
//! * **engineer** — runs the race on every team: strategy, targets, pinning a
//!   driver, asking the AI. Does not change the hub's settings.
//! * **guest** — watches, and only the teams listed in `teams`.
//!
//! The administrator is not a person in this file: it is whoever sits at the
//! hub's PC (see `crate::access`). The hub's own token, from `config.json`,
//! stays valid as an engineer's link.
//!
//! Until the hub has a panel for this, people are added by editing the file:
//! `{"name": "Marco", "role": "engineer"}` is enough. The hub fills in the id
//! and a token, writes them back, and picks up the change without a restart.
//! Removing a person from the file disconnects them within seconds.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::access::{self, Role};

const MAX_NAME_CHARS: usize = 40;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// `engineer` or `guest`. Anything else (a missing role, a typo, `admin`)
    /// is read as `guest`: a mistake in the file never grants more.
    #[serde(default, deserialize_with = "role_from_file")]
    pub role: Role,
    /// Teams a guest may watch, by id. Engineers watch every team.
    #[serde(default)]
    pub teams: Vec<String>,
    /// The personal link's token. Never logged.
    #[serde(default)]
    pub token: String,
    /// When the person was added (Unix seconds).
    #[serde(default)]
    pub created: u64,
}

fn role_from_file<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Role, D::Error> {
    let s = Option::<String>::deserialize(d)?.unwrap_or_default();
    Ok(match s.trim().to_ascii_lowercase().as_str() {
        "engineer" => Role::Engineer,
        _ => Role::Guest,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsersFile {
    #[serde(default)]
    pub users: Vec<User>,
}

struct State {
    file: UsersFile,
    /// Modification time of the file as last read or written, to notice an
    /// edit made by hand.
    mtime: Option<SystemTime>,
}

pub struct UsersStore {
    path: PathBuf,
    state: Mutex<State>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Make a file as written by hand usable: names trimmed, every person with an
/// id and a token of their own. Returns whether anything changed (and so
/// should be written back).
fn normalize(file: &mut UsersFile) -> bool {
    let before = file.clone();
    file.users.retain(|u| !u.name.trim().is_empty());
    let mut ids = std::collections::HashSet::new();
    let mut tokens = std::collections::HashSet::new();
    for u in &mut file.users {
        u.name = u.name.trim().chars().take(MAX_NAME_CHARS).collect();
        if u.id.trim().is_empty() || !ids.insert(u.id.clone()) {
            u.id = access::generate_token()[..8].to_string();
            ids.insert(u.id.clone());
        }
        // Short or shared tokens would be easy to guess or would make two
        // people one: give them a fresh one.
        if u.token.len() < 16 || !tokens.insert(u.token.clone()) {
            u.token = access::generate_token();
            tokens.insert(u.token.clone());
        }
        if u.created == 0 {
            u.created = now();
        }
        u.teams.retain(|t| !t.trim().is_empty());
        u.teams.dedup();
    }
    *file != before
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl UsersStore {
    /// The people of the hub whose main folder is `root` (none if there is no
    /// `users.json` yet).
    pub fn open(root: &Path) -> Self {
        let store = UsersStore {
            path: root.join("users.json"),
            state: Mutex::new(State { file: UsersFile::default(), mtime: None }),
        };
        if let Ok(mut st) = store.state.lock() {
            store.load(&mut st);
            info!("Hub: {} person(s) with a personal link", st.file.users.len());
        }
        store
    }

    /// Read the file into `st`, completing and writing it back if needed. A
    /// file that does not parse is left alone and the previous list kept.
    fn load(&self, st: &mut State) {
        st.mtime = mtime(&self.path);
        let Ok(data) = std::fs::read(&self.path) else {
            st.file = UsersFile::default();
            return;
        };
        match serde_json::from_slice::<UsersFile>(&data) {
            Ok(mut file) => {
                if normalize(&mut file) {
                    self.write(st, &file);
                }
                st.file = file;
            }
            Err(e) => warn!("users.json is not valid, keeping the previous list: {}", e),
        }
    }

    fn write(&self, st: &mut State, file: &UsersFile) {
        let write = || -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = self.path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(file).unwrap_or_default())?;
            std::fs::rename(&tmp, &self.path)
        };
        match write() {
            Ok(()) => st.mtime = mtime(&self.path),
            Err(e) => warn!("Could not save users.json: {}", e),
        }
    }

    /// The list as it is now, re-read first if the file changed on disk.
    fn current(&self) -> std::sync::MutexGuard<'_, State> {
        let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if mtime(&self.path) != st.mtime {
            info!("Hub: users.json changed, reloading");
            self.load(&mut st);
        }
        st
    }

    /// Pick up an edit of the file, completing new entries with an id and a
    /// token. The hub calls it every second, so a person added by hand gets
    /// their link at once.
    pub fn refresh(&self) {
        drop(self.current());
    }

    /// The person whose personal link carries `token`.
    pub fn by_token(&self, token: &str) -> Option<User> {
        let st = self.current();
        // Look at every entry, so the time taken does not tell how close a
        // guess came.
        let mut found = None;
        for u in &st.file.users {
            if access::constant_time_eq(u.token.as_bytes(), token.as_bytes()) {
                found = Some(u.clone());
            }
        }
        found
    }

    pub fn get(&self, id: &str) -> Option<User> {
        self.current().file.users.iter().find(|u| u.id == id).cloned()
    }

    /// A team was deleted: no guest keeps it in their list.
    pub fn forget_team(&self, team_id: &str) {
        let mut st = self.current();
        let mut file = st.file.clone();
        for u in &mut file.users {
            u.teams.retain(|t| t != team_id);
        }
        if file != st.file {
            self.write(&mut st, &file);
            st.file = file;
        }
    }

    #[cfg(test)]
    fn snapshot(&self) -> UsersFile {
        self.current().file.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(dir: &Path, json: &str) {
        std::fs::write(dir.join("users.json"), json).unwrap();
    }

    #[test]
    fn no_file_means_nobody() {
        let dir = tempfile::tempdir().unwrap();
        let store = UsersStore::open(dir.path());
        assert!(store.snapshot().users.is_empty());
        assert!(store.by_token("anything-at-all-1234").is_none());
        assert!(!dir.path().join("users.json").exists());
    }

    #[test]
    fn a_hand_written_entry_gets_an_id_and_a_token() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), r#"{"users":[{"name":"  Marco ","role":"engineer"},{"name":"Gianni","role":"guest","teams":["clienti"]},{"name":"  "}]}"#);
        let store = UsersStore::open(dir.path());
        let users = store.snapshot().users;
        assert_eq!(users.len(), 2, "an entry without a name is dropped");
        let marco = &users[0];
        assert_eq!(marco.name, "Marco");
        assert_eq!(marco.role, Role::Engineer);
        assert_eq!(marco.id.len(), 8);
        assert_eq!(marco.token.len(), 32);
        assert!(marco.created > 0);
        assert_ne!(marco.token, users[1].token);
        assert_eq!(users[1].role, Role::Guest);
        assert_eq!(users[1].teams, vec!["clienti".to_string()]);

        // Written back, so the admin can copy the link from the file.
        let on_disk: UsersFile = serde_json::from_slice(&std::fs::read(dir.path().join("users.json")).unwrap()).unwrap();
        assert_eq!(on_disk.users, users);

        assert_eq!(store.by_token(&marco.token).map(|u| u.name), Some("Marco".into()));
        assert_eq!(store.get(&users[1].id).map(|u| u.name), Some("Gianni".into()));
    }

    #[test]
    fn a_mistake_in_the_role_never_grants_more() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), r#"{"users":[{"name":"A","role":"admin"},{"name":"B","role":"Engineer"},{"name":"C","role":"enginer"},{"name":"D"}]}"#);
        let roles: Vec<Role> = UsersStore::open(dir.path()).snapshot().users.iter().map(|u| u.role).collect();
        assert_eq!(roles, vec![Role::Guest, Role::Engineer, Role::Guest, Role::Guest]);
    }

    #[test]
    fn weak_or_shared_tokens_are_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let shared = "0123456789abcdef0123456789abcdef";
        write_file(
            dir.path(),
            &format!(r#"{{"users":[{{"id":"x","name":"A","token":"1234"}},{{"id":"x","name":"B","token":"{shared}"}},{{"name":"C","token":"{shared}"}}]}}"#),
        );
        let users = UsersStore::open(dir.path()).snapshot().users;
        assert_ne!(users[0].token, "1234");
        assert_eq!(users[1].token, shared, "the first holder keeps a good token");
        assert_ne!(users[2].token, shared);
        assert_ne!(users[0].id, users[1].id, "ids are unique");
    }

    #[test]
    fn an_edit_on_disk_is_picked_up_and_a_removed_person_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), r#"{"users":[{"name":"Marco","role":"engineer"}]}"#);
        let store = UsersStore::open(dir.path());
        let marco = store.snapshot().users[0].clone();
        assert!(store.by_token(&marco.token).is_some());

        // Some file systems keep modification times to the second.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write_file(dir.path(), r#"{"users":[]}"#);
        assert!(store.by_token(&marco.token).is_none());
        assert!(store.get(&marco.id).is_none());
    }

    #[test]
    fn a_broken_file_keeps_the_previous_list() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), r#"{"users":[{"name":"Marco","role":"engineer"}]}"#);
        let store = UsersStore::open(dir.path());
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write_file(dir.path(), r#"{"users":[{"name":"Marco""#);
        assert_eq!(store.snapshot().users.len(), 1);
    }

    #[test]
    fn a_deleted_team_leaves_the_guests_lists() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), r#"{"users":[{"name":"G","role":"guest","teams":["ufficiale","clienti"]}]}"#);
        let store = UsersStore::open(dir.path());
        store.forget_team("clienti");
        assert_eq!(store.snapshot().users[0].teams, vec!["ufficiale".to_string()]);
    }
}
