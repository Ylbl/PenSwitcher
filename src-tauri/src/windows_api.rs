use crate::{
    models::ProcessWindow,
    utils::{rect_to_dto, win_err},
};
use std::collections::HashSet;
use uiautomation::types::Rect;
use windows::{
    core::{BOOL, PWSTR},
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT},
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetAncestor, GetClassNameW, GetCursorPos, GetWindow, GetWindowRect,
            GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
            WindowFromPoint, GA_ROOT, GW_OWNER,
        },
    },
};

pub fn list_windows() -> Result<Vec<ProcessWindow>, String> {
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let result = unsafe { &mut *(lparam.0 as *mut Vec<ProcessWindow>) };
        if let Some(process) = process_from_top_level_hwnd(hwnd) {
            result.push(process);
        }
        true.into()
    }

    let mut result: Vec<ProcessWindow> = Vec::new();
    unsafe {
        EnumWindows(Some(enum_proc), LPARAM(&mut result as *mut _ as isize)).map_err(win_err)?;
    }
    let mut seen = HashSet::new();
    result.retain(|item| seen.insert(item.hwnd));
    result.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    tracing::debug!(count = result.len(), "已枚举顶层窗口");
    Ok(result)
}

pub fn window_under_cursor() -> Result<Option<ProcessWindow>, String> {
    unsafe {
        let mut point = POINT::default();
        GetCursorPos(&mut point).map_err(win_err)?;
        let hwnd = WindowFromPoint(point);
        if hwnd.0.is_null() {
            return Ok(None);
        }
        let root = GetAncestor(hwnd, GA_ROOT);
        if root.0.is_null() || !IsWindowVisible(root).as_bool() {
            return Ok(None);
        }
        let mut pid = 0;
        GetWindowThreadProcessId(root, Some(&mut pid));
        if pid == std::process::id() {
            return Ok(None);
        }
        Ok(process_from_top_level_hwnd(root).or_else(|| Some(process_from_hwnd(root, pid))))
    }
}

fn process_from_top_level_hwnd(hwnd: HWND) -> Option<ProcessWindow> {
    unsafe {
        if hwnd.0.is_null() || !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        if GetWindow(hwnd, GW_OWNER).is_ok() {
            return None;
        }
        let title_len = GetWindowTextLengthW(hwnd);
        if title_len <= 0 {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return None;
        }
        let process = process_from_hwnd(hwnd, pid);
        if process.title.trim().is_empty() {
            return None;
        }
        Some(process)
    }
}

fn process_from_hwnd(hwnd: HWND, process_id: u32) -> ProcessWindow {
    unsafe {
        let title_len = GetWindowTextLengthW(hwnd);
        let mut title_buf = vec![0u16; (title_len + 1).max(1) as usize];
        let copied = GetWindowTextW(hwnd, &mut title_buf);
        let title = String::from_utf16_lossy(&title_buf[..copied as usize]);

        let mut class_buf = vec![0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_buf);
        let class_name = String::from_utf16_lossy(&class_buf[..class_len as usize]);

        let mut rect = RECT::default();
        let bounds = GetWindowRect(hwnd, &mut rect)
            .ok()
            .map(|_| rect_to_dto(Rect::from(rect)));
        let exe_path = process_image_path(process_id).unwrap_or_default();
        let process_name = process_name_from_path(&exe_path);

        ProcessWindow {
            process_id,
            title,
            hwnd: hwnd.0 as isize,
            class_name,
            exe_path,
            process_name,
            bounds,
        }
    }
}

fn process_image_path(process_id: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id).ok()?;
        let mut size = 32768u32;
        let mut buffer = vec![0u16; size as usize];
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        Some(String::from_utf16_lossy(&buffer[..size as usize]))
    }
}

fn process_name_from_path(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}
