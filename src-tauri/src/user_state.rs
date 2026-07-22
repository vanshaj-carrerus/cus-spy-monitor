//! Persist active user so any sibling can resume monitoring after takeover.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::watchdog;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedUser {
    pub user_id: String,
    pub username: String,
}

fn user_file() -> PathBuf {
    watchdog::data_dir().join("active_user.json")
}

pub fn save(user_id: &str, username: &str) {
    let data = PersistedUser {
        user_id: user_id.to_string(),
        username: username.to_string(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&data) {
        let _ = fs::write(user_file(), json);
    }
}

pub fn load() -> Option<PersistedUser> {
    let raw = fs::read_to_string(user_file()).ok()?;
    serde_json::from_str(&raw).ok()
}
