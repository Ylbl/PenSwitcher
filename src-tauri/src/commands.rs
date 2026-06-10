use crate::{
    hotkeys::rebuild_hotkeys,
    models::{ElementDetails, ProcessWindow, ShortcutItem, UiNode},
    picker,
    state::AppState,
    storage::persist_shortcuts,
    uia,
    utils::{lock_err, shortcut_id},
    windows_api,
};
use tauri::AppHandle;

#[tauri::command]
pub fn list_process_windows() -> Result<Vec<ProcessWindow>, String> {
    windows_api::list_windows()
}

#[tauri::command]
pub fn debug_uia() -> Result<String, String> {
    uia::debug_uia_tree()
}

#[tauri::command]
pub fn load_tree_root(process: ProcessWindow) -> Result<Vec<UiNode>, String> {
    uia::load_root(process)
}

#[tauri::command]
pub fn load_children(process: ProcessWindow, node_id: String) -> Result<Vec<UiNode>, String> {
    uia::load_children(process, node_id)
}

#[tauri::command]
pub fn get_element_details(
    state: tauri::State<AppState>,
    process: ProcessWindow,
    node_id: String,
) -> Result<ElementDetails, String> {
    let id = shortcut_id(&process, &node_id);
    let shortcut_enabled = state
        .shortcuts
        .lock()
        .map_err(lock_err)?
        .items
        .iter()
        .any(|item| item.id == id);
    let (node, groups, supports_invoke) = uia::element_details(process, node_id)?;
    Ok(ElementDetails {
        node,
        groups,
        supports_invoke,
        shortcut_enabled,
    })
}

#[tauri::command]
pub fn highlight_element(
    state: tauri::State<AppState>,
    process: ProcessWindow,
    node_id: String,
) -> Result<(), String> {
    let rect = uia::element_bounds(process, node_id)?;
    state.overlay.lock().map_err(lock_err)?.show(rect)
}

#[tauri::command]
pub fn hide_overlay(state: tauri::State<AppState>) -> Result<(), String> {
    state.overlay.lock().map_err(lock_err)?.hide();
    Ok(())
}

#[tauri::command]
pub fn preview_window_under_cursor(
    state: tauri::State<AppState>,
) -> Result<Option<ProcessWindow>, String> {
    let candidate = windows_api::window_under_cursor()?;
    if let Some(process) = &candidate {
        if let Some(bounds) = &process.bounds {
            state
                .overlay
                .lock()
                .map_err(lock_err)?
                .show(bounds.clone())?;
        }
    }
    Ok(candidate)
}

#[tauri::command]
pub fn finish_window_pick(state: tauri::State<AppState>) -> Result<Option<ProcessWindow>, String> {
    let candidate = windows_api::window_under_cursor()?;
    state.overlay.lock().map_err(lock_err)?.hide();
    Ok(candidate)
}

#[tauri::command]
pub fn start_element_pick(
    app: AppHandle,
    state: tauri::State<AppState>,
    process: ProcessWindow,
) -> Result<(), String> {
    picker::start_pick(app, &state, process)
}

#[tauri::command]
pub fn cancel_element_pick(state: tauri::State<AppState>) -> Result<(), String> {
    picker::cancel_pick_worker(&state)
}

#[tauri::command]
pub fn set_shortcut_membership(
    state: tauri::State<AppState>,
    app: AppHandle,
    process: ProcessWindow,
    node_id: String,
    checked: bool,
) -> Result<Vec<ShortcutItem>, String> {
    {
        let mut store = state.shortcuts.lock().map_err(lock_err)?;
        let id = shortcut_id(&process, &node_id);
        if checked {
            if !store.items.iter().any(|item| item.id == id) {
                let (ancestors, supports) = uia::shortcut_node_with_ancestors(process.clone(), node_id)?;
                let node = ancestors.last().cloned().unwrap();
                store.items.push(ShortcutItem {
                    id,
                    process,
                    node,
                    ancestors,
                    hotkey: String::new(),
                    enabled: false,
                    supports_invoke: supports,
                    status: if supports {
                        "未绑定".into()
                    } else {
                        "无法调用".into()
                    },
                });
            }
        } else {
            store.items.retain(|item| item.id != id);
        }
        persist_shortcuts(&app, &store)?;
    }
    rebuild_hotkeys(&state)?;
    Ok(state.shortcuts.lock().map_err(lock_err)?.items.clone())
}

#[tauri::command]
pub fn list_shortcuts(state: tauri::State<AppState>) -> Result<Vec<ShortcutItem>, String> {
    Ok(state.shortcuts.lock().map_err(lock_err)?.items.clone())
}

#[tauri::command]
pub fn remove_shortcut(
    state: tauri::State<AppState>,
    app: AppHandle,
    item_id: String,
) -> Result<Vec<ShortcutItem>, String> {
    {
        let mut store = state.shortcuts.lock().map_err(lock_err)?;
        store.items.retain(|item| item.id != item_id);
        persist_shortcuts(&app, &store)?;
    }
    rebuild_hotkeys(&state)?;
    Ok(state.shortcuts.lock().map_err(lock_err)?.items.clone())
}

#[tauri::command]
pub fn set_shortcut_hotkey(
    state: tauri::State<AppState>,
    app: AppHandle,
    item_id: String,
    hotkey: String,
) -> Result<Vec<ShortcutItem>, String> {
    {
        let mut store = state.shortcuts.lock().map_err(lock_err)?;
        let normalized = hotkey.trim().to_string();
        if !normalized.is_empty()
            && store
                .items
                .iter()
                .any(|item| item.id != item_id && item.hotkey.eq_ignore_ascii_case(&normalized))
        {
            return Err("快捷键已被其他元素占用".into());
        }
        if let Some(item) = store.items.iter_mut().find(|item| item.id == item_id) {
            item.hotkey = normalized;
        }
        persist_shortcuts(&app, &store)?;
    }
    rebuild_hotkeys(&state)?;
    Ok(state.shortcuts.lock().map_err(lock_err)?.items.clone())
}

#[tauri::command]
pub fn invoke_shortcut(state: tauri::State<AppState>, item_id: String) -> Result<(), String> {
    tracing::info!(%item_id, "invoke_shortcut 命令");
    let item = state
        .shortcuts
        .lock()
        .map_err(lock_err)?
        .items
        .iter()
        .find(|item| item.id == item_id)
        .cloned()
        .ok_or_else(|| "快捷操作不存在".to_string())?;
    tracing::info!(
        %item_id,
        node_name = %item.node.name,
        node_id = %item.node.id,
        ancestors_len = item.ancestors.len(),
        "开始调用 invoke_shortcut_item"
    );
    uia::invoke_shortcut_item(item)
}
