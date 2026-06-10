use crate::{hotkeys::HotkeyRegistry, models::ShortcutStore, overlay::Overlay, picker::PickWorker};
use std::sync::Mutex;

pub struct AppState {
    pub overlay: Mutex<Overlay>,
    pub shortcuts: Mutex<ShortcutStore>,
    pub hotkeys: Mutex<HotkeyRegistry>,
    pub pick_worker: Mutex<Option<PickWorker>>,
}
