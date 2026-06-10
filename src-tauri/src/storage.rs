use crate::models::ShortcutStore;
use std::fs;
use tauri::{AppHandle, Manager};

pub fn load_shortcuts(app: &AppHandle) -> Result<ShortcutStore, String> {
    let path = shortcuts_path(app)?;
    if !path.exists() {
        return Ok(ShortcutStore::default());
    }
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&content).map_err(|e| e.to_string())
}

pub fn persist_shortcuts(app: &AppHandle, store: &ShortcutStore) -> Result<(), String> {
    let path = shortcuts_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())
}

fn shortcuts_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("shortcuts.json"))
}
