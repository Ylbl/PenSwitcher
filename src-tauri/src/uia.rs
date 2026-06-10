use crate::{
    models::{
        DetailGroup, DetailRow, ProcessWindow, ShortcutItem, UiLocator, UiLocatorSegment, UiNode,
        WindowIdentity,
    },
    utils::{
        bool_property, err, format_rect, id_depth, parent_id, prop_string, rect_to_dto, row, yes_no,
    },
};
use std::{
    sync::{mpsc, Mutex, OnceLock},
    thread,
};
use uiautomation::{
    patterns::{
        UIExpandCollapsePattern, UIInvokePattern, UILegacyIAccessiblePattern, UIScrollItemPattern,
        UISelectionItemPattern, UITogglePattern,
    },
    types::{ControlType, Handle, Point, TreeScope, UIProperty},
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
        let supports = supports_any_action(&element);
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

pub fn shortcut_node_with_ancestors(
    process: ProcessWindow,
    node_id: String,
) -> Result<(Vec<UiNode>, UiLocator, bool), String> {
    with_uia(move |uia| {
        let root = element_from_process(uia, &process)?;
        let segments: Vec<&str> = node_id
            .trim_start_matches("root/")
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        let mut ancestors = Vec::new();
        let mut locator_segments = Vec::new();
        let mut current = root.clone();

        ancestors.push(node_from_element(uia, &current, "root".into(), None, 0));
        locator_segments.push(locator_segment_from_element(
            uia,
            &current,
            "root".into(),
            0,
            0,
        ));

        let mut path = String::from("root");
        for segment in segments {
            let index: usize = segment.parse().map_err(|_| "无效节点路径".to_string())?;
            let children = find_child_elements(uia, &current);
            let child = children
                .get(index)
                .cloned()
                .ok_or_else(|| "节点路径已失效".to_string())?;
            path = format!("{path}/{index}");
            let same_type_ordinal = same_type_ordinal(uia, &children, index);
            current = child;
            ancestors.push(node_from_element(
                uia,
                &current,
                path.clone(),
                parent_id(&path),
                id_depth(&path),
            ));
            locator_segments.push(locator_segment_from_element(
                uia,
                &current,
                path.clone(),
                index,
                same_type_ordinal,
            ));
        }
        let supports = supports_any_action(&current);
        Ok((
            ancestors,
            UiLocator {
                window: window_identity(&process),
                segments: locator_segments,
            },
            supports,
        ))
    })
}

pub fn invoke_shortcut_item(mut item: ShortcutItem) -> Result<(), String> {
    let locator = effective_locator(&item);
    let current_process = resolve_current_process(&item, &locator)?;
    item.process = current_process;
    with_uia(move |uia| invoke_item_with_locator(uia, &item, &locator))
}

fn invoke_item_with_locator(
    uia: &UIAutomation,
    item: &ShortcutItem,
    locator: &UiLocator,
) -> Result<(), String> {
    tracing::info!("========================================");
    tracing::info!("触发快捷键: {} → {}", item.hotkey, item.node.name);
    tracing::info!(
        segments = locator.segments.len(),
        hwnd = item.process.hwnd,
        pid = item.process.process_id,
        title = %item.process.title,
        "开始稳定定位"
    );
    for (index, segment) in locator.segments.iter().enumerate() {
        tracing::info!(
            "  [{index}] id={} name={} auto={} ctrl={} class={} ord={} same_type={}",
            segment.id,
            segment.name,
            segment.automation_id,
            segment.control_type,
            segment.class_name,
            segment.ordinal,
            segment.same_type_ordinal
        );
    }
    tracing::info!("========================================");

    let root = element_from_process(uia, &item.process)?;
    let window_root = root.clone();
    let mut current = root;
    let segments = locator_segments(locator, item);
    if segments.len() <= 1 {
        return try_activate(uia, &current, item);
    }

    for (index, segment) in segments.iter().enumerate().skip(1) {
        let is_last = index == segments.len() - 1;
        tracing::info!(
            layer = index,
            name = %segment.name,
            automation_id = %segment.automation_id,
            control_type = %segment.control_type,
            "定位下一层"
        );

        let mut found = find_best_child_by_segment(uia, &current, segment).or_else(|| {
            if has_segment_identity(segment) {
                tracing::warn!(layer = index, "直接子元素未命中，尝试有限深度搜索");
                search_descendants_by_segment(uia, &current, segment, 4)
            } else {
                None
            }
        });

        if found.is_none()
            && should_activate_named_navigation(segment)
            && activate_named_navigation_target(uia, &window_root, segment)
        {
            tracing::info!(layer = index, name = %segment.name, "已激活同名导航入口，短轮询重试当前层");
            found = wait_for_segment_after_navigation(uia, &current, segment);
        }

        let Some(found) = found else {
            log_children(uia, &current, segment);
            return Err(format!(
                "定位中断: 第 {index} 层未找到 {} {}",
                segment.control_type, segment.name
            ));
        };

        if !is_last {
            prepare_for_navigation(&found);
        }
        current = found;
    }

    tracing::info!("定位完成，激活最终目标节点");
    try_activate(uia, &current, item)
}

fn effective_locator(item: &ShortcutItem) -> UiLocator {
    if !item.locator.segments.is_empty() {
        return item.locator.clone();
    }

    let segments = if !item.ancestors.is_empty() {
        item.ancestors
            .iter()
            .map(locator_segment_from_node)
            .collect()
    } else {
        vec![locator_segment_from_node(&item.node)]
    };

    UiLocator {
        window: window_identity(&item.process),
        segments,
    }
}

fn locator_segments(locator: &UiLocator, item: &ShortcutItem) -> Vec<UiLocatorSegment> {
    if !locator.segments.is_empty() {
        locator.segments.clone()
    } else if !item.ancestors.is_empty() {
        item.ancestors
            .iter()
            .map(locator_segment_from_node)
            .collect()
    } else {
        vec![locator_segment_from_node(&item.node)]
    }
}

fn resolve_current_process(
    item: &ShortcutItem,
    locator: &UiLocator,
) -> Result<ProcessWindow, String> {
    let windows = crate::windows_api::list_windows()?;
    let identity = if has_window_identity(&locator.window) {
        locator.window.clone()
    } else {
        window_identity(&item.process)
    };

    let expected_process_name = expected_process_name(item, &identity);
    let mut best: Option<(i32, ProcessWindow)> = None;
    for candidate in windows {
        if meaningful(&expected_process_name)
            && !same_text(&expected_process_name, &candidate.process_name)
        {
            continue;
        }
        let score = score_window(&item.process, &identity, &candidate, &expected_process_name);
        if score > 0
            && best
                .as_ref()
                .map(|(best_score, _)| score > *best_score)
                .unwrap_or(true)
        {
            best = Some((score, candidate));
        }
    }

    let Some((score, candidate)) = best else {
        return Err("没有找到匹配的目标窗口".into());
    };

    if score < 80 {
        return Err(format!(
            "目标窗口匹配度过低: score={score}, title={}",
            candidate.title
        ));
    }

    tracing::info!(
        score,
        old_hwnd = item.process.hwnd,
        old_pid = item.process.process_id,
        new_hwnd = candidate.hwnd,
        new_pid = candidate.process_id,
        title = %candidate.title,
        class_name = %candidate.class_name,
        process_name = %candidate.process_name,
        "已解析当前目标窗口"
    );
    Ok(candidate)
}

fn score_window(
    old: &ProcessWindow,
    identity: &WindowIdentity,
    candidate: &ProcessWindow,
    expected_process_name: &str,
) -> i32 {
    if candidate.process_id == std::process::id() {
        return 0;
    }

    let mut score = 0;
    if old.hwnd == candidate.hwnd && old.process_id == candidate.process_id {
        score += 220;
    }
    if meaningful(expected_process_name)
        && same_text(expected_process_name, &candidate.process_name)
    {
        score += 180;
    }
    if meaningful(&identity.process_name)
        && same_text(&identity.process_name, &candidate.process_name)
    {
        score += 120;
    }
    if meaningful(&identity.exe_path) && same_text(&identity.exe_path, &candidate.exe_path) {
        score += 90;
    }
    if meaningful(&identity.class_name) && identity.class_name == candidate.class_name {
        score += 50;
    }
    if meaningful(&identity.title) {
        if identity.title == candidate.title {
            score += 45;
        } else if soft_text_match(&identity.title, &candidate.title) {
            score += 25;
        }
    }
    if looks_like_same_app(&identity.title, &candidate.title)
        || looks_like_same_app(&identity.process_name, &candidate.process_name)
    {
        score += 25;
    }
    score
}

fn expected_process_name(item: &ShortcutItem, identity: &WindowIdentity) -> String {
    if meaningful(&identity.process_name) {
        return identity.process_name.clone();
    }
    if meaningful(&item.process.process_name) {
        return item.process.process_name.clone();
    }

    let title = format!("{} {}", identity.title, item.process.title).to_lowercase();
    if title.contains("onenote") || title.contains("one note") {
        return "ONENOTE.EXE".into();
    }
    String::new()
}

fn has_window_identity(identity: &WindowIdentity) -> bool {
    meaningful(&identity.exe_path)
        || meaningful(&identity.process_name)
        || meaningful(&identity.class_name)
        || meaningful(&identity.title)
}

fn find_best_child_by_segment(
    uia: &UIAutomation,
    parent: &UIElement,
    segment: &UiLocatorSegment,
) -> Option<UIElement> {
    let children = find_child_elements(uia, parent);
    best_match_in_children(uia, &children, segment)
}

fn best_match_in_children(
    uia: &UIAutomation,
    children: &[UIElement],
    segment: &UiLocatorSegment,
) -> Option<UIElement> {
    let mut best: Option<(i32, UIElement)> = None;
    for (index, child) in children.iter().enumerate() {
        let same_type = same_type_ordinal(uia, children, index);
        let score = score_element(uia, child, segment, index, same_type);
        if score >= segment_threshold(segment)
            && best
                .as_ref()
                .map(|(best_score, _)| score > *best_score)
                .unwrap_or(true)
        {
            best = Some((score, child.clone()));
        }
    }
    best.map(|(_, element)| element)
}

fn search_descendants_by_segment(
    uia: &UIAutomation,
    root: &UIElement,
    segment: &UiLocatorSegment,
    max_depth: usize,
) -> Option<UIElement> {
    if max_depth == 0 {
        return None;
    }
    let children = find_child_elements(uia, root);
    if let Some(found) = best_match_in_children(uia, &children, segment) {
        return Some(found);
    }
    for child in &children {
        if let Some(found) = search_descendants_by_segment(uia, child, segment, max_depth - 1) {
            return Some(found);
        }
    }
    None
}

fn activate_named_navigation_target(
    uia: &UIAutomation,
    root: &UIElement,
    segment: &UiLocatorSegment,
) -> bool {
    if !meaningful(&segment.name) {
        return false;
    }

    let mut candidates = Vec::new();
    collect_exact_name_candidates(uia, root, &segment.name, 10, &mut candidates);
    let mut best: Option<(i32, UIElement)> = None;
    for candidate in candidates {
        let score = navigation_candidate_score(&candidate, segment);
        if score > 0
            && best
                .as_ref()
                .map(|(best_score, _)| score > *best_score)
                .unwrap_or(true)
        {
            best = Some((score, candidate));
        }
    }

    let Some((score, element)) = best else {
        tracing::warn!(name = %segment.name, "未找到同名导航入口");
        return false;
    };

    let control_type = element
        .get_control_type()
        .map(|value| format!("{value:?}"))
        .unwrap_or_default();
    tracing::info!(name = %segment.name, control_type, score, "激活同名导航入口");
    activate_navigation_element(&element)
}

fn wait_for_segment_after_navigation(
    uia: &UIAutomation,
    parent: &UIElement,
    segment: &UiLocatorSegment,
) -> Option<UIElement> {
    const MAX_WAIT_MS: u64 = 60;
    const STEP_MS: u64 = 5;

    let mut elapsed = 0;
    loop {
        let found = find_best_child_by_segment(uia, parent, segment).or_else(|| {
            if has_segment_identity(segment) {
                search_descendants_by_segment(uia, parent, segment, 4)
            } else {
                None
            }
        });
        if found.is_some() || elapsed >= MAX_WAIT_MS {
            return found;
        }
        std::thread::sleep(std::time::Duration::from_millis(STEP_MS));
        elapsed += STEP_MS;
    }
}

fn should_activate_named_navigation(segment: &UiLocatorSegment) -> bool {
    meaningful(&segment.name)
        && segment.control_type == "Custom"
        && segment.class_name == "NetUIOrderedGroup"
}

fn collect_exact_name_candidates(
    uia: &UIAutomation,
    root: &UIElement,
    name: &str,
    max_depth: usize,
    output: &mut Vec<UIElement>,
) {
    if max_depth == 0 {
        return;
    }
    let children = find_child_elements(uia, root);
    for child in &children {
        if child.get_name().unwrap_or_default() == name {
            output.push(child.clone());
        }
    }
    for child in &children {
        collect_exact_name_candidates(uia, child, name, max_depth - 1, output);
    }
}

fn navigation_candidate_score(element: &UIElement, segment: &UiLocatorSegment) -> i32 {
    let control_type = element
        .get_control_type()
        .map(|value| format!("{value:?}"))
        .unwrap_or_default();
    let class_name = element.get_classname().unwrap_or_default();
    let mut score = 1;

    match control_type.as_str() {
        "TabItem" => score += 140,
        "MenuItem" => score += 110,
        "Button" | "SplitButton" => score += 80,
        "Custom" => score += 65,
        "Text" => score -= 40,
        _ => score += 10,
    }
    if class_name.contains("NetUI") || class_name.contains("Mso") {
        score += 35;
    }
    if meaningful(&segment.class_name) && class_name == segment.class_name {
        score -= 25;
    }
    score
}

fn activate_navigation_element(element: &UIElement) -> bool {
    if let Ok(pattern) = element.get_pattern::<UISelectionItemPattern>() {
        if pattern.select().is_ok() {
            return true;
        }
    }
    if let Ok(pattern) = element.get_pattern::<UIInvokePattern>() {
        if pattern.invoke().is_ok() {
            return true;
        }
    }
    if let Ok(pattern) = element.get_pattern::<UILegacyIAccessiblePattern>() {
        let _ = pattern.select(3);
        if pattern.do_default_action().is_ok() {
            return true;
        }
    }
    element.click().is_ok()
}

fn score_element(
    uia: &UIAutomation,
    element: &UIElement,
    segment: &UiLocatorSegment,
    ordinal: usize,
    same_type_ordinal: usize,
) -> i32 {
    let mut score = 0;
    let control_type = prop_string(|| element.get_control_type().map(|v| format!("{v:?}")));
    let name = prop_string(|| element.get_name());
    let automation_id = prop_string(|| element.get_automation_id());
    let class_name = prop_string(|| element.get_classname());
    let framework_id = prop_string(|| element.get_framework_id());

    if meaningful(&segment.automation_id) && automation_id != segment.automation_id {
        return -200;
    }
    if meaningful(&segment.name) && !soft_text_match(&name, &segment.name) {
        return -160;
    }
    if meaningful(&segment.control_type) && control_type != segment.control_type {
        return -120;
    }

    if meaningful(&segment.automation_id) {
        if automation_id == segment.automation_id {
            score += 120;
        }
    }
    if meaningful(&segment.name) {
        if name == segment.name {
            score += 90;
        } else if soft_text_match(&name, &segment.name) {
            score += 35;
        }
    }
    if meaningful(&segment.control_type) {
        if control_type == segment.control_type {
            score += 40;
        }
    }
    if meaningful(&segment.class_name) && class_name == segment.class_name {
        score += 25;
    }
    if meaningful(&segment.framework_id) && framework_id == segment.framework_id {
        score += 15;
    }
    if ordinal == segment.ordinal {
        score += 8;
    }
    if same_type_ordinal == segment.same_type_ordinal {
        score += 18;
    }

    tracing::trace!(
        score,
        ordinal,
        same_type_ordinal,
        target_name = %segment.name,
        name = %name,
        target_type = %segment.control_type,
        control_type = %control_type,
        "UIA 子元素匹配评分"
    );
    let _ = uia;
    score
}

fn segment_threshold(segment: &UiLocatorSegment) -> i32 {
    if meaningful(&segment.automation_id) {
        90
    } else if meaningful(&segment.name) {
        70
    } else {
        45
    }
}

fn has_segment_identity(segment: &UiLocatorSegment) -> bool {
    meaningful(&segment.automation_id) || meaningful(&segment.name)
}

fn prepare_for_navigation(element: &UIElement) {
    if let Ok(pattern) = element.get_pattern::<UIScrollItemPattern>() {
        let _ = pattern.scroll_into_view();
    }
    if let Ok(pattern) = element.get_pattern::<UISelectionItemPattern>() {
        let _ = pattern.select();
    }
    if let Ok(pattern) = element.get_pattern::<UIExpandCollapsePattern>() {
        let _ = pattern.expand();
    }
    if let Ok(pattern) = element.get_pattern::<UILegacyIAccessiblePattern>() {
        let _ = pattern.select(3);
    }
}

fn log_children(uia: &UIAutomation, parent: &UIElement, target: &UiLocatorSegment) {
    let children = find_child_elements(uia, parent);
    tracing::warn!(
        child_count = children.len(),
        target_name = %target.name,
        target_auto = %target.automation_id,
        target_type = %target.control_type,
        "定位失败，输出当前父节点子元素"
    );
    for (index, child) in children.iter().enumerate().take(80) {
        let name = child.get_name().unwrap_or_default();
        let automation_id = child.get_automation_id().unwrap_or_default();
        let control_type = child
            .get_control_type()
            .map(|v| format!("{v:?}"))
            .unwrap_or_default();
        tracing::warn!(index, name, automation_id, control_type, "候选子元素");
    }
}

fn try_activate(
    _uia: &UIAutomation,
    element: &UIElement,
    _item: &ShortcutItem,
) -> Result<(), String> {
    if let Ok(pattern) = element.get_pattern::<UIScrollItemPattern>() {
        let _ = pattern.scroll_into_view();
    }
    if let Ok(pattern) = element.get_pattern::<UISelectionItemPattern>() {
        return pattern.select().map_err(|e| format!("Select: {e}"));
    }
    if let Ok(pattern) = element.get_pattern::<UIInvokePattern>() {
        return pattern.invoke().map_err(|e| format!("Invoke: {e}"));
    }
    if let Ok(pattern) = element.get_pattern::<UITogglePattern>() {
        return pattern.toggle().map_err(|e| format!("Toggle: {e}"));
    }
    if let Ok(pattern) = element.get_pattern::<UIExpandCollapsePattern>() {
        return pattern.expand().map_err(|e| format!("Expand: {e}"));
    }
    if let Ok(pattern) = element.get_pattern::<UILegacyIAccessiblePattern>() {
        let _ = pattern.select(3);
        return pattern
            .do_default_action()
            .map_err(|e| format!("DoDefaultAction: {e}"));
    }
    if element.click().is_ok() {
        return Ok(());
    }
    Err("元素不支持任何激活方式".into())
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
    tracing::debug!(hwnd = process.hwnd, pid = process.process_id, title = %process.title, "查找窗口元素");
    if let Some(element) = element_from_exact_process(uia, process) {
        return Ok(element);
    }

    tracing::error!(
        hwnd = process.hwnd,
        pid = process.process_id,
        "未找到窗口元素"
    );
    Err(format!("未找到当前 HWND {} 对应的窗口元素", process.hwnd))
}

fn element_from_exact_process(uia: &UIAutomation, process: &ProcessWindow) -> Option<UIElement> {
    if let Ok(element) = uia.element_from_handle(Handle::from(process.hwnd)) {
        if element.get_process_id().unwrap_or_default() == process.process_id {
            let children = find_child_elements(uia, &element);
            if !children.is_empty() {
                tracing::debug!(
                    hwnd = process.hwnd,
                    children = children.len(),
                    "通过 HWND 找到窗口元素"
                );
                return Some(element);
            }
            tracing::debug!(hwnd = process.hwnd, "HWND 匹配但无子元素，回退搜索");
        }
    }

    tracing::debug!(
        hwnd = process.hwnd,
        pid = process.process_id,
        "旧 HWND/PID 未命中，跳过旧 PID UIA 扫描"
    );
    None
}

fn looks_like_same_app(old: &str, current: &str) -> bool {
    let old = normalize_title(old);
    let current = normalize_title(current);
    ["onenote", "one note"]
        .iter()
        .any(|needle| old.contains(needle) && current.contains(needle))
}

fn normalize_title(value: &str) -> String {
    value.trim().to_lowercase()
}

fn meaningful(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value != "Not Supported"
}

fn same_text(left: &str, right: &str) -> bool {
    meaningful(left) && meaningful(right) && left.eq_ignore_ascii_case(right)
}

fn soft_text_match(left: &str, right: &str) -> bool {
    let left = normalize_title(left);
    let right = normalize_title(right);
    meaningful(&left)
        && meaningful(&right)
        && (left == right || left.contains(&right) || right.contains(&left))
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

fn locator_segment_from_element(
    _uia: &UIAutomation,
    element: &UIElement,
    id: String,
    ordinal: usize,
    same_type_ordinal: usize,
) -> UiLocatorSegment {
    UiLocatorSegment {
        id,
        name: prop_string(|| element.get_name()),
        automation_id: prop_string(|| element.get_automation_id()),
        control_type: prop_string(|| element.get_control_type().map(|v| format!("{v:?}"))),
        class_name: prop_string(|| element.get_classname()),
        framework_id: prop_string(|| element.get_framework_id()),
        ordinal,
        same_type_ordinal,
    }
}

fn locator_segment_from_node(node: &UiNode) -> UiLocatorSegment {
    UiLocatorSegment {
        id: node.id.clone(),
        name: node.name.clone(),
        automation_id: node.automation_id.clone(),
        control_type: node.control_type.clone(),
        class_name: node.class_name.clone(),
        framework_id: node.framework_id.clone(),
        ordinal: node
            .id
            .rsplit('/')
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default(),
        same_type_ordinal: 0,
    }
}

fn same_type_ordinal(_uia: &UIAutomation, siblings: &[UIElement], index: usize) -> usize {
    let Some(target) = siblings.get(index) else {
        return 0;
    };
    let target_type = prop_string(|| target.get_control_type().map(|v| format!("{v:?}")));
    siblings
        .iter()
        .take(index)
        .filter(|sibling| {
            prop_string(|| sibling.get_control_type().map(|v| format!("{v:?}"))) == target_type
        })
        .count()
}

fn window_identity(process: &ProcessWindow) -> WindowIdentity {
    WindowIdentity {
        title: process.title.clone(),
        class_name: process.class_name.clone(),
        exe_path: process.exe_path.clone(),
        process_name: process.process_name.clone(),
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

    let mut pattern_rows = vec![row("Invoke", yes_no(supports_any_action(element)))];
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
    if element.get_pattern::<UIInvokePattern>().is_ok() {
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

fn supports_any_action(element: &UIElement) -> bool {
    element.get_pattern::<UISelectionItemPattern>().is_ok()
        || element.get_pattern::<UIInvokePattern>().is_ok()
        || element.get_pattern::<UITogglePattern>().is_ok()
        || element.get_pattern::<UIExpandCollapsePattern>().is_ok()
        || element.get_pattern::<UILegacyIAccessiblePattern>().is_ok()
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
