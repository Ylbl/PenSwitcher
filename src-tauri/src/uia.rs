use crate::{
    models::{DetailGroup, DetailRow, ProcessWindow, ShortcutItem, UiNode},
    utils::{
        bool_property, err, format_rect, id_depth, parent_id, prop_string, rect_to_dto, row, yes_no,
    },
};
use std::{
    sync::{mpsc, Mutex, OnceLock},
    thread,
};
use uiautomation::{
    patterns::UIInvokePattern,
    types::{ControlType, Handle, Point, TreeScope, UIProperty},
    variants::Variant,
    UIAutomation, UIElement,
};

pub fn debug_uia_tree() -> Result<String, String> {
    with_uia(|uia| {
        let root = uia.get_root_element().map_err(|e| format!("root: {e}"))?;
        let cond = uia
            .create_true_condition()
            .map_err(|e| format!("cond: {e}"))?;
        let children = root
            .find_all(TreeScope::Children, &cond)
            .map_err(|e| format!("find: {e}"))?;
        let mut lines = vec![format!("桌面直接子元素: {} 个", children.len())];
        for child in children.iter().take(30) {
            let name = child.get_name().unwrap_or_default();
            let pid = child.get_process_id().unwrap_or_default();
            let ct = format!(
                "{:?}",
                child.get_control_type().unwrap_or(ControlType::Custom)
            );
            lines.push(format!("  PID={pid} Type={ct} Name=\"{name}\""));
        }
        Ok(lines.join("\n"))
    })
}

pub fn warm_uia_worker() {
    let _ = thread::spawn(|| {
        if let Err(error) = with_uia(|_| Ok(())) {
            tracing::warn!(%error, "UIA 预热失败");
        }
    });
}

pub fn automation() -> Result<UIAutomation, String> {
    UIAutomation::new().map_err(err)
}

pub fn with_uia<T: Send + 'static>(
    f: impl FnOnce(&UIAutomation) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    type Task = Box<dyn FnOnce(&UIAutomation) + Send>;
    static SENDER: OnceLock<Mutex<mpsc::Sender<Task>>> = OnceLock::new();

    let (done_tx, done_rx) = mpsc::channel::<Result<T, String>>();
    let task: Task = Box::new(move |uia| {
        let _ = done_tx.send(f(uia));
    });
    let tx = SENDER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Task>();
            thread::spawn(move || {
                tracing::info!("UIA worker 正在初始化");
                let uia = match UIAutomation::new() {
                    Ok(uia) => uia,
                    Err(error) => {
                        tracing::error!(%error, "UIA worker 初始化失败");
                        return;
                    }
                };
                tracing::info!("UIA worker 初始化完成");
                let leaked: &'static UIAutomation = Box::leak(Box::new(uia));
                for task in rx {
                    task(leaked);
                }
            });
            Mutex::new(tx)
        })
        .lock()
        .map_err(|e| format!("UIA 线程锁异常: {e}"))?
        .clone();
    tx.send(task).map_err(|_| "UIA 线程已关闭".to_string())?;
    done_rx.recv().map_err(|_| "UIA 调用失败".to_string())?
}

pub fn load_root(process: ProcessWindow) -> Result<Vec<UiNode>, String> {
    with_uia(move |uia| {
        let root = element_from_process(uia, &process)?;
        Ok(vec![node_from_element(uia, &root, "root".into(), None, 0)])
    })
}

pub fn load_children(process: ProcessWindow, node_id: String) -> Result<Vec<UiNode>, String> {
    with_uia(move |uia| {
        let root = element_from_process(uia, &process)?;
        let parent = resolve_node(uia, root, &node_id)?;
        let children = find_child_elements(uia, &parent);
        Ok(children
            .into_iter()
            .enumerate()
            .map(|(index, child)| {
                let id = format!("{node_id}/{index}");
                node_from_element(
                    uia,
                    &child,
                    id.clone(),
                    Some(node_id.clone()),
                    id_depth(&id),
                )
            })
            .collect())
    })
}

pub fn element_details(
    process: ProcessWindow,
    node_id: String,
) -> Result<(UiNode, Vec<DetailGroup>, bool), String> {
    with_uia(move |uia| {
        let root = element_from_process(uia, &process)?;
        let element = resolve_node(uia, root, &node_id)?;
        let node = node_from_element(
            uia,
            &element,
            node_id.clone(),
            parent_id(&node_id),
            id_depth(&node_id),
        );
        let supports = supports_invoke(&element);
        let groups = build_details(&element);
        Ok((node, groups, supports))
    })
}

pub fn element_bounds(
    process: ProcessWindow,
    node_id: String,
) -> Result<crate::models::RectDto, String> {
    with_uia(move |uia| {
        let root = element_from_process(uia, &process)?;
        let element = resolve_node(uia, root, &node_id)?;
        element
            .get_bounding_rectangle()
            .map(rect_to_dto)
            .map_err(err)
    })
}

pub fn shortcut_node(process: ProcessWindow, node_id: String) -> Result<(UiNode, bool), String> {
    with_uia(move |uia| {
        let root = element_from_process(uia, &process)?;
        let element = resolve_node(uia, root, &node_id)?;
        let node = node_from_element(
            uia,
            &element,
            node_id.clone(),
            parent_id(&node_id),
            id_depth(&node_id),
        );
        Ok((node, supports_invoke(&element)))
    })
}

pub fn invoke_shortcut_item(item: ShortcutItem) -> Result<(), String> {
    with_uia(move |uia| invoke_item(uia, &item))
}

pub fn invoke_item(uia: &UIAutomation, item: &ShortcutItem) -> Result<(), String> {
    let root = element_from_process(uia, &item.process)?;
    let element = resolve_node(uia, root, &item.node.id)?;
    let pattern = element.get_pattern::<UIInvokePattern>().map_err(err)?;
    tracing::info!(name = %item.node.name, hotkey = %item.hotkey, "执行快捷键 Invoke");
    pattern.invoke().map_err(err)
}

pub fn picked_payload(
    process: &ProcessWindow,
    x: i32,
    y: i32,
) -> Result<crate::models::PickedElementPayload, String> {
    let uia = automation()?;
    let element = uia.element_from_point(Point::new(x, y)).map_err(err)?;
    let walker_raw = uia.get_raw_view_walker().map_err(err)?;
    let mut ancestor = element;
    let target_pid = process.process_id;
    loop {
        if ancestor.get_process_id().unwrap_or_default() == target_pid {
            break;
        }
        ancestor = walker_raw
            .get_parent(&ancestor)
            .map_err(|_| "光标下无目标进程的元素".to_string())?;
    }
    let root = element_from_process(&uia, process)?;
    let path = build_path_from_ancestors(&uia, &root, &ancestor)?;
    let node = path
        .last()
        .cloned()
        .ok_or_else(|| "未找到元素路径".to_string())?;
    Ok(crate::models::PickedElementPayload {
        process: process.clone(),
        node,
        path,
    })
}

fn element_from_process(uia: &UIAutomation, process: &ProcessWindow) -> Result<UIElement, String> {
    if let Ok(element) = uia.element_from_handle(Handle::from(process.hwnd)) {
        if element.get_process_id().unwrap_or_default() == process.process_id {
            let children = find_child_elements(uia, &element);
            if !children.is_empty() {
                return Ok(element);
            }
        }
    }
    let root = uia.get_root_element().map_err(err)?;
    let condition = uia
        .create_property_condition(
            UIProperty::ProcessId,
            Variant::from(process.process_id as i32),
            None,
        )
        .map_err(err)?;
    let elements = root
        .find_all(TreeScope::Children, &condition)
        .map_err(err)?;
    for element in elements {
        if let Ok(hwnd_val) = element.get_native_window_handle() {
            let raw: windows::Win32::Foundation::HWND = hwnd_val.into();
            if raw.0 as isize == process.hwnd {
                return Ok(element);
            }
        }
    }
    Err(format!("未找到 HWND {} 对应的窗口元素", process.hwnd))
}

fn find_child_elements(uia: &UIAutomation, parent: &UIElement) -> Vec<UIElement> {
    let Ok(condition) = uia.create_true_condition() else {
        return Vec::new();
    };
    parent
        .find_all(TreeScope::Children, &condition)
        .unwrap_or_default()
}

fn resolve_node(uia: &UIAutomation, root: UIElement, node_id: &str) -> Result<UIElement, String> {
    if node_id == "root" {
        return Ok(root);
    }
    let mut current = root;
    for part in node_id.trim_start_matches("root/").split('/') {
        if part.is_empty() {
            continue;
        }
        let wanted = part
            .parse::<usize>()
            .map_err(|_| "无效节点路径".to_string())?;
        let children = find_child_elements(uia, &current);
        current = children
            .into_iter()
            .nth(wanted)
            .ok_or_else(|| "节点路径已失效".to_string())?;
    }
    Ok(current)
}

fn node_from_element(
    uia: &UIAutomation,
    element: &UIElement,
    id: String,
    parent_id: Option<String>,
    depth: usize,
) -> UiNode {
    let children = find_child_elements(uia, element);
    UiNode {
        id,
        parent_id,
        name: prop_string(|| element.get_name()),
        automation_id: prop_string(|| element.get_automation_id()),
        control_type: prop_string(|| element.get_control_type().map(|v| format!("{v:?}"))),
        class_name: prop_string(|| element.get_classname()),
        framework_id: prop_string(|| element.get_framework_id()),
        process_id: element.get_process_id().unwrap_or_default(),
        bounds: element.get_bounding_rectangle().ok().map(rect_to_dto),
        depth,
        has_children: !children.is_empty(),
    }
}

fn build_details(element: &UIElement) -> Vec<DetailGroup> {
    let mut groups = vec![
        DetailGroup {
            title: "Identification".into(),
            rows: vec![
                row("AutomationId", prop_string(|| element.get_automation_id())),
                row("Name", prop_string(|| element.get_name())),
                row("ClassName", prop_string(|| element.get_classname())),
                row(
                    "ControlType",
                    prop_string(|| element.get_control_type().map(|v| format!("{v:?}"))),
                ),
                row(
                    "LocalizedControlType",
                    prop_string(|| element.get_localized_control_type()),
                ),
                row("FrameworkType", prop_string(|| element.get_framework_id())),
                row("FrameworkId", prop_string(|| element.get_framework_id())),
                row(
                    "ProcessId",
                    prop_string(|| element.get_process_id().map(|v| v.to_string())),
                ),
            ],
        },
        DetailGroup {
            title: "Details".into(),
            rows: vec![
                row(
                    "IsEnabled",
                    prop_string(|| element.is_enabled().map(|v| v.to_string())),
                ),
                row(
                    "IsOffscreen",
                    prop_string(|| element.is_offscreen().map(|v| v.to_string())),
                ),
                row(
                    "BoundingRectangle",
                    prop_string(|| element.get_bounding_rectangle().map(format_rect)),
                ),
                row("HelpText", prop_string(|| element.get_help_text())),
                row(
                    "IsPassword",
                    prop_string(|| element.is_password().map(|v| v.to_string())),
                ),
                row(
                    "NativeWindowHandle",
                    prop_string(|| element.get_native_window_handle().map(|v| format!("{v}"))),
                ),
            ],
        },
    ];

    let mut pattern_rows = vec![row("Invoke", yes_no(supports_invoke(element)))];
    for (name, prop) in [
        ("Value", UIProperty::IsValuePatternAvailable),
        ("SelectionItem", UIProperty::IsSelectionItemPatternAvailable),
        (
            "LegacyIAccessible",
            UIProperty::IsLegacyIAccessiblePatternAvailable,
        ),
        ("Toggle", UIProperty::IsTogglePatternAvailable),
        ("Text", UIProperty::IsTextPatternAvailable),
    ] {
        pattern_rows.push(row(name, bool_property(element, prop)));
    }
    groups.push(DetailGroup {
        title: "Pattern Support".into(),
        rows: pattern_rows,
    });
    if supports_invoke(element) {
        groups.push(DetailGroup {
            title: "Invoke".into(),
            rows: vec![DetailRow {
                name: "Invoke".into(),
                value: "可调用".into(),
                action: Some("invoke".into()),
            }],
        });
    }
    groups
}

fn supports_invoke(element: &UIElement) -> bool {
    element
        .get_pattern::<UIInvokePattern>()
        .map(|_| true)
        .unwrap_or(false)
}

fn build_path_from_ancestors(
    uia: &UIAutomation,
    root: &UIElement,
    target: &UIElement,
) -> Result<Vec<UiNode>, String> {
    let walker = uia.get_raw_view_walker().map_err(err)?;
    let mut ancestors: Vec<UIElement> = Vec::new();
    let mut current = target.clone();
    loop {
        if uia.compare_elements(&current, root).unwrap_or(false) {
            break;
        }
        ancestors.push(current.clone());
        current = walker
            .get_parent(&current)
            .map_err(|_| "无法回溯元素路径".to_string())?;
        if ancestors.len() > 1000 {
            return Err("元素嵌套过深".into());
        }
    }
    ancestors.push(root.clone());
    ancestors.reverse();

    let root_node = node_from_element(uia, &ancestors[0], "root".into(), None, 0);
    let mut path = vec![root_node];
    for index in 1..ancestors.len() {
        let children = find_child_elements(uia, &ancestors[index - 1]);
        let child_idx = children
            .iter()
            .position(|child| {
                uia.compare_elements(child, &ancestors[index])
                    .unwrap_or(false)
            })
            .ok_or_else(|| "无法确定子元素位置索引".to_string())?;
        let id = format!("{}/{}", path[index - 1].id, child_idx);
        path.push(node_from_element(
            uia,
            &ancestors[index],
            id,
            Some(path[index - 1].id.clone()),
            path[index - 1].depth + 1,
        ));
    }
    Ok(path)
}
