use crate::{
    models::ShortcutItem,
    state::AppState,
    uia::{automation, invoke_item},
    utils::lock_err,
    HOTKEY_EVENT,
};
use rdev::{listen, Event, EventType, Key};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

pub struct HotkeyRegistry {
    pub sequences: Arc<Mutex<HashMap<String, HotkeySequence>>>,
}

impl Default for HotkeyRegistry {
    fn default() -> Self {
        Self {
            sequences: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HotkeySequence {
    pub item_id: String,
    pub modifiers: Vec<Key>,
    pub keys: Vec<Key>,
}

impl HotkeySequence {
    pub fn from_hotkey_string(item_id: &str, hotkey: &str) -> Result<Self, String> {
        let parts: Vec<&str> = hotkey
            .split('+')
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect();
        if parts.is_empty() {
            return Err("空快捷键".into());
        }
        let mut modifiers = Vec::new();
        let mut keys = Vec::new();
        for part in &parts {
            if let Some(key) = parse_key(part) {
                if is_modifier(&key) {
                    if !modifiers.contains(&key) {
                        modifiers.push(key);
                    }
                } else {
                    keys.push(key);
                }
            } else {
                return Err(format!("无法解析按键: {part}"));
            }
        }
        if keys.is_empty() {
            return Err("快捷键需要至少一个非修饰键".into());
        }
        if modifiers.is_empty() {
            return Err("快捷键需要至少一个修饰键(Ctrl/Alt/Shift/Win)".into());
        }
        Ok(HotkeySequence {
            item_id: item_id.to_string(),
            modifiers,
            keys,
        })
    }
}

fn is_modifier(key: &Key) -> bool {
    matches!(
        key,
        Key::ControlLeft
            | Key::ControlRight
            | Key::ShiftLeft
            | Key::ShiftRight
            | Key::Alt
            | Key::AltGr
            | Key::MetaLeft
            | Key::MetaRight
    )
}

pub fn start_listener(app: AppHandle, sequences: Arc<Mutex<HashMap<String, HotkeySequence>>>) {
    std::thread::spawn(move || {
        let mut active_modifiers: Vec<Key> = Vec::new();
        let mut sequence_modifiers: Vec<Key> = Vec::new();
        let mut key_buffer: Vec<Key> = Vec::new();

        if let Err(error) = listen(move |event: Event| {
            match event.event_type {
                EventType::KeyPress(key) => {
                    if is_modifier(&key) {
                        if !active_modifiers.contains(&key) {
                            active_modifiers.push(key);
                        }
                        if key_buffer.is_empty() {
                            sequence_modifiers = active_modifiers.clone();
                        }
                    } else if !active_modifiers.is_empty() {
                        if key_buffer.is_empty() {
                            sequence_modifiers = active_modifiers.clone();
                        }
                        key_buffer.push(key);
                    }
                }
                EventType::KeyRelease(key) => {
                    if is_modifier(&key) {
                        active_modifiers.retain(|k| k != &key);
                        if active_modifiers.is_empty() && !key_buffer.is_empty() {
                            let buffer = std::mem::take(&mut key_buffer);
                            check_sequences(&app, &sequences, &sequence_modifiers, &buffer);
                            sequence_modifiers.clear();
                        }
                    }
                }
                _ => {}
            }
        }) {
            tracing::error!(?error, "全局键盘监听失败");
        }
    });
}

fn check_sequences(
    app: &AppHandle,
    sequences: &Arc<Mutex<HashMap<String, HotkeySequence>>>,
    modifiers: &[Key],
    buffer: &[Key],
) {
    let sequences = match sequences.lock() {
        Ok(s) => s,
        Err(_) => return,
    };

    let modifier_set: std::collections::HashSet<&Key> = modifiers.iter().collect();

    for seq in sequences.values() {
        let mods_match = seq.modifiers.len() == modifier_set.len()
            && seq
                .modifiers
                .iter()
                .all(|m| modifier_set.iter().any(|k| key_match(m, k)));
        if !mods_match {
            continue;
        }
        if seq.keys.len() == buffer.len()
            && seq
                .keys
                .iter()
                .zip(buffer.iter())
                .all(|(a, b)| key_match(a, b))
        {
            handle_match(app, &seq.item_id);
            break;
        }
    }
}

fn key_match(a: &Key, b: &Key) -> bool {
    match (a, b) {
        (Key::ControlLeft, Key::ControlRight) | (Key::ControlRight, Key::ControlLeft) => true,
        (Key::ShiftLeft, Key::ShiftRight) | (Key::ShiftRight, Key::ShiftLeft) => true,
        _ => a == b,
    }
}

fn handle_match(app: &AppHandle, item_id: &str) {
    let state = app.state::<AppState>();
    let item: Option<ShortcutItem> = {
        let store = match state.shortcuts.lock() {
            Ok(store) => store,
            Err(error) => {
                tracing::error!(%error, "读取快捷键配置失败");
                return;
            }
        };
        store.items.iter().find(|i| i.id == item_id).cloned()
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

pub fn rebuild_hotkeys(state: &AppState) -> Result<(), String> {
    let new_sequences: HashMap<String, HotkeySequence> = {
        let mut store = state.shortcuts.lock().map_err(lock_err)?;
        let mut map = HashMap::new();
        for item in &mut store.items {
            if item.supports_invoke && !item.hotkey.trim().is_empty() {
                match HotkeySequence::from_hotkey_string(&item.id, &item.hotkey) {
                    Ok(seq) => {
                        map.insert(item.id.clone(), seq);
                        item.enabled = true;
                        item.status = "待命".into();
                    }
                    Err(error) => {
                        item.enabled = false;
                        item.status = error;
                    }
                }
            } else if item.hotkey.trim().is_empty() {
                item.enabled = false;
                item.status = if item.supports_invoke {
                    "未绑定".into()
                } else {
                    "无法调用".into()
                };
            }
        }
        map
    };

    let registry = state.hotkeys.lock().map_err(lock_err)?;
    let mut sequences = registry.sequences.lock().map_err(lock_err)?;
    *sequences = new_sequences;
    Ok(())
}

fn parse_key(s: &str) -> Option<Key> {
    match s {
        "Ctrl" | "Control" | "ControlLeft" => Some(Key::ControlLeft),
        "ControlRight" => Some(Key::ControlRight),
        "Alt" | "AltLeft" | "AltL" => Some(Key::Alt),
        "AltRight" | "AltR" | "AltGr" => Some(Key::AltGr),
        "Shift" | "ShiftLeft" => Some(Key::ShiftLeft),
        "ShiftRight" => Some(Key::ShiftRight),
        "Meta" | "Win" | "Super" | "MetaLeft" => Some(Key::MetaLeft),
        "MetaRight" => Some(Key::MetaRight),

        "KeyA" | "A" => Some(Key::KeyA),
        "KeyB" | "B" => Some(Key::KeyB),
        "KeyC" | "C" => Some(Key::KeyC),
        "KeyD" | "D" => Some(Key::KeyD),
        "KeyE" | "E" => Some(Key::KeyE),
        "KeyF" | "F" => Some(Key::KeyF),
        "KeyG" | "G" => Some(Key::KeyG),
        "KeyH" | "H" => Some(Key::KeyH),
        "KeyI" | "I" => Some(Key::KeyI),
        "KeyJ" | "J" => Some(Key::KeyJ),
        "KeyK" | "K" => Some(Key::KeyK),
        "KeyL" | "L" => Some(Key::KeyL),
        "KeyM" | "M" => Some(Key::KeyM),
        "KeyN" | "N" => Some(Key::KeyN),
        "KeyO" | "O" => Some(Key::KeyO),
        "KeyP" | "P" => Some(Key::KeyP),
        "KeyQ" | "Q" => Some(Key::KeyQ),
        "KeyR" | "R" => Some(Key::KeyR),
        "KeyS" | "S" => Some(Key::KeyS),
        "KeyT" | "T" => Some(Key::KeyT),
        "KeyU" | "U" => Some(Key::KeyU),
        "KeyV" | "V" => Some(Key::KeyV),
        "KeyW" | "W" => Some(Key::KeyW),
        "KeyX" | "X" => Some(Key::KeyX),
        "KeyY" | "Y" => Some(Key::KeyY),
        "KeyZ" | "Z" => Some(Key::KeyZ),

        "Digit0" | "0" => Some(Key::Num0),
        "Digit1" | "1" => Some(Key::Num1),
        "Digit2" | "2" => Some(Key::Num2),
        "Digit3" | "3" => Some(Key::Num3),
        "Digit4" | "4" => Some(Key::Num4),
        "Digit5" | "5" => Some(Key::Num5),
        "Digit6" | "6" => Some(Key::Num6),
        "Digit7" | "7" => Some(Key::Num7),
        "Digit8" | "8" => Some(Key::Num8),
        "Digit9" | "9" => Some(Key::Num9),

        "Numpad0" => Some(Key::Num0),
        "Numpad1" => Some(Key::Num1),
        "Numpad2" => Some(Key::Num2),
        "Numpad3" => Some(Key::Num3),
        "Numpad4" => Some(Key::Num4),
        "Numpad5" => Some(Key::Num5),
        "Numpad6" => Some(Key::Num6),
        "Numpad7" => Some(Key::Num7),
        "Numpad8" => Some(Key::Num8),
        "Numpad9" => Some(Key::Num9),

        "F1" => Some(Key::F1),
        "F2" => Some(Key::F2),
        "F3" => Some(Key::F3),
        "F4" => Some(Key::F4),
        "F5" => Some(Key::F5),
        "F6" => Some(Key::F6),
        "F7" => Some(Key::F7),
        "F8" => Some(Key::F8),
        "F9" => Some(Key::F9),
        "F10" => Some(Key::F10),
        "F11" => Some(Key::F11),
        "F12" => Some(Key::F12),

        "Space" => Some(Key::Space),
        "Enter" | "Return" => Some(Key::Return),
        "Tab" => Some(Key::Tab),
        "Escape" | "Esc" => Some(Key::Escape),
        "Backspace" => Some(Key::Backspace),
        "Delete" | "Del" => Some(Key::Delete),
        "Insert" | "Ins" => Some(Key::Insert),
        "Home" => Some(Key::Home),
        "End" => Some(Key::End),
        "PageUp" => Some(Key::PageUp),
        "PageDown" => Some(Key::PageDown),

        "ArrowUp" | "Up" => Some(Key::UpArrow),
        "ArrowDown" | "Down" => Some(Key::DownArrow),
        "ArrowLeft" | "Left" => Some(Key::LeftArrow),
        "ArrowRight" | "Right" => Some(Key::RightArrow),

        "Minus" | "-" => Some(Key::Minus),
        "Equal" | "=" => Some(Key::Equal),
        "BracketLeft" | "[" => Some(Key::LeftBracket),
        "BracketRight" | "]" => Some(Key::RightBracket),
        "Backslash" | "\\" => Some(Key::BackSlash),
        "Semicolon" | ";" => Some(Key::SemiColon),
        "Quote" | "'" => Some(Key::Quote),
        "Comma" | "," => Some(Key::Comma),
        "Period" | "." => Some(Key::Dot),
        "Slash" | "/" => Some(Key::Slash),
        "Backquote" | "`" => Some(Key::BackQuote),

        "CapsLock" => Some(Key::CapsLock),
        "PrintScreen" => Some(Key::PrintScreen),
        "ScrollLock" => Some(Key::ScrollLock),
        "Pause" => Some(Key::Pause),
        "NumLock" => Some(Key::NumLock),

        "NumpadAdd" => Some(Key::KpPlus),
        "NumpadSubtract" => Some(Key::KpMinus),
        "NumpadMultiply" => Some(Key::KpMultiply),
        "NumpadDivide" => Some(Key::KpDivide),
        "NumpadDecimal" | "NumpadDelete" => Some(Key::KpDelete),
        "NumpadEnter" => Some(Key::KpReturn),

        _ => None,
    }
}
