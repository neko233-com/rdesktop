//! Read-only Windows UI Automation, rooted at one renderer-owned HWND. Never
//! starts at the desktop root or enumerates other applications' windows.
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::*;

struct Com;
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

pub fn snapshot(hwnd: isize, params: &Value) -> Result<Value, String> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|e| e.to_string())?;
    let _com = Com;
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
            let value = if password == Some(true) {
                None
            } else {
                element
                    .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                    .ok()
                    .and_then(|pattern| pattern.CurrentValue().ok())
                    .map(|v| v.to_string())
            };
            json!({"id":id,"parent":parent,"depth":depth,
                "name":element.CurrentName().ok().map(|v|v.to_string()),
                "control_type":element.CurrentControlType().ok().map(|v|v.0),
                "localized_control_type":element.CurrentLocalizedControlType().ok().map(|v|v.to_string()),
                "automation_id":element.CurrentAutomationId().ok().map(|v|v.to_string()),
                "class_name":element.CurrentClassName().ok().map(|v|v.to_string()),
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
        if depth >= depth_limit {
            truncated = true;
            continue;
        }
        let mut child = unsafe { walker.GetFirstChildElement(&element) }.ok();
        while let Some(element) = child {
            if nodes.len() + queue.len() >= limit || Instant::now() >= deadline {
                truncated = true;
                break;
            }
            child = unsafe { walker.GetNextSiblingElement(&element) }.ok();
            queue.push_back((element, Some(id), depth + 1));
        }
    }
    Ok(
        json!({"source":"windows-uia","view":params["view"].as_str().unwrap_or("raw"),"nodes":nodes,
        "truncated":truncated,"max_nodes":limit,"max_depth":depth_limit,
        "scope":"selected native window and its descendants; snapshot IDs are not persistent"}),
    )
}
