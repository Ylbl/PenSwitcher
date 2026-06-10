mod commands;
mod hotkeys;
mod logging;
mod models;
mod overlay;
mod picker;
mod state;
mod storage;
mod uia;
mod utils;
mod windows_api;

use crate::{
    hotkeys::{rebuild_hotkeys, start_listener},
    logging::init_logging,
    models::ShortcutStore,
    overlay::Overlay,
    state::AppState,
    storage::load_shortcuts,
    uia::warm_uia_worker,
};
use std::sync::Mutex;
use tauri::Manager;

pub const PICK_EVENT: &str = "uia-picked";
pub const HOTKEY_EVENT: &str = "shortcut-invoked";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();
    tracing::info!("PenSwitcher 启动");

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let shortcuts = match load_shortcuts(app.handle()) {
                Ok(shortcuts) => shortcuts,
                Err(error) => {
                    tracing::error!(%error, "加载快捷键配置失败，使用空配置");
                    ShortcutStore::default()
                }
            };
            app.manage(AppState {
                overlay: Mutex::new(Overlay::default()),
                shortcuts: Mutex::new(shortcuts),
                hotkeys: Mutex::new(Default::default()),
                pick_worker: Mutex::new(None),
            });

            warm_uia_worker();

            let state = app.state::<AppState>();
            if let Err(error) = rebuild_hotkeys(&state) {
                tracing::error!(%error, "恢复快捷键监听失败");
            }

            {
                let registry = state.hotkeys.lock().map_err(|e| e.to_string())?;
                start_listener(app.handle().clone(), registry.sequences.clone());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_process_windows,
            commands::debug_uia,
            commands::load_tree_root,
            commands::load_children,
            commands::get_element_details,
            commands::highlight_element,
            commands::hide_overlay,
            commands::preview_window_under_cursor,
            commands::finish_window_pick,
            commands::start_element_pick,
            commands::cancel_element_pick,
            commands::set_shortcut_membership,
            commands::list_shortcuts,
            commands::remove_shortcut,
            commands::set_shortcut_hotkey,
            commands::invoke_shortcut,
        ])
        .run(tauri::generate_context!());

    if let Err(error) = result {
        tracing::error!(%error, "Tauri 运行失败");
        panic!("error while running tauri application: {error}");
    }
}
