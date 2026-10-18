//! Windows UI Automation, rooted at one renderer-owned HWND. Never
//! starts at the desktop root or enumerates other applications' windows.
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    SAFEARRAY,
};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetDim, SafeArrayGetLBound, SafeArrayGetUBound,
    SafeArrayGetVartype, SafeArrayUnaccessData,
};
use windows::Win32::System::Variant::VT_I4;
use windows::Win32::UI::Accessibility::*;

mod collections;

pub const OPERATIONS: &[&str] = &[
    "snapshot",
    "inspect",
    "text",
    "text-selection",
    "focus",
    "invoke",
    "set-value",
    "set-range-value",
    "toggle",
    "expand",
    "collapse",
    "select",
    "add-to-selection",
    "remove-from-selection",
    "scroll",
    "set-scroll-percent",
    "scroll-into-view",
    "realize",
    "legacy-default-action",
    "multiple-view-set",
    "window-state",
    "window-close",
    "move",
    "resize",
    "rotate",
    "text-select",
    "grid-item",
    "table-headers",
    "table-item-headers",
    "selection",
    "containing-grid",
    "selection-container",
];

struct Com;
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

fn connect(
    hwnd: isize,
    pid: u32,
    params: &Value,
) -> Result<
    (
        Com,
        IUIAutomation,
        IUIAutomationTreeWalker,
        IUIAutomationElement,
    ),
    String,
> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|e| e.to_string())?;
    let com = Com;
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| e.to_string())?;
    if let Ok(client) = automation.cast::<IUIAutomation2>() {
        unsafe {
            client.SetConnectionTimeout(1000).ok();
            client.SetTransactionTimeout(1500).ok();
        }
    }
    let walker = unsafe {
        match params["view"].as_str().unwrap_or("raw") {
            "raw" => automation.RawViewWalker(),
            "control" => automation.ControlViewWalker(),
            "content" => automation.ContentViewWalker(),
            _ => return Err("UIA view must be raw, control or content".into()),
        }
    }
    .map_err(|e| e.to_string())?;
    let root =
        unsafe { automation.ElementFromHandle(HWND(hwnd as *mut _)) }.map_err(|e| e.to_string())?;
    validate_root(&root, hwnd, pid)?;
    Ok((com, automation, walker, root))
}

fn validate_root(root: &IUIAutomationElement, hwnd: isize, pid: u32) -> Result<(), String> {
    if unsafe { root.CurrentProcessId() }.map_err(|e| e.to_string())? as u32 != pid
        || unsafe { root.CurrentNativeWindowHandle() }
            .map_err(|e| e.to_string())?
            .0 as isize
            != hwnd
    {
        return Err("native accessibility window was replaced".into());
    }
    Ok(())
}

pub(crate) struct NativeChildren {
    pub windows: Vec<(isize, u32)>,
    pub truncated: bool,
}
/// Read only immediate raw provider children, without names, values or a
/// desktop-root search. Windows can broker a modal in a separate process.
pub(crate) fn native_children(
    hwnd: isize,
    pid: u32,
    deadline: Instant,
) -> Result<NativeChildren, String> {
    let (_com, _automation, walker, root) = connect(hwnd, pid, &json!({}))?;
    let mut child = collections::walk(&walker, &root, collections::WalkerStep::FirstChild)
        .map_err(|error| error.to_string())?;
    let mut result = NativeChildren {
        windows: Vec::new(),
        truncated: false,
    };
    let mut visited = 0;
    while let Some(element) = child {
        if visited >= 256 || Instant::now() >= deadline {
            result.truncated = true;
            break;
        }
        visited += 1;
        unsafe {
            let native = element
                .CurrentNativeWindowHandle()
                .map_err(|error| error.to_string())?
                .0 as isize;
            let process = element
                .CurrentProcessId()
                .map_err(|error| error.to_string())? as u32;
            if native != 0 && native != hwnd {
                let mut actual = 0;
                windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
                    native as _,
                    &mut actual,
                );
                if process != 0 && process == actual {
                    result.windows.push((native, process));
                }
            }
        }
        child = collections::walk(&walker, &element, collections::WalkerStep::NextSibling)
            .map_err(|error| error.to_string())?;
    }
    validate_root(&root, hwnd, pid)?;
    Ok(result)
}

pub fn snapshot(hwnd: isize, pid: u32, params: &Value) -> Result<Value, String> {
    let (_com, _automation, walker, root) = connect(hwnd, pid, params)?;
    let limit = params["max_nodes"].as_u64().unwrap_or(1000).clamp(1, 5000) as usize;
    let depth_limit = params["max_depth"].as_u64().unwrap_or(64).clamp(1, 128) as usize;
    let deadline = Instant::now() + Duration::from_secs(5);
    let patterns = params["patterns"].as_bool().unwrap_or(true);
    let mut queue = VecDeque::from([(root, None, 0_usize)]);
    let mut nodes = Vec::new();
    let mut truncated = false;
    while let Some((element, parent, depth)) = queue.pop_front() {
        if nodes.len() >= limit || Instant::now() >= deadline {
            truncated = true;
            break;
        }
        let id = nodes.len();
        let node = unsafe {
            let bounds = element.CurrentBoundingRectangle().ok();
            let password = element.CurrentIsPassword().ok().map(|v| v.as_bool());
            let mut supported = Vec::new();
            let mut patterns_truncated = false;
            if patterns {
                for pattern in 10000..=10034 {
                    if Instant::now() >= deadline {
                        patterns_truncated = true;
                        break;
                    }
                    if element.GetCurrentPattern(UIA_PATTERN_ID(pattern)).is_ok() {
                        supported.push(pattern);
                    }
                }
            }
            let value = if password != Some(false) {
                None
            } else {
                element
                    .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                    .ok()
                    .and_then(|pattern| pattern.CurrentValue().ok())
                    .map(|v| v.to_string())
            };
            json!({"id":id,"parent":parent,"depth":depth,"runtime_id":runtime_id(&element),
                "name":element.CurrentName().ok().map(|v|v.to_string()),
                "control_type":element.CurrentControlType().ok().map(|v|v.0),
                "localized_control_type":element.CurrentLocalizedControlType().ok().map(|v|v.to_string()),
                "automation_id":element.CurrentAutomationId().ok().map(|v|v.to_string()),
                "class_name":element.CurrentClassName().ok().map(|v|v.to_string()),
                "process_id":element.CurrentProcessId().ok(),
                "native_window_handle":element.CurrentNativeWindowHandle().ok().map(|value|format!("{:X}",value.0 as usize)),
                "help_text":element.CurrentHelpText().ok().map(|v|v.to_string()),
                "access_key":element.CurrentAccessKey().ok().map(|v|v.to_string()),
                "accelerator_key":element.CurrentAcceleratorKey().ok().map(|v|v.to_string()),
                "aria_role":element.CurrentAriaRole().ok().map(|v|v.to_string()),
                "aria_properties":element.CurrentAriaProperties().ok().map(|v|v.to_string()),
                "keyboard_focusable":element.CurrentIsKeyboardFocusable().ok().map(|v|v.as_bool()),
                "focused":element.CurrentHasKeyboardFocus().ok().map(|v|v.as_bool()),
                "enabled":element.CurrentIsEnabled().ok().map(|v|v.as_bool()),
                "offscreen":element.CurrentIsOffscreen().ok().map(|v|v.as_bool()),
                "password":password,"value":value,
                "bounds":bounds.map(|r|json!({"x":r.left,"y":r.top,"width":r.right-r.left,"height":r.bottom-r.top})),
                "supported_pattern_ids":supported,"patterns_truncated":patterns_truncated})
        };
        truncated |= node["patterns_truncated"] == true;
        nodes.push(node);
        let mut child = collections::walk(&walker, &element, collections::WalkerStep::FirstChild)
            .map_err(|error| error.to_string())?;
        if depth >= depth_limit {
            truncated |= child.is_some();
            continue;
        }
        while let Some(element) = child {
            if nodes.len() + queue.len() >= limit || Instant::now() >= deadline {
                truncated = true;
                break;
            }
            child = collections::walk(&walker, &element, collections::WalkerStep::NextSibling)
                .map_err(|error| error.to_string())?;
            queue.push_back((element, Some(id), depth + 1));
        }
    }
    Ok(
        json!({"source":"windows-uia","view":params["view"].as_str().unwrap_or("raw"),"nodes":nodes,
        "truncated":truncated,"max_nodes":limit,"max_depth":depth_limit,
        "scope":"selected native window and its descendants; snapshot IDs are not persistent"}),
    )
}

struct OwnedArray(*mut SAFEARRAY);
impl Drop for OwnedArray {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = SafeArrayDestroy(self.0);
            }
        }
    }
}

fn runtime_id(element: &IUIAutomationElement) -> Option<Vec<i32>> {
    let array = OwnedArray(unsafe { element.GetRuntimeId() }.ok()?);
    if array.0.is_null() {
        return None;
    }
    unsafe {
        if SafeArrayGetDim(array.0) != 1 || SafeArrayGetVartype(array.0).ok()? != VT_I4 {
            return None;
        }
        let lower = SafeArrayGetLBound(array.0, 1).ok()?;
        let upper = SafeArrayGetUBound(array.0, 1).ok()?;
        let length = upper.checked_sub(lower)?.checked_add(1)?;
        if !(1..=64).contains(&length) {
            return None;
        }
        let mut data = std::ptr::null_mut();
        SafeArrayAccessData(array.0, &mut data).ok()?;
        let value = if data.is_null() {
            None
        } else {
            Some(std::slice::from_raw_parts(data.cast::<i32>(), length as usize).to_vec())
        };
        SafeArrayUnaccessData(array.0).ok()?;
        value
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    runtime_id: Option<Vec<i32>>,
    name: Option<String>,
    automation_id: Option<String>,
    control_type: Option<i32>,
    class_name: Option<String>,
}
impl Target {
    fn parse(value: &Value) -> Result<Self, String> {
        let target: Self = serde_json::from_value(value.clone()).map_err(|_| {
            "UIA target accepts runtime_id, name, automation_id, control_type and class_name only"
        })?;
        if target.runtime_id.is_none()
            && target.name.is_none()
            && target.automation_id.is_none()
            && target.control_type.is_none()
            && target.class_name.is_none()
        {
            return Err("UIA target must contain at least one exact criterion".into());
        }
        if target
            .runtime_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 64)
            || [&target.name, &target.automation_id, &target.class_name]
                .into_iter()
                .flatten()
                .any(|value| value.len() > 4096 || value.contains('\0'))
        {
            return Err("UIA target criterion is invalid or too long".into());
        }
        Ok(target)
    }
    fn matches(&self, element: &IUIAutomationElement) -> bool {
        unsafe {
            self.control_type.is_none_or(|value| {
                element
                    .CurrentControlType()
                    .is_ok_and(|actual| actual.0 == value)
            }) && self.name.as_ref().is_none_or(|value| {
                element
                    .CurrentName()
                    .is_ok_and(|actual| actual == value.as_str())
            }) && self.automation_id.as_ref().is_none_or(|value| {
                element
                    .CurrentAutomationId()
                    .is_ok_and(|actual| actual == value.as_str())
            }) && self.class_name.as_ref().is_none_or(|value| {
                element
                    .CurrentClassName()
                    .is_ok_and(|actual| actual == value.as_str())
            }) && self
                .runtime_id
                .as_ref()
                .is_none_or(|value| runtime_id(element).as_ref() == Some(value))
        }
    }
}

fn find_target(
    root: &IUIAutomationElement,
    walker: &IUIAutomationTreeWalker,
    target: &Target,
    params: &Value,
) -> Result<IUIAutomationElement, String> {
    let limit = params["max_nodes"].as_u64().unwrap_or(1000).clamp(1, 5000) as usize;
    let depth_limit = params["max_depth"].as_u64().unwrap_or(64).clamp(1, 128) as usize;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut queue = VecDeque::from([(root.clone(), 0_usize)]);
    let mut matched = None;
    let mut visited = 0;
    let mut truncated = false;
    while let Some((element, depth)) = queue.pop_front() {
        if visited >= limit || Instant::now() >= deadline {
            truncated = true;
            break;
        }
        visited += 1;
        if target.matches(&element) {
            // UIA runtime IDs are opaque and unique on the current desktop.
            // An exact live ID needs no scan of unrelated remaining controls.
            if target.runtime_id.is_some() {
                return Ok(element);
            }
            if matched.is_some() {
                return Err(
                    "UIA target is ambiguous; use additional exact criteria or a runtime_id".into(),
                );
            }
            matched = Some(element.clone());
        }
        let mut child = collections::walk(walker, &element, collections::WalkerStep::FirstChild)
            .map_err(|error| error.to_string())?;
        if depth >= depth_limit {
            truncated |= child.is_some();
            continue;
        }
        while let Some(element) = child {
            if visited + queue.len() >= limit || Instant::now() >= deadline {
                truncated = true;
                break;
            }
            child = collections::walk(walker, &element, collections::WalkerStep::NextSibling)
                .map_err(|error| error.to_string())?;
            queue.push_back((element, depth + 1));
        }
    }
    if truncated {
        return Err(
            "UIA target search was truncated; narrow the target or raise its bounded limits".into(),
        );
    }
    matched.ok_or_else(|| "UIA target was not found in the selected native window".into())
}

fn inspect(element: &IUIAutomationElement) -> Value {
    unsafe {
        let password = element
            .CurrentIsPassword()
            .ok()
            .map(|value| value.as_bool());
        let mut states = serde_json::Map::new();
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        {
            states.insert("value".into(), json!({"read_only":pattern.CurrentIsReadOnly().ok().map(|value|value.as_bool()),
                "value":(password==Some(false)).then(||pattern.CurrentValue().ok().map(|value|value.to_string())).flatten()}));
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
        {
            states.insert(
                "toggle".into(),
                json!(pattern.CurrentToggleState().ok().map(|value| value.0)),
            );
        }
        if let Ok(pattern) = element
            .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
        {
            states.insert(
                "expand_collapse".into(),
                json!(pattern
                    .CurrentExpandCollapseState()
                    .ok()
                    .map(|value| value.0)),
            );
        }
        if let Ok(pattern) = element
            .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
        {
            states.insert(
                "selected".into(),
                json!(pattern
                    .CurrentIsSelected()
                    .ok()
                    .map(|value| value.as_bool())),
            );
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId)
        {
            states.insert("range".into(), json!({"value":(password==Some(false)).then(||pattern.CurrentValue().ok()).flatten(),"minimum":pattern.CurrentMinimum().ok(),"maximum":pattern.CurrentMaximum().ok(),
                "small_change":pattern.CurrentSmallChange().ok(),"large_change":pattern.CurrentLargeChange().ok(),"read_only":pattern.CurrentIsReadOnly().ok().map(|value|value.as_bool())}));
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId)
        {
            states.insert("scroll".into(), json!({"horizontal_percent":pattern.CurrentHorizontalScrollPercent().ok(),"vertical_percent":pattern.CurrentVerticalScrollPercent().ok(),
                "horizontal_view_size":pattern.CurrentHorizontalViewSize().ok(),"vertical_view_size":pattern.CurrentVerticalViewSize().ok(),
                "horizontal_scrollable":pattern.CurrentHorizontallyScrollable().ok().map(|value|value.as_bool()),"vertical_scrollable":pattern.CurrentVerticallyScrollable().ok().map(|value|value.as_bool())}));
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationSelectionPattern>(UIA_SelectionPatternId)
        {
            states.insert("selection".into(), json!({"can_select_multiple":pattern.CurrentCanSelectMultiple().ok().map(|value|value.as_bool()),"selection_required":pattern.CurrentIsSelectionRequired().ok().map(|value|value.as_bool())}));
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationTransformPattern>(UIA_TransformPatternId)
        {
            states.insert("transform".into(), json!({"can_move":pattern.CurrentCanMove().ok().map(|value|value.as_bool()),"can_resize":pattern.CurrentCanResize().ok().map(|value|value.as_bool()),"can_rotate":pattern.CurrentCanRotate().ok().map(|value|value.as_bool())}));
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationWindowPattern>(UIA_WindowPatternId)
        {
            states.insert("window".into(), json!({"visual_state":pattern.CurrentWindowVisualState().ok().map(|value|value.0),"modal":pattern.CurrentIsModal().ok().map(|value|value.as_bool()),"topmost":pattern.CurrentIsTopmost().ok().map(|value|value.as_bool()),"can_maximize":pattern.CurrentCanMaximize().ok().map(|value|value.as_bool()),"can_minimize":pattern.CurrentCanMinimize().ok().map(|value|value.as_bool())}));
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationGridPattern>(UIA_GridPatternId)
        {
            states.insert("grid".into(), json!({"rows":pattern.CurrentRowCount().ok(),"columns":pattern.CurrentColumnCount().ok()}));
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationGridItemPattern>(UIA_GridItemPatternId)
        {
            states.insert("grid_item".into(), json!({"row":pattern.CurrentRow().ok(),"column":pattern.CurrentColumn().ok(),"row_span":pattern.CurrentRowSpan().ok(),"column_span":pattern.CurrentColumnSpan().ok()}));
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationTablePattern>(UIA_TablePatternId)
        {
            states.insert("table".into(), json!({"row_or_column_major":pattern.CurrentRowOrColumnMajor().ok().map(|value|value.0)}));
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationDockPattern>(UIA_DockPatternId)
        {
            states.insert(
                "dock".into(),
                json!({"position":pattern.CurrentDockPosition().ok().map(|value|value.0)}),
            );
        }
        if let Ok(pattern) = element
            .GetCurrentPatternAs::<IUIAutomationMultipleViewPattern>(UIA_MultipleViewPatternId)
        {
            states.insert(
                "multiple_view".into(),
                json!({"current":pattern.CurrentCurrentView().ok()}),
            );
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationSelectionPattern2>(UIA_SelectionPattern2Id)
        {
            states.insert(
                "selection2".into(),
                json!({"item_count":pattern.CurrentItemCount().ok()}),
            );
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationTransformPattern2>(UIA_TransformPattern2Id)
        {
            states.insert("transform2".into(), json!({"can_zoom":pattern.CurrentCanZoom().ok().map(|value|value.as_bool()),"zoom":pattern.CurrentZoomLevel().ok(),"zoom_minimum":pattern.CurrentZoomMinimum().ok(),"zoom_maximum":pattern.CurrentZoomMaximum().ok()}));
        }
        if let Ok(pattern) = element.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(
            UIA_LegacyIAccessiblePatternId,
        ) {
            states.insert("legacy".into(), json!({"role":pattern.CurrentRole().ok(),"state":pattern.CurrentState().ok(),"child_id":pattern.CurrentChildId().ok(),
                "description":pattern.CurrentDescription().ok().map(|value|value.to_string()),"default_action":pattern.CurrentDefaultAction().ok().map(|value|value.to_string()),
                "value":(password==Some(false)).then(||pattern.CurrentValue().ok().map(|value|value.to_string())).flatten()}));
        }
        json!({"runtime_id":runtime_id(element),"name":element.CurrentName().ok().map(|value|value.to_string()),
            "automation_id":element.CurrentAutomationId().ok().map(|value|value.to_string()),"control_type":element.CurrentControlType().ok().map(|value|value.0),
            "class_name":element.CurrentClassName().ok().map(|value|value.to_string()),"password":password,
            "enabled":element.CurrentIsEnabled().ok().map(|value|value.as_bool()),"focused":element.CurrentHasKeyboardFocus().ok().map(|value|value.as_bool()),"states":states})
    }
}

pub fn is_mutating(params: &Value) -> bool {
    !matches!(
        params["operation"].as_str().unwrap_or("snapshot"),
        "snapshot"
            | "inspect"
            | "text"
            | "text-selection"
            | "grid-item"
            | "table-headers"
            | "table-item-headers"
            | "selection"
            | "containing-grid"
            | "selection-container"
    )
}

fn validate_request(params: &Value) -> Result<&str, String> {
    let object = params
        .as_object()
        .ok_or("UIA parameters must be an object")?;
    let operation = match object.get("operation") {
        None => "snapshot",
        Some(value) => value.as_str().ok_or("UIA operation must be a string")?,
    };
    for (name, maximum) in [("max_nodes", 5000), ("max_depth", 128)] {
        if params.get(name).is_some_and(|value| {
            !value
                .as_u64()
                .is_some_and(|value| (1..=maximum).contains(&value))
        }) {
            return Err(format!("UIA {name} must be between 1 and {maximum}"));
        }
    }
    if params.get("view").is_some_and(|value| {
        !value
            .as_str()
            .is_some_and(|value| ["raw", "control", "content"].contains(&value))
    }) {
        return Err("UIA view must be raw, control or content".into());
    }
    if params
        .get("patterns")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err("UIA patterns must be a boolean".into());
    }
    let extras: &[&str] = match operation {
        "snapshot" => &["patterns"],
        "inspect"
        | "invoke"
        | "focus"
        | "toggle"
        | "expand"
        | "collapse"
        | "select"
        | "add-to-selection"
        | "remove-from-selection"
        | "scroll-into-view"
        | "realize"
        | "legacy-default-action"
        | "window-close" => &["target"],
        "set-value" | "set-range-value" | "multiple-view-set" => &["target", "value"],
        "text" | "text-selection" => &["target", "max_chars"],
        "grid-item" => &["target", "row", "column"],
        "table-headers" | "table-item-headers" => &["target", "max_items"],
        "selection" => &[
            "target",
            "max_items",
            "strategy",
            "scan_max_nodes",
            "scan_max_depth",
        ],
        "containing-grid" | "selection-container" => &["target"],
        "scroll" => &["target", "horizontal_amount", "vertical_amount"],
        "set-scroll-percent" => &["target", "horizontal_percent", "vertical_percent"],
        "window-state" => &["target", "state"],
        "move" => &["target", "x", "y"],
        "resize" => &["target", "width", "height"],
        "rotate" => &["target", "degrees"],
        "text-select" => &["target", "start", "end"],
        _ => return Err("unsupported UIA operation".into()),
    };
    if object.keys().any(|key| {
        ![
            "window",
            "native_window",
            "view",
            "operation",
            "max_nodes",
            "max_depth",
        ]
        .contains(&key.as_str())
            && !extras.contains(&key.as_str())
    }) {
        return Err("unsupported UIA parameter for this operation".into());
    }
    if operation == "grid-item" {
        collections::coordinate(params, "row")?;
        collections::coordinate(params, "column")?;
    }
    if matches!(
        operation,
        "table-headers" | "table-item-headers" | "selection"
    ) {
        collections::limit(params)?;
    }
    if operation == "selection"
        && params.get("strategy").is_some_and(|value| {
            !value
                .as_str()
                .is_some_and(|value| ["provider", "items"].contains(&value))
        })
    {
        return Err("UIA selection strategy must be provider or items".into());
    }
    for (name, maximum) in [("scan_max_nodes", 5000), ("scan_max_depth", 128)] {
        if let Some(value) = params.get(name) {
            if operation != "selection" || params["strategy"].as_str() != Some("items") {
                return Err("UIA selection scan limits require the items strategy".into());
            }
            if !value
                .as_u64()
                .is_some_and(|value| (1..=maximum).contains(&value))
            {
                return Err(format!("UIA {name} must be between 1 and {maximum}"));
            }
        }
    }
    Ok(operation)
}

fn number(params: &Value, name: &str) -> Result<f64, String> {
    params[name]
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("UIA {name} must be a finite number"))
}

fn scroll_amount(params: &Value, name: &str) -> Result<ScrollAmount, String> {
    match params.get(name).map(Value::as_str).unwrap_or(Some("none")) {
        Some("none") => Ok(ScrollAmount_NoAmount),
        Some("small-decrement") => Ok(ScrollAmount_SmallDecrement),
        Some("large-decrement") => Ok(ScrollAmount_LargeDecrement),
        Some("small-increment") => Ok(ScrollAmount_SmallIncrement),
        Some("large-increment") => Ok(ScrollAmount_LargeIncrement),
        _ => Err(format!("UIA {name} must be none, small-decrement, large-decrement, small-increment or large-increment")),
    }
}

enum Action {
    Focus,
    Invoke,
    Value(windows::core::BSTR),
    Range(f64),
    Toggle,
    Expand,
    Collapse,
    Select,
    AddSelection,
    RemoveSelection,
    Scroll(ScrollAmount, ScrollAmount),
    ScrollPercent(f64, f64),
    ScrollIntoView,
    Realize,
    LegacyDefault,
    MultipleView(i32),
    WindowState(WindowVisualState),
    WindowClose,
    Move(f64, f64),
    Resize(f64, f64),
    Rotate(f64),
    TextSelect(i32, i32),
}
impl Action {
    fn parse(operation: &str, params: &Value) -> Result<Self, String> {
        let percent = |name: &str| -> Result<f64, String> {
            let value = if params.get(name).is_some() {
                number(params, name)?
            } else {
                -1.0
            };
            if value == -1.0 || (0.0..=100.0).contains(&value) {
                Ok(value)
            } else {
                Err(format!("UIA {name} must be -1 or between 0 and 100"))
            }
        };
        Ok(match operation {
            "focus" => Self::Focus,
            "invoke" => Self::Invoke,
            "set-value" => Self::Value(windows::core::BSTR::from(
                params["value"]
                    .as_str()
                    .filter(|value| value.len() <= 65536 && !value.contains('\0'))
                    .ok_or("UIA value must be a string of at most 65536 bytes without NUL")?,
            )),
            "set-range-value" => Self::Range(number(params, "value")?),
            "toggle" => Self::Toggle,
            "expand" => Self::Expand,
            "collapse" => Self::Collapse,
            "select" => Self::Select,
            "add-to-selection" => Self::AddSelection,
            "remove-from-selection" => Self::RemoveSelection,
            "scroll" => Self::Scroll(
                scroll_amount(params, "horizontal_amount")?,
                scroll_amount(params, "vertical_amount")?,
            ),
            "set-scroll-percent" => {
                Self::ScrollPercent(percent("horizontal_percent")?, percent("vertical_percent")?)
            }
            "scroll-into-view" => Self::ScrollIntoView,
            "realize" => Self::Realize,
            "legacy-default-action" => Self::LegacyDefault,
            "multiple-view-set" => Self::MultipleView(
                params["value"]
                    .as_i64()
                    .and_then(|value| i32::try_from(value).ok())
                    .ok_or("UIA view value must be a signed 32-bit integer")?,
            ),
            "window-state" => Self::WindowState(match params["state"].as_str() {
                Some("normal") => WindowVisualState_Normal,
                Some("minimized") => WindowVisualState_Minimized,
                Some("maximized") => WindowVisualState_Maximized,
                _ => return Err("UIA window state must be normal, minimized or maximized".into()),
            }),
            "window-close" => Self::WindowClose,
            "move" => Self::Move(number(params, "x")?, number(params, "y")?),
            "resize" => {
                let (width, height) = (number(params, "width")?, number(params, "height")?);
                if width <= 0.0 || height <= 0.0 {
                    return Err("UIA dimensions must be positive".into());
                }
                Self::Resize(width, height)
            }
            "rotate" => Self::Rotate(number(params, "degrees")?),
            "text-select" => {
                let offset = |name: &str| {
                    params[name]
                        .as_u64()
                        .filter(|value| *value <= 1_000_000)
                        .map(|value| value as i32)
                        .ok_or_else(|| format!("UIA {name} must be between 0 and 1000000"))
                };
                let (start, end) = (offset("start")?, offset("end")?);
                if end < start {
                    return Err("UIA text end must not precede start".into());
                }
                Self::TextSelect(start, end)
            }
            _ => return Err("unsupported UIA action".into()),
        })
    }
    fn perform(&self, element: &IUIAutomationElement) -> windows::core::Result<()> {
        let invalid = || {
            windows::core::Error::from_hresult(windows::core::HRESULT(
                UIA_E_INVALIDOPERATION as i32,
            ))
        };
        unsafe {
            match self {
                Self::Focus => {
                    if !element.CurrentIsKeyboardFocusable()?.as_bool()
                        || element.CurrentIsOffscreen()?.as_bool()
                    {
                        return Err(invalid());
                    }
                    element.SetFocus()
                }
                Self::Invoke => element
                    .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)?
                    .Invoke(),
                Self::Value(value) => {
                    let pattern = element
                        .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)?;
                    if pattern.CurrentIsReadOnly()?.as_bool() {
                        return Err(invalid());
                    }
                    pattern.SetValue(value)
                }
                Self::Range(value) => {
                    let pattern = element.GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(
                        UIA_RangeValuePatternId,
                    )?;
                    let (minimum, maximum) = (pattern.CurrentMinimum()?, pattern.CurrentMaximum()?);
                    if pattern.CurrentIsReadOnly()?.as_bool()
                        || !minimum.is_finite()
                        || !maximum.is_finite()
                        || *value < minimum
                        || *value > maximum
                    {
                        return Err(invalid());
                    }
                    pattern.SetValue(*value)
                }
                Self::Toggle => element
                    .GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)?
                    .Toggle(),
                Self::Expand => element
                    .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                        UIA_ExpandCollapsePatternId,
                    )?
                    .Expand(),
                Self::Collapse => element
                    .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                        UIA_ExpandCollapsePatternId,
                    )?
                    .Collapse(),
                Self::Select => element
                    .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                        UIA_SelectionItemPatternId,
                    )?
                    .Select(),
                Self::AddSelection => element
                    .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                        UIA_SelectionItemPatternId,
                    )?
                    .AddToSelection(),
                Self::RemoveSelection => element
                    .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                        UIA_SelectionItemPatternId,
                    )?
                    .RemoveFromSelection(),
                Self::Scroll(horizontal, vertical) => element
                    .GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId)?
                    .Scroll(*horizontal, *vertical),
                Self::ScrollPercent(horizontal, vertical) => element
                    .GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId)?
                    .SetScrollPercent(*horizontal, *vertical),
                Self::ScrollIntoView => element
                    .GetCurrentPatternAs::<IUIAutomationScrollItemPattern>(UIA_ScrollItemPatternId)?
                    .ScrollIntoView(),
                Self::Realize => element
                    .GetCurrentPatternAs::<IUIAutomationVirtualizedItemPattern>(
                        UIA_VirtualizedItemPatternId,
                    )?
                    .Realize(),
                Self::LegacyDefault => element
                    .GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(
                        UIA_LegacyIAccessiblePatternId,
                    )?
                    .DoDefaultAction(),
                Self::MultipleView(value) => element
                    .GetCurrentPatternAs::<IUIAutomationMultipleViewPattern>(
                        UIA_MultipleViewPatternId,
                    )?
                    .SetCurrentView(*value),
                Self::WindowState(value) => element
                    .GetCurrentPatternAs::<IUIAutomationWindowPattern>(UIA_WindowPatternId)?
                    .SetWindowVisualState(*value),
                Self::WindowClose => element
                    .GetCurrentPatternAs::<IUIAutomationWindowPattern>(UIA_WindowPatternId)?
                    .Close(),
                Self::Move(x, y) => {
                    let pattern = element.GetCurrentPatternAs::<IUIAutomationTransformPattern>(
                        UIA_TransformPatternId,
                    )?;
                    if !pattern.CurrentCanMove()?.as_bool() {
                        return Err(invalid());
                    }
                    pattern.Move(*x, *y)
                }
                Self::Resize(width, height) => {
                    let pattern = element.GetCurrentPatternAs::<IUIAutomationTransformPattern>(
                        UIA_TransformPatternId,
                    )?;
                    if !pattern.CurrentCanResize()?.as_bool() {
                        return Err(invalid());
                    }
                    pattern.Resize(*width, *height)
                }
                Self::Rotate(degrees) => {
                    let pattern = element.GetCurrentPatternAs::<IUIAutomationTransformPattern>(
                        UIA_TransformPatternId,
                    )?;
                    if !pattern.CurrentCanRotate()?.as_bool() {
                        return Err(invalid());
                    }
                    pattern.Rotate(*degrees)
                }
                Self::TextSelect(start, end) => {
                    if element
                        .CurrentIsPassword()
                        .ok()
                        .map(|value| value.as_bool())
                        != Some(false)
                    {
                        return Err(invalid());
                    }
                    let pattern = element
                        .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)?;
                    let range = pattern.DocumentRange()?;
                    range.MoveEndpointByRange(
                        TextPatternRangeEndpoint_End,
                        &range,
                        TextPatternRangeEndpoint_Start,
                    )?;
                    if range.MoveEndpointByUnit(
                        TextPatternRangeEndpoint_End,
                        TextUnit_Character,
                        *end,
                    )? != *end
                        || range.MoveEndpointByUnit(
                            TextPatternRangeEndpoint_Start,
                            TextUnit_Character,
                            *start,
                        )? != *start
                    {
                        return Err(invalid());
                    }
                    range.Select()
                }
            }
        }
    }
}

pub fn handle(
    hwnd: isize,
    pid: u32,
    params: &Value,
    revalidate: impl Fn() -> Result<(), String>,
) -> Result<Value, String> {
    let operation = validate_request(params)?;
    revalidate()?;
    if operation == "snapshot" {
        let result = snapshot(hwnd, pid, params)?;
        revalidate()?;
        return Ok(result);
    }
    let target = Target::parse(&params["target"])?;
    let action = is_mutating(params)
        .then(|| Action::parse(operation, params))
        .transpose()?;
    let (_com, automation, walker, root) = connect(hwnd, pid, params)?;
    let element = find_target(&root, &walker, &target, params)?;
    // Do not let a stale HWND/closed framework ID resolve to a replacement
    // window during the native provider traversal.
    revalidate()?;
    validate_root(&root, hwnd, pid)?;
    if operation == "inspect" {
        return Ok(json!({"source":"windows-uia","element":inspect(&element)}));
    }
    if collections::OPERATIONS.contains(&operation) {
        let result = collections::read(hwnd, &automation, &root, &element, operation, params)?;
        revalidate()?;
        validate_root(&root, hwnd, pid)?;
        return Ok(result);
    }
    if matches!(operation, "text" | "text-selection") {
        if unsafe { element.CurrentIsPassword() }
            .ok()
            .map(|value| value.as_bool())
            != Some(false)
        {
            return Err("UIA text is redacted for password or unknown-password controls".into());
        }
        let max_chars = params.get("max_chars").map_or(Ok(4096), |value| {
            value
                .as_u64()
                .filter(|value| (1..=65536).contains(value))
                .ok_or("UIA max_chars must be between 1 and 65536")
        })? as i32;
        let pattern =
            unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }
                .map_err(|error| format!("UIA Text pattern unavailable: {error}"))?;
        if operation == "text" {
            let range = unsafe { pattern.DocumentRange() }.map_err(|error| error.to_string())?;
            let text = unsafe { range.GetText(max_chars) }
                .map_err(|error| error.to_string())?
                .to_string();
            return Ok(json!({"source":"windows-uia","text":text,"max_chars":max_chars}));
        }
        let selected = unsafe { pattern.GetSelection() }.map_err(|error| error.to_string())?;
        let count = unsafe { selected.Length() }.map_err(|error| error.to_string())?;
        let mut texts = Vec::new();
        for index in 0..count.min(32) {
            let range = unsafe { selected.GetElement(index) }.map_err(|error| error.to_string())?;
            texts.push(
                unsafe { range.GetText(max_chars) }
                    .map_err(|error| error.to_string())?
                    .to_string(),
            );
        }
        return Ok(
            json!({"source":"windows-uia","selection":texts,"truncated":count>32,"max_chars":max_chars}),
        );
    }
    if !matches!(
        action.as_ref(),
        Some(Action::Realize | Action::ScrollIntoView)
    ) && !unsafe { element.CurrentIsEnabled() }
        .map_err(|error| error.to_string())?
        .as_bool()
    {
        return Err("UIA target is disabled".into());
    }
    // Every operation calls the real typed UIA pattern. Unsupported patterns
    // fail; there is deliberately no DOM/coordinate fallback.
    action.ok_or("unsupported UIA action")?.perform(&element).map_err(|error| {
        format!("UIA {operation} failed (HRESULT 0x{:08X}); the target may be read-only or its pattern unsupported", error.code().0 as u32)
    })?;
    // Never echo the supplied value, especially when setting a password.
    Ok(json!({"source":"windows-uia","operation":operation,"dispatched":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn targets_do_not_accept_snapshot_indices_arbitrary_handles_or_unbounded_ids() {
        for target in [
            json!({}),
            json!({"id":1}),
            json!({"hwnd":123}),
            json!({"runtime_id":[]}),
            json!({"runtime_id":vec![42;65]}),
            json!({"runtime_id":[4294967296_u64]}),
            json!({"name":"x\0y"}),
        ] {
            assert!(Target::parse(&target).is_err());
        }
        assert!(Target::parse(&json!({"runtime_id":[42,-2147483647,3]})).is_ok());
        assert!(Target::parse(&json!({"name":"中文","control_type":50000})).is_ok());
    }
    #[test]
    fn invalid_operations_and_arguments_fail_before_native_calls() {
        for params in [
            json!({"operation":"click"}),
            json!({"target":{"name":"x"}}),
            json!({"max_nodes":5001}),
            json!({"max_depth":0}),
            json!({"patterns":"false"}),
            json!({"operation":"invoke","value":"must not be ignored"}),
            json!({"operation":"grid-item","row":-1,"column":0}),
            json!({"operation":"grid-item","row":0,"column":1000001}),
            json!({"operation":"grid-item","row":0}),
            json!({"operation":"selection","max_items":0}),
            json!({"operation":"selection","strategy":"dom"}),
            json!({"operation":"selection","strategy":"items","scan_max_nodes":0}),
            json!({"operation":"selection","scan_max_depth":1}),
            json!({"operation":"table-headers","max_items":257}),
            json!({"operation":"selection-container","max_items":1}),
            json!({"view":"desktop"}),
        ] {
            assert!(validate_request(&params).is_err());
        }
        for (operation, params) in [
            ("set-value", json!({"value":3})),
            ("set-value", json!({"value":"x\0y"})),
            ("set-range-value", json!({"value":"30"})),
            ("scroll", json!({"vertical_amount":"invalid"})),
            ("set-scroll-percent", json!({"vertical_percent":101})),
            ("set-scroll-percent", json!({"horizontal_percent":"100"})),
            ("window-state", json!({"state":"fullscreen"})),
            ("multiple-view-set", json!({"value":4294967296_u64})),
            ("resize", json!({"width":0,"height":100})),
            ("text-select", json!({"start":4,"end":2})),
        ] {
            assert!(Action::parse(operation, &params).is_err(), "{operation}");
        }
        assert!(Action::parse("set-value", &json!({"value":""})).is_ok());
        assert!(Action::parse("set-scroll-percent", &json!({"vertical_percent":100})).is_ok());
        assert!(!is_mutating(&json!({"operation":"text"})));
        assert!(!is_mutating(&json!({"operation":"selection"})));
        assert!(is_mutating(&json!({"operation":"text-select"})));
    }
}
