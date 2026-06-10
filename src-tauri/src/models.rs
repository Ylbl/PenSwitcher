use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RectDto {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessWindow {
    pub process_id: u32,
    pub title: String,
    pub hwnd: isize,
    pub class_name: String,
    #[serde(default)]
    pub exe_path: String,
    #[serde(default)]
    pub process_name: String,
    pub bounds: Option<RectDto>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowIdentity {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub class_name: String,
    #[serde(default)]
    pub exe_path: String,
    #[serde(default)]
    pub process_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiLocatorSegment {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub automation_id: String,
    #[serde(default)]
    pub control_type: String,
    #[serde(default)]
    pub class_name: String,
    #[serde(default)]
    pub framework_id: String,
    #[serde(default)]
    pub ordinal: usize,
    #[serde(default)]
    pub same_type_ordinal: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiLocator {
    #[serde(default)]
    pub window: WindowIdentity,
    #[serde(default)]
    pub segments: Vec<UiLocatorSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub automation_id: String,
    pub control_type: String,
    pub class_name: String,
    pub framework_id: String,
    pub process_id: u32,
    pub bounds: Option<RectDto>,
    pub depth: usize,
    pub has_children: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailRow {
    pub name: String,
    pub value: String,
    pub action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailGroup {
    pub title: String,
    pub rows: Vec<DetailRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementDetails {
    pub node: UiNode,
    pub groups: Vec<DetailGroup>,
    pub supports_invoke: bool,
    pub shortcut_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedElementPayload {
    pub process: ProcessWindow,
    pub node: UiNode,
    pub path: Vec<UiNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutItem {
    pub id: String,
    pub process: ProcessWindow,
    pub node: UiNode,
    #[serde(default)]
    pub ancestors: Vec<UiNode>,
    #[serde(default)]
    pub locator: UiLocator,
    pub hotkey: String,
    pub enabled: bool,
    pub supports_invoke: bool,
    pub status: String,
}

#[derive(Default, Serialize, Deserialize)]
pub struct ShortcutStore {
    pub items: Vec<ShortcutItem>,
}
