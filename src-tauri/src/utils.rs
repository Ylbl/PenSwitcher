use crate::models::{DetailRow, ProcessWindow, RectDto};
use uiautomation::{types::Rect, UIElement};

pub fn rect_to_dto(rect: Rect) -> RectDto {
    RectDto {
        x: rect.get_left(),
        y: rect.get_top(),
        width: rect.get_width().max(0),
        height: rect.get_height().max(0),
    }
}

pub fn format_rect(rect: Rect) -> String {
    let dto = rect_to_dto(rect);
    format!(
        "{{X={},Y={},Width={},Height={}}}",
        dto.x, dto.y, dto.width, dto.height
    )
}

pub fn row(name: &str, value: String) -> DetailRow {
    DetailRow {
        name: name.into(),
        value,
        action: None,
    }
}

pub fn yes_no(value: bool) -> String {
    if value { "Yes" } else { "No" }.into()
}

pub fn bool_property(element: &UIElement, property: uiautomation::types::UIProperty) -> String {
    element
        .get_property_value(property)
        .ok()
        .and_then(|v| v.try_into().ok())
        .map(|v: bool| yes_no(v))
        .unwrap_or_else(|| "Not Supported".into())
}

pub fn prop_string<T: ToString, F: FnOnce() -> uiautomation::Result<T>>(f: F) -> String {
    f().map(|v| v.to_string())
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "Not Supported".into())
}

pub fn id_depth(id: &str) -> usize {
    id.matches('/').count()
}

pub fn parent_id(id: &str) -> Option<String> {
    id.rsplit_once('/').map(|(p, _)| p.to_string())
}

pub fn shortcut_id(process: &ProcessWindow, node_id: &str) -> String {
    format!("{}:{node_id}", process.hwnd)
}

pub fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

pub fn win_err<E: std::fmt::Debug>(e: E) -> String {
    format!("{e:?}")
}

pub fn lock_err<E: std::fmt::Display>(e: E) -> String {
    format!("锁状态异常: {e}")
}
