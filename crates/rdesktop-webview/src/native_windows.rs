//! Debug-only renderer-owned native windows. A window property identifies its
//! lifetime, so reusing a closed HWND cannot revive an old Agent target.
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
    System::Diagnostics::ToolHelp::*,
    UI::{
        HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::IsWindowEnabled, WindowsAndMessaging::*,
    },
};

const PROPERTY: *const u16 = windows::core::w!("rdesktop.agent.native-window").0;
const ROOT_PROPERTY: *const u16 = windows::core::w!("rdesktop.agent.root-window").0;
static NEXT_COOKIE: AtomicUsize = AtomicUsize::new(1);

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) struct FrameBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}
pub(crate) fn frame_bounds(anchor: Anchor) -> Option<FrameBounds> {
    if !anchor.valid() {
        return None;
    }
    let mut bounds = RECT::default();
    let result = unsafe {
        DwmGetWindowAttribute(
            anchor.hwnd as _,
            DWMWA_EXTENDED_FRAME_BOUNDS as _,
            (&mut bounds as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
    };
    let width = bounds.right.checked_sub(bounds.left)?;
    let height = bounds.bottom.checked_sub(bounds.top)?;
    (result >= 0 && width > 0 && height > 0 && anchor.valid()).then_some(FrameBounds {
        x: bounds.left,
        y: bounds.top,
        width,
        height,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Anchor {
    pub hwnd: isize,
    pub pid: u32,
    cookie: usize,
    browser_pid: Option<u32>,
}
impl Anchor {
    fn create(hwnd: isize, root: bool) -> Result<Self, String> {
        let cookie = NEXT_COOKIE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| "native window identities exhausted")?;
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd as _, &mut pid);
        }
        let anchor = Self {
            hwnd,
            pid,
            cookie,
            browser_pid: None,
        };
        unsafe {
            if pid == 0
                || (root && pid != std::process::id())
                || !GetPropW(hwnd as _, PROPERTY).is_null()
            {
                return Err("native window is not an unregistered eligible window".into());
            }
            if SetPropW(hwnd as _, PROPERTY, cookie as _) == 0 {
                return Err("could not register native window lifetime".into());
            }
            if root && SetPropW(hwnd as _, ROOT_PROPERTY, cookie as _) == 0 {
                anchor.clear();
                return Err("could not register renderer window lifetime".into());
            }
        }
        Ok(anchor)
    }
    pub(crate) fn valid(self) -> bool {
        unsafe {
            let mut pid = 0;
            IsWindow(self.hwnd as _) != 0
                && GetWindowThreadProcessId(self.hwnd as _, &mut pid) != 0
                && pid == self.pid
                && GetPropW(self.hwnd as _, PROPERTY) as usize == self.cookie
        }
    }
    fn clear(self) {
        if !self.valid() {
            return;
        }
        unsafe {
            if GetPropW(self.hwnd as _, PROPERTY) as usize == self.cookie {
                RemovePropW(self.hwnd as _, PROPERTY);
            }
            if GetPropW(self.hwnd as _, ROOT_PROPERTY) as usize == self.cookie {
                RemovePropW(self.hwnd as _, ROOT_PROPERTY);
            }
        }
    }
}

#[derive(Clone)]
struct Root {
    anchor: Anchor,
    url: Option<String>,
}
#[derive(Clone)]
struct Owned {
    root: u64,
    anchor: Anchor,
}
#[derive(Default)]
struct State {
    roots: HashMap<u64, Root>,
    owned: HashMap<String, Owned>,
}
#[derive(Default)]
pub(crate) struct NativeWindows {
    state: Mutex<State>,
}

struct ChildProcesses {
    pids: HashSet<u32>,
    visited: usize,
    deadline: Instant,
    truncated: bool,
}
unsafe extern "system" fn child_process(window: HWND, context: LPARAM) -> windows_sys::core::BOOL {
    let scan = unsafe { &mut *(context as *mut ChildProcesses) };
    if scan.visited >= 2048 || Instant::now() >= scan.deadline {
        scan.truncated = true;
        return 0;
    }
    scan.visited += 1;
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut pid);
    }
    if pid != 0 {
        scan.pids.insert(pid);
    }
    1
}
fn associated_processes(root: Anchor) -> ChildProcesses {
    let mut scan = ChildProcesses {
        pids: HashSet::from([root.pid]),
        visited: 0,
        deadline: Instant::now() + Duration::from_millis(100),
        truncated: false,
    };
    // Read once from the registered WebView2 COM object, never from user input
    // or process-name matching. WebView2 can broker a file picker without a
    // cross-process child HWND. The exact owner chain is still mandatory.
    if let Some(pid) = root.browser_pid {
        scan.pids.insert(pid);
    }
    if root.valid() {
        unsafe {
            EnumChildWindows(
                root.hwnd as _,
                Some(child_process),
                &mut scan as *mut _ as isize,
            );
        }
    }
    scan
}
fn owner_chain_matches(root: Anchor, window: isize) -> bool {
    if !root.valid() || window == root.hwnd {
        return false;
    }
    unsafe {
        if !GetPropW(window as _, ROOT_PROPERTY).is_null() {
            return false;
        }
        let mut owner = GetWindow(window as _, GW_OWNER);
        for _ in 0..64 {
            if owner.is_null() {
                break;
            }
            if owner as isize == root.hwnd {
                return true;
            }
            // An owned secondary renderer belongs to itself, even when its
            // ultimate owner is the selected renderer.
            if !GetPropW(owner, ROOT_PROPERTY).is_null() {
                return false;
            }
            if IsChild(root.hwnd as _, owner) != 0 {
                return true;
            }
            owner = GetWindow(owner, GW_OWNER);
        }
    }
    false
}
fn is_owned_in(root: Anchor, window: isize, processes: &HashSet<u32>) -> bool {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window as _, &mut pid);
    }
    processes.contains(&pid) && owner_chain_matches(root, window)
}
fn is_owned(root: Anchor, window: isize) -> bool {
    if !owner_chain_matches(root, window) {
        return false;
    }
    if is_owned_in(root, window, &associated_processes(root).pids) {
        return true;
    }
    // Broker PIDs are never a global allowlist: a fresh immediate UIA child
    // relation AND the native owner chain must identify this exact window.
    crate::native_accessibility::native_children(
        root.hwnd,
        root.pid,
        Instant::now() + Duration::from_secs(2),
    )
    .is_ok_and(|children| children.windows.iter().any(|&(hwnd, _)| hwnd == window))
}

struct Scan {
    root: Anchor,
    windows: Vec<isize>,
    maximum: usize,
    deadline: Instant,
    truncated: bool,
    processes: HashSet<u32>,
}
unsafe extern "system" fn collect(window: HWND, context: LPARAM) -> windows_sys::core::BOOL {
    let scan = unsafe { &mut *(context as *mut Scan) };
    if Instant::now() >= scan.deadline {
        scan.truncated = true;
        return 0;
    }
    if is_owned_in(scan.root, window as isize, &scan.processes) {
        if scan.windows.len() >= scan.maximum {
            scan.truncated = true;
            return 0;
        }
        scan.windows.push(window as isize);
    }
    1
}
struct Snapshot(HANDLE);
impl Drop for Snapshot {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

impl NativeWindows {
    pub(crate) fn attach(
        &self,
        id: u64,
        hwnd: isize,
        url: Option<String>,
        browser_pid: Option<u32>,
    ) -> Result<(), String> {
        let mut anchor = Anchor::create(hwnd, true)?;
        anchor.browser_pid = browser_pid.filter(|pid| *pid != 0);
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => {
                anchor.clear();
                return Err("native window registry poisoned".into());
            }
        };
        if let Some(old) = state.roots.insert(
            id,
            Root {
                anchor,
                url: url.filter(|value| value.len() <= 65536),
            },
        ) {
            old.anchor.clear();
        }
        Ok(())
    }
    pub(crate) fn detach(&self, id: u64) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(root) = state.roots.remove(&id) {
                root.anchor.clear();
            }
            state.owned.retain(|_, window| {
                if window.root == id {
                    window.anchor.clear();
                    false
                } else {
                    true
                }
            });
        }
    }
    pub(crate) fn navigated(&self, id: u64, url: Option<&str>) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(root) = state.roots.get_mut(&id) {
                root.url = url.filter(|value| value.len() <= 65536).map(str::to_owned);
            }
        }
    }
    fn root(&self, id: u64) -> Result<Root, String> {
        let root = self
            .state
            .lock()
            .map_err(|_| "native window registry poisoned")?
            .roots
            .get(&id)
            .cloned()
            .ok_or("renderer window is not registered")?;
        if !root.anchor.valid() {
            return Err("renderer window lifetime ended".into());
        }
        Ok(root)
    }
    pub(crate) fn target(&self, id: u64, native: Option<&str>) -> Result<Anchor, String> {
        let root = self.root(id)?;
        if let Some(native) = native {
            let owned = self
                .state
                .lock()
                .map_err(|_| "native window registry poisoned")?
                .owned
                .get(native)
                .cloned()
                .ok_or("unknown or expired native window target")?;
            if owned.root != id
                || !owned.anchor.valid()
                || !is_owned(root.anchor, owned.anchor.hwnd)
            {
                return Err(
                    "native window is closed, replaced or belongs to another renderer".into(),
                );
            }
            Ok(owned.anchor)
        } else {
            Ok(root.anchor)
        }
    }
    pub(crate) fn windows(&self) -> Result<Value, String> {
        let (roots, total) = {
            let state = self
                .state
                .lock()
                .map_err(|_| "native window registry poisoned")?;
            let mut ids: Vec<_> = state.roots.keys().copied().collect();
            ids.sort_unstable();
            let total = ids.len();
            let roots: Vec<_> = ids
                .into_iter()
                .take(256)
                .map(|id| (id, state.roots[&id].clone()))
                .collect();
            (roots, total)
        };
        let mut values = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut truncated = total > 256;
        for (id, root) in roots {
            if Instant::now() >= deadline {
                truncated = true;
                break;
            }
            if root.anchor.valid() {
                let mut value = metadata(root.anchor);
                value["id"] = json!(id.to_string());
                value["kind"] = json!("webview");
                value["url"] = json!(root.url);
                values.push(value);
            }
        }
        values.sort_by_key(|value| {
            value["id"]
                .as_str()
                .unwrap_or_default()
                .parse::<u64>()
                .unwrap_or_default()
        });
        Ok(
            json!({"windows":values,"truncated":truncated,"max_windows":256,"source":"windows-window-metadata"}),
        )
    }
    pub(crate) fn enumerate(&self, id: u64, params: &Value) -> Result<Value, String> {
        let object = params
            .as_object()
            .ok_or("native-windows parameters must be an object")?;
        if object
            .keys()
            .any(|name| !["window", "max_windows"].contains(&name.as_str()))
        {
            return Err("unknown native-windows parameter".into());
        }
        let maximum = match params.get("max_windows") {
            None => 32,
            Some(value) => value
                .as_u64()
                .filter(|value| (1..=64).contains(value))
                .ok_or("max_windows must be between 1 and 64")? as usize,
        };
        let root = self.root(id)?;
        let associated = associated_processes(root.anchor);
        let mut scan = Scan {
            root: root.anchor,
            windows: Vec::new(),
            maximum,
            deadline: Instant::now() + Duration::from_secs(2),
            truncated: associated.truncated,
            processes: associated.pids,
        };
        unsafe {
            let handle = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if handle == INVALID_HANDLE_VALUE {
                return Err("could not inspect renderer-associated GUI threads".into());
            }
            let snapshot = Snapshot(handle);
            let mut entry = THREADENTRY32 {
                dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            let mut available = Thread32First(snapshot.0, &mut entry) != 0;
            if !available {
                return Err("renderer-associated thread snapshot is unavailable".into());
            }
            let mut visited = 0;
            let mut threads = 0;
            while available {
                if visited >= 65536 || threads >= 256 || Instant::now() >= scan.deadline {
                    scan.truncated = true;
                    break;
                }
                visited += 1;
                if scan.processes.contains(&entry.th32OwnerProcessID) {
                    threads += 1;
                    EnumThreadWindows(
                        entry.th32ThreadID,
                        Some(collect),
                        &mut scan as *mut _ as isize,
                    );
                    if scan.truncated {
                        break;
                    }
                }
                available = Thread32Next(snapshot.0, &mut entry) != 0;
                if !available && GetLastError() != ERROR_NO_MORE_FILES {
                    scan.truncated = true;
                }
            }
        }
        let children = crate::native_accessibility::native_children(
            root.anchor.hwnd,
            root.anchor.pid,
            scan.deadline,
        )?;
        scan.truncated |= children.truncated;
        for (hwnd, _) in children.windows {
            if !scan.windows.contains(&hwnd) && owner_chain_matches(root.anchor, hwnd) {
                if scan.windows.len() >= maximum {
                    scan.truncated = true;
                    break;
                }
                scan.windows.push(hwnd);
            }
        }
        if !root.anchor.valid() {
            return Err("renderer closed during native window discovery".into());
        }
        // Never call a UIA provider while holding the registry mutex. A native
        // provider can message the UI thread, whose engine events update URLs.
        let (roots, recorded) = {
            let state = self
                .state
                .lock()
                .map_err(|_| "native window registry poisoned")?;
            let roots: HashMap<_, _> = state
                .roots
                .iter()
                .map(|(&id, root)| (id, root.anchor))
                .collect();
            (roots, state.owned.clone())
        };
        let mut expired = Vec::new();
        for (key, entry) in recorded {
            if Instant::now() >= scan.deadline {
                scan.truncated = true;
                break;
            }
            let valid = entry.anchor.valid()
                && roots
                    .get(&entry.root)
                    .is_some_and(|root| is_owned(*root, entry.anchor.hwnd));
            if !valid {
                expired.push((key, entry.anchor));
            }
        }
        let mut candidates = Vec::new();
        for hwnd in scan.windows {
            if Instant::now() >= scan.deadline {
                scan.truncated = true;
                break;
            }
            if is_owned(root.anchor, hwnd) {
                candidates.push(hwnd);
            }
        }
        let targets = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "native window registry poisoned")?;
            if state
                .roots
                .get(&id)
                .is_none_or(|current| current.anchor != root.anchor)
            {
                return Err("renderer replaced during native window discovery".into());
            }
            for (key, anchor) in expired {
                if state
                    .owned
                    .get(&key)
                    .is_some_and(|entry| entry.anchor == anchor)
                {
                    state.owned.remove(&key);
                    anchor.clear();
                }
            }
            let mut targets = Vec::new();
            for hwnd in candidates {
                if Instant::now() >= scan.deadline {
                    scan.truncated = true;
                    break;
                }
                let existing = state
                    .owned
                    .iter()
                    .find(|(_, entry)| entry.root == id && entry.anchor.hwnd == hwnd)
                    .map(|(key, entry)| (key.clone(), entry.anchor));
                let (key, anchor) = if let Some(value) = existing {
                    value
                } else {
                    if state.owned.len() >= 256 {
                        scan.truncated = true;
                        break;
                    }
                    let anchor = Anchor::create(hwnd, false)?;
                    let key = format!("native-{}", anchor.cookie);
                    state.owned.insert(key.clone(), Owned { root: id, anchor });
                    (key, anchor)
                };
                targets.push((key, anchor));
            }
            targets
        };
        let mut windows = Vec::new();
        for (key, anchor) in targets {
            if Instant::now() >= scan.deadline {
                scan.truncated = true;
                break;
            }
            if !anchor.valid() || !is_owned(root.anchor, anchor.hwnd) {
                continue;
            }
            let mut value = metadata(anchor);
            value["id"] = json!(key);
            value["kind"] = json!("owned-native-window");
            value["owner_window"] = json!(id.to_string());
            windows.push(value);
        }
        Ok(
            json!({"source":"windows-owned-windows","window":id.to_string(),"windows":windows,"max_windows":maximum,"truncated":scan.truncated,"scope":"native owner chain under the selected renderer; broker windows additionally require a fresh immediate raw UIA child relation"}),
        )
    }
}
impl Drop for NativeWindows {
    fn drop(&mut self) {
        if let Ok(state) = self.state.lock() {
            for root in state.roots.values() {
                root.anchor.clear();
            }
            for owned in state.owned.values() {
                owned.anchor.clear();
            }
        }
    }
}

fn metadata(anchor: Anchor) -> Value {
    unsafe {
        let hwnd = anchor.hwnd as HWND;
        let (mut outer, mut client) = (RECT::default(), RECT::default());
        let has_outer = GetWindowRect(hwnd, &mut outer) != 0;
        let has_client = GetClientRect(hwnd, &mut client) != 0;
        let mut title = vec![0_u16; 2048];
        let mut length = 0_usize;
        let title_ok = SendMessageTimeoutW(
            hwnd,
            WM_GETTEXT,
            title.len(),
            title.as_mut_ptr() as _,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            100,
            &mut length,
        ) != 0;
        let mut class = vec![0_u16; 256];
        let class_length =
            GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        json!({"process_id":anchor.pid,"title":title_ok.then(||String::from_utf16_lossy(&title[..length.min(title.len()-1)])),"title_unavailable":!title_ok,"title_truncated":title_ok&&length>=title.len()-1,
            "class_name":String::from_utf16_lossy(&class[..class_length.min(class.len())]),"visible":IsWindowVisible(hwnd)!=0,"enabled":IsWindowEnabled(hwnd)!=0,
            "focused":GetForegroundWindow()==hwnd,"minimized":IsIconic(hwnd)!=0,"maximized":IsZoomed(hwnd)!=0,"scale_factor":GetDpiForWindow(hwnd).max(96) as f64/96.0,
            "frame":frame_bounds(anchor),
            "client":has_client.then(||json!({"width":client.right-client.left,"height":client.bottom-client.top})),
            "outer":has_outer.then(||json!({"x":outer.left,"y":outer.top,"width":outer.right-outer.left,"height":outer.bottom-outer.top}))})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestWindow(isize);
    impl TestWindow {
        fn new(title: &str, owner: isize) -> Self {
            let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    windows::core::w!("STATIC").0,
                    title.as_ptr(),
                    WS_OVERLAPPED,
                    0,
                    0,
                    320,
                    200,
                    owner as _,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!hwnd.is_null());
            Self(hwnd as isize)
        }
        fn destroy(&mut self) {
            if self.0 != 0 {
                if unsafe { IsWindow(self.0 as _) } != 0 {
                    assert_ne!(unsafe { DestroyWindow(self.0 as _) }, 0);
                }
                self.0 = 0;
            }
        }
    }
    impl Drop for TestWindow {
        fn drop(&mut self) {
            self.destroy();
        }
    }
    fn named(registry: &NativeWindows, root: u64, title: &str) -> Option<String> {
        registry.enumerate(root, &json!({})).unwrap()["windows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|window| window["title"] == title)
            .map(|window| window["id"].as_str().unwrap().to_owned())
    }
    #[test]
    fn owned_window_ids_follow_real_owner_and_lifetime_properties() {
        let first = TestWindow::new("scope root one", 0);
        let second = TestWindow::new("scope root two", first.0);
        let mut child = TestWindow::new("owned scope fixture", first.0);
        let _nested = TestWindow::new("nested scope fixture", child.0);
        let _unowned = TestWindow::new("unowned scope fixture", 0);
        let registry = NativeWindows::default();
        registry.attach(1, first.0, None, None).unwrap();
        registry.attach(2, second.0, None, None).unwrap();
        let original = named(&registry, 1, "owned scope fixture").unwrap();
        assert_eq!(
            named(&registry, 1, "owned scope fixture").unwrap(),
            original
        );
        assert!(named(&registry, 1, "nested scope fixture").is_some());
        assert!(named(&registry, 1, "scope root two").is_none());
        assert!(named(&registry, 1, "unowned scope fixture").is_none());
        assert!(named(&registry, 2, "owned scope fixture").is_none());
        assert!(registry.target(1, Some(&original)).is_ok());
        assert!(registry.target(2, Some(&original)).is_err());
        assert_ne!(
            unsafe { SetWindowLongPtrW(child.0 as _, GWLP_HWNDPARENT, second.0) },
            0
        );
        assert!(registry.target(1, Some(&original)).is_err());
        let moved = named(&registry, 2, "owned scope fixture").unwrap();
        assert_ne!(moved, original);
        let anchor = registry.target(2, Some(&moved)).unwrap();
        unsafe {
            assert_ne!(
                SetPropW(child.0 as _, PROPERTY, (anchor.cookie + 1024) as _),
                0
            );
        }
        assert!(registry.target(2, Some(&moved)).is_err());
        unsafe {
            assert_ne!(SetPropW(child.0 as _, PROPERTY, anchor.cookie as _), 0);
        }
        assert!(registry.target(2, Some(&moved)).is_ok());
        child.destroy();
        assert!(registry.target(2, Some(&moved)).is_err());
        registry.detach(1);
        assert!(registry.target(1, None).is_err());
        assert!(unsafe { GetPropW(first.0 as _, PROPERTY) }.is_null());
    }
}
