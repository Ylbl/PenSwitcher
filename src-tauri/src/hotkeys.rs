use crate::{
    models::ShortcutItem,
    state::AppState,
    storage::persist_shortcuts,
    uia::{automation, invoke_item},
    utils::lock_err,
    HOTKEY_EVENT,
};
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use std::{collections::HashMap, str::FromStr};
use tauri::{AppHandle, Emitter, Manager};

pub struct HotkeyRegistry {
    manager: Option<GlobalHotKeyManager>,
    by_hotkey_id: HashMap<u32, String>,
    registered: HashMap<String, HotKey>,
}

impl Default for HotkeyRegistry {
    fn default() -> Self {
        Self {
            manager: GlobalHotKeyManager::new().ok(),
            by_hotkey_id: HashMap::new(),
            registered: HashMap::new(),
        }
    }
}

unsafe impl Send for HotkeyRegistry {}

pub fn install_global_handler(app: AppHandle) {
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state == HotKeyState::Pressed {
            let app = app.clone();
            std::thread::spawn(move || handle_hotkey_event(app, event.id));
        }
    }));
}

pub fn rebuild_hotkeys(state: &AppState, app: AppHandle) -> Result<(), String> {
    let mut registry = state.hotkeys.lock().map_err(lock_err)?;
    let old: Vec<HotKey> = registry.registered.values().copied().collect();
    if let Some(manager) = &registry.manager {
        for hotkey in old {
            let _ = manager.unregister(hotkey);
        }
    }
    registry.by_hotkey_id.clear();
    registry.registered.clear();

    let mut store = state.shortcuts.lock().map_err(lock_err)?;
    for item in &mut store.items {
        if item.enabled && !item.hotkey.trim().is_empty() && item.supports_invoke {
            match parse_hotkey(&item.hotkey) {
                Ok(hotkey) => {
                    if let Some(manager) = &registry.manager {
                        match manager.register(hotkey) {
                            Ok(_) => {
                                registry.by_hotkey_id.insert(hotkey.id(), item.id.clone());
                                registry.registered.insert(item.id.clone(), hotkey);
                                item.status = "已监听".into();
                                tracing::info!(hotkey = %item.hotkey, name = %item.node.name, "快捷键已注册");
                            }
                            Err(error) => {
                                item.status = format!("注册失败: {error}");
                                item.enabled = false;
                                tracing::warn!(%error, hotkey = %item.hotkey, "快捷键注册失败");
                            }
                        }
                    } else {
                        item.status = "快捷键管理器不可用".into();
                        item.enabled = false;
                    }
                }
                Err(error) => {
                    item.status = error;
                    item.enabled = false;
                }
            }
        }
    }
    persist_shortcuts(&app, &store)?;
    Ok(())
}

fn handle_hotkey_event(app: AppHandle, hotkey_id: u32) {
    let state = app.state::<AppState>();
    let item_id = {
        let registry = match state.hotkeys.lock() {
            Ok(registry) => registry,
            Err(error) => {
                tracing::error!(%error, "读取快捷键注册表失败");
                return;
            }
        };
        registry.by_hotkey_id.get(&hotkey_id).cloned()
    };

    let Some(item_id) = item_id else {
        return;
    };

    let item: Option<ShortcutItem> = {
        let store = match state.shortcuts.lock() {
            Ok(store) => store,
            Err(error) => {
                tracing::error!(%error, "读取快捷键配置失败");
                return;
            }
        };
        store.items.iter().find(|item| item.id == item_id).cloned()
    };

    if let Some(item) = item {
        match automation().and_then(|uia| invoke_item(&uia, &item)) {
            Ok(_) => {
                let _ = app.emit(HOTKEY_EVENT, item);
            }
            Err(error) => {
                tracing::error!(%error, "快捷键 Invoke 失败");
            }
        }
    }
}

fn parse_hotkey(value: &str) -> Result<HotKey, String> {
    let mut mods = Modifiers::empty();
    let mut key = None;
    for part in value.split('+').map(|p| p.trim()).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "meta" | "win" | "super" => mods |= Modifiers::META,
            other => key = Some(code_from_key(other)?),
        }
    }
    key.map(|code| HotKey::new(if mods.is_empty() { None } else { Some(mods) }, code))
        .ok_or_else(|| "缺少主键".into())
}

fn code_from_key(key: &str) -> Result<Code, String> {
    if let Ok(code) = Code::from_str(key) {
        return Ok(code);
    }
    let upper = key.to_ascii_uppercase();
    if upper.len() == 1 {
        let ch = upper.chars().next().unwrap();
        if ch.is_ascii_alphabetic() {
            return Ok(match ch {
                'A' => Code::KeyA,
                'B' => Code::KeyB,
                'C' => Code::KeyC,
                'D' => Code::KeyD,
                'E' => Code::KeyE,
                'F' => Code::KeyF,
                'G' => Code::KeyG,
                'H' => Code::KeyH,
                'I' => Code::KeyI,
                'J' => Code::KeyJ,
                'K' => Code::KeyK,
                'L' => Code::KeyL,
                'M' => Code::KeyM,
                'N' => Code::KeyN,
                'O' => Code::KeyO,
                'P' => Code::KeyP,
                'Q' => Code::KeyQ,
                'R' => Code::KeyR,
                'S' => Code::KeyS,
                'T' => Code::KeyT,
                'U' => Code::KeyU,
                'V' => Code::KeyV,
                'W' => Code::KeyW,
                'X' => Code::KeyX,
                'Y' => Code::KeyY,
                'Z' => Code::KeyZ,
                _ => unreachable!(),
            });
        }
        if ch.is_ascii_digit() {
            return Ok(match ch {
                '0' => Code::Digit0,
                '1' => Code::Digit1,
                '2' => Code::Digit2,
                '3' => Code::Digit3,
                '4' => Code::Digit4,
                '5' => Code::Digit5,
                '6' => Code::Digit6,
                '7' => Code::Digit7,
                '8' => Code::Digit8,
                '9' => Code::Digit9,
                _ => unreachable!(),
            });
        }
    }
    if let Some(rest) = upper.strip_prefix('F') {
        return match rest {
            "1" => Ok(Code::F1),
            "2" => Ok(Code::F2),
            "3" => Ok(Code::F3),
            "4" => Ok(Code::F4),
            "5" => Ok(Code::F5),
            "6" => Ok(Code::F6),
            "7" => Ok(Code::F7),
            "8" => Ok(Code::F8),
            "9" => Ok(Code::F9),
            "10" => Ok(Code::F10),
            "11" => Ok(Code::F11),
            "12" => Ok(Code::F12),
            _ => Err(format!("不支持快捷键主键 {key}")),
        };
    }
    match upper.as_str() {
        "SPACE" => Ok(Code::Space),
        "ENTER" => Ok(Code::Enter),
        "TAB" => Ok(Code::Tab),
        "ESCAPE" | "ESC" => Ok(Code::Escape),
        _ => Err(format!("不支持快捷键主键 {key}")),
    }
}
