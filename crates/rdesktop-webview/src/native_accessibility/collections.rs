//! Native pattern collections and related controls. Providers can return
//! elements outside a container, so each result must prove window ancestry.
use super::{runtime_id, Duration, Instant};
use serde_json::{json, Value};
use std::collections::VecDeque;
use windows::core::Interface;
use windows::Win32::UI::Accessibility::*;

pub(super) const OPERATIONS: &[&str] = &[
    "grid-item",
    "table-headers",
    "table-item-headers",
    "selection",
    "containing-grid",
    "selection-container",
    "relations",
];

pub(super) fn relation(params: &Value) -> Result<&str, String> {
    params["relation"]
        .as_str()
        .filter(|value| {
            [
                "described-by",
                "labeled-by",
                "controller-for",
                "flows-to",
                "flows-from",
            ]
            .contains(value)
        })
        .ok_or_else(|| {
            "UIA relation must be described-by, labeled-by, controller-for, flows-to or flows-from"
                .into()
        })
}

pub(super) fn coordinate(params: &Value, name: &str) -> Result<i32, String> {
    params[name]
        .as_u64()
        .filter(|value| *value <= 1_000_000)
        .map(|value| value as i32)
        .ok_or_else(|| format!("UIA {name} must be an integer between 0 and 1000000"))
}

pub(super) fn limit(params: &Value) -> Result<i32, String> {
    match params.get("max_items") {
        None => Ok(32),
        Some(value) => value
            .as_u64()
            .filter(|value| (1..=256).contains(value))
            .map(|value| value as i32)
            .ok_or_else(|| "UIA max_items must be an integer between 1 and 256".into()),
    }
}

struct Scope<'a> {
    hwnd: isize,
    automation: &'a IUIAutomation,
    root: &'a IUIAutomationElement,
    walker: IUIAutomationTreeWalker,
    deadline: Instant,
}

impl Scope<'_> {
    fn relations(
        &self,
        element: &IUIAutomationElement,
        params: &Value,
    ) -> windows::core::Result<Value> {
        let relation = relation(params).unwrap();
        let maximum = limit(params).unwrap();
        // These properties legitimately return S_OK + null. Keep that distinct
        // from a failed HRESULT, and own any returned COM pointer immediately.
        let mut array: Option<IUIAutomationElementArray> = None;
        let related = unsafe {
            let table = element.vtable();
            match relation {
                "labeled-by" => {
                    let mut label: Option<IUIAutomationElement> = None;
                    (table.CurrentLabeledBy)(element.as_raw(), &mut label as *mut _ as _).ok()?;
                    let elements = label
                        .as_ref()
                        .map(|item| self.descriptor(item))
                        .transpose()?
                        .into_iter()
                        .collect::<Vec<_>>();
                    json!({"total":elements.len(),"truncated":false,"elements":elements,"max_items":maximum})
                }
                _ => {
                    if relation == "flows-from" {
                        let element2 = element.cast::<IUIAutomationElement2>()?;
                        (element2.vtable().CurrentFlowsFrom)(
                            element2.as_raw(),
                            &mut array as *mut _ as _,
                        )
                        .ok()?;
                    } else {
                        let method = match relation {
                            "described-by" => table.CurrentDescribedBy,
                            "controller-for" => table.CurrentControllerFor,
                            "flows-to" => table.CurrentFlowsTo,
                            _ => unreachable!("validated UIA relation"),
                        };
                        method(element.as_raw(), &mut array as *mut _ as _).ok()?;
                    }
                    match array {
                        Some(array) => self.array(array, maximum)?,
                        None => {
                            json!({"total":0,"truncated":false,"elements":[],"max_items":maximum})
                        }
                    }
                }
            }
        };
        Ok(json!({"relation":relation,"related":related}))
    }

    fn selected_items(
        &self,
        container: &IUIAutomationElement,
        params: &Value,
    ) -> windows::core::Result<Value> {
        let maximum = limit(params).unwrap();
        let max_nodes = params["scan_max_nodes"].as_u64().unwrap_or(1000) as usize;
        let max_depth = params["scan_max_depth"].as_u64().unwrap_or(64) as usize;
        let mut queue = VecDeque::from([(container.clone(), 0_usize)]);
        let (mut visited, mut matched) = (0, 0);
        let mut elements = Vec::new();
        let mut truncated = false;
        // This explicit strategy uses the native selected flag AND its native
        // selection-container link, independently of GetCurrentSelection.
        while let Some((element, depth)) = queue.pop_front() {
            if visited >= max_nodes || Instant::now() >= self.deadline {
                truncated = true;
                break;
            }
            visited += 1;
            self.ensure_inside(&element)?;
            unsafe {
                if depth > 0 {
                    let candidate = element
                        .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                            UIA_SelectionItemPatternId,
                        );
                    let candidate = match candidate {
                        Ok(pattern) => Some(pattern),
                        Err(error)
                            if matches!(error.code().0 as u32, 0 | 0x80004002)
                                || error.code().0 == UIA_E_NOTSUPPORTED as i32 =>
                        {
                            None
                        }
                        Err(error) => return Err(error),
                    };
                    if let Some(pattern) = candidate {
                        if pattern.CurrentIsSelected()?.as_bool()
                            && self
                                .automation
                                .CompareElements(&pattern.CurrentSelectionContainer()?, container)?
                                .as_bool()
                        {
                            matched += 1;
                            if elements.len() < maximum as usize {
                                elements.push(self.descriptor(&element)?);
                            }
                        }
                    }
                }
                let mut child = walk(&self.walker, &element, WalkerStep::FirstChild)?;
                if depth >= max_depth {
                    truncated |= child.is_some();
                    continue;
                }
                while let Some(element) = child {
                    if visited + queue.len() >= max_nodes || Instant::now() >= self.deadline {
                        truncated = true;
                        break;
                    }
                    child = walk(&self.walker, &element, WalkerStep::NextSibling)?;
                    queue.push_back((element, depth + 1));
                }
            }
        }
        Ok(
            json!({"total":(!truncated).then_some(matched),"matched":matched,
            "truncated":truncated || elements.len()<matched,"search_truncated":truncated,
            "max_items":maximum,"visited":visited,"elements":elements}),
        )
    }
    fn descriptor(&self, element: &IUIAutomationElement) -> windows::core::Result<Value> {
        self.ensure_inside(element)?;
        unsafe {
            // No text/value is read here, including from password providers.
            Ok(json!({"runtime_id":runtime_id(element),
                "name":element.CurrentName().ok().map(|value|value.to_string()),
                "automation_id":element.CurrentAutomationId().ok().map(|value|value.to_string()),
                "control_type":element.CurrentControlType().ok().map(|value|value.0),
                "class_name":element.CurrentClassName().ok().map(|value|value.to_string()),
                "password":element.CurrentIsPassword().ok().map(|value|value.as_bool()),
                "enabled":element.CurrentIsEnabled().ok().map(|value|value.as_bool()),
                "offscreen":element.CurrentIsOffscreen().ok().map(|value|value.as_bool())}))
        }
    }

    fn ensure_inside(&self, element: &IUIAutomationElement) -> windows::core::Result<()> {
        let mut current = element.clone();
        for _ in 0..=128 {
            if Instant::now() >= self.deadline {
                break;
            }
            unsafe {
                if self
                    .automation
                    .CompareElements(&current, self.root)?
                    .as_bool()
                {
                    return Ok(());
                }
                let native = current.CurrentNativeWindowHandle()?.0;
                if !native.is_null()
                    && native as isize != self.hwnd
                    && windows_sys::Win32::UI::WindowsAndMessaging::IsChild(
                        self.hwnd as _,
                        native as _,
                    ) == 0
                {
                    break;
                }
                let Some(parent) = walk(&self.walker, &current, WalkerStep::Parent)? else {
                    break;
                };
                current = parent;
            }
        }
        Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            UIA_E_ELEMENTNOTAVAILABLE as i32,
        )))
    }

    fn array(
        &self,
        array: IUIAutomationElementArray,
        maximum: i32,
    ) -> windows::core::Result<Value> {
        let total = unsafe { array.Length()? };
        if total < 0 {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                UIA_E_INVALIDOPERATION as i32,
            )));
        }
        let mut elements = Vec::new();
        for index in 0..total.min(maximum) {
            if Instant::now() >= self.deadline {
                break;
            }
            elements.push(self.descriptor(&unsafe { array.GetElement(index)? })?);
        }
        Ok(
            json!({"truncated":elements.len() < total as usize,"elements":elements,"total":total,"max_items":maximum}),
        )
    }

    unsafe fn perform(
        &self,
        element: &IUIAutomationElement,
        operation: &str,
        params: &Value,
    ) -> windows::core::Result<Value> {
        unsafe {
            match operation {
                "relations" => self.relations(element, params),
                "grid-item" => {
                    let pattern = element
                        .GetCurrentPatternAs::<IUIAutomationGridPattern>(UIA_GridPatternId)?;
                    // Validation runs before the provider is connected; these conversions cannot fail here.
                    let row = coordinate(params, "row").unwrap();
                    let column = coordinate(params, "column").unwrap();
                    if row >= pattern.CurrentRowCount()?
                        || column >= pattern.CurrentColumnCount()?
                    {
                        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                            UIA_E_INVALIDOPERATION as i32,
                        )));
                    }
                    Ok(
                        json!({"element":self.descriptor(&pattern.GetItem(row,column)?)?,"row":row,"column":column}),
                    )
                }
                "containing-grid" => {
                    let pattern = element.GetCurrentPatternAs::<IUIAutomationGridItemPattern>(
                        UIA_GridItemPatternId,
                    )?;
                    Ok(json!({"element":self.descriptor(&pattern.CurrentContainingGrid()?)?}))
                }
                "selection-container" => {
                    let pattern = element
                        .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                            UIA_SelectionItemPatternId,
                        )?;
                    Ok(json!({"element":self.descriptor(&pattern.CurrentSelectionContainer()?)?}))
                }
                "selection" => {
                    let pattern = element.GetCurrentPatternAs::<IUIAutomationSelectionPattern>(
                        UIA_SelectionPatternId,
                    )?;
                    let strategy = params["strategy"].as_str().unwrap_or("provider");
                    let selected = if strategy == "items" {
                        self.selected_items(element, params)?
                    } else {
                        self.array(pattern.GetCurrentSelection()?, limit(params).unwrap())?
                    };
                    Ok(json!({"selection":selected,"strategy":strategy}))
                }
                "table-headers" => {
                    let pattern = element
                        .GetCurrentPatternAs::<IUIAutomationTablePattern>(UIA_TablePatternId)?;
                    let maximum = limit(params).unwrap();
                    Ok(
                        json!({"rows":self.array(pattern.GetCurrentRowHeaders()?,maximum)?,"columns":self.array(pattern.GetCurrentColumnHeaders()?,maximum)?}),
                    )
                }
                "table-item-headers" => {
                    let pattern = element.GetCurrentPatternAs::<IUIAutomationTableItemPattern>(
                        UIA_TableItemPatternId,
                    )?;
                    let maximum = limit(params).unwrap();
                    Ok(
                        json!({"rows":self.array(pattern.GetCurrentRowHeaderItems()?,maximum)?,"columns":self.array(pattern.GetCurrentColumnHeaderItems()?,maximum)?}),
                    )
                }
                _ => unreachable!("validated collection operation"),
            }
        }
    }
}

pub(super) enum WalkerStep {
    FirstChild,
    NextSibling,
    Parent,
}

// UIA walkers legitimately return S_OK with a null element at a tree edge.
// Preserve that nullable ABI explicitly: the generated non-null binding loses
// the distinction between this result and a real provider error.
pub(super) fn walk(
    walker: &IUIAutomationTreeWalker,
    element: &IUIAutomationElement,
    step: WalkerStep,
) -> windows::core::Result<Option<IUIAutomationElement>> {
    unsafe {
        let mut result = std::ptr::null_mut();
        let table = IUIAutomationTreeWalker::vtable(walker);
        let method = match step {
            WalkerStep::FirstChild => table.GetFirstChildElement,
            WalkerStep::NextSibling => table.GetNextSiblingElement,
            WalkerStep::Parent => table.GetParentElement,
        };
        method(walker.as_raw(), element.as_raw(), &mut result).ok()?;
        Ok((!result.is_null()).then(|| IUIAutomationElement::from_raw(result)))
    }
}

pub(super) fn read(
    hwnd: isize,
    automation: &IUIAutomation,
    root: &IUIAutomationElement,
    element: &IUIAutomationElement,
    operation: &str,
    params: &Value,
) -> Result<Value, String> {
    let scope = Scope {
        hwnd,
        automation,
        root,
        walker: unsafe { automation.RawViewWalker() }.map_err(|error| error.to_string())?,
        deadline: Instant::now() + Duration::from_secs(5),
    };
    let mut result = unsafe { scope.perform(element,operation,params) }.map_err(|error|format!("UIA {operation} failed (HRESULT 0x{:08X}); pattern unavailable, invalid coordinates or related element outside the selected window",error.code().0 as u32))?;
    result["source"] = json!("windows-uia");
    result["operation"] = json!(operation);
    Ok(result)
}
