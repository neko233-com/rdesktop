//! Windows development adapter. The entire module is absent without both the
//! `native-agent` feature and debug assertions; release code cannot enable it.
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use rdesktop_dev::native::{NativeAgentBackend, NativeAgentServer};
use serde_json::{json, Value};
use webview2_com::{
    CallDevToolsProtocolMethodCompletedHandler, DevToolsProtocolEventReceivedEventHandler,
};
use windows::core::{HSTRING, PWSTR};
use wry::WebViewExtWindows;

use crate::NativeRuntime;

pub(crate) struct NativeAgent {
    backend: Arc<Backend>,
    server: NativeAgentServer,
    pub headless: bool,
}

struct Backend {
    runtime: NativeRuntime,
    ids: Mutex<HashSet<u64>>,
    events: Mutex<VecDeque<Value>>,
    event_notify: Condvar,
    event_sequence: AtomicU64,
    input_busy: Mutex<HashSet<u64>>,
}

struct InputPermit<'a> {
    busy: &'a Mutex<HashSet<u64>>,
    window: u64,
}
impl Drop for InputPermit<'_> {
    fn drop(&mut self) {
        if let Ok(mut busy) = self.busy.lock() {
            busy.remove(&self.window);
        }
    }
}

impl NativeAgent {
    pub fn start(runtime: NativeRuntime) -> rdesktop_core::Result<Self> {
        let discovery = std::env::var_os("RDESKTOP_AGENT_SESSION")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(format!(".rdesktop/agent-{}.json", std::process::id()))
            });
        let backend = Arc::new(Backend {
            runtime,
            ids: Mutex::new(HashSet::new()),
            events: Mutex::new(VecDeque::new()),
            event_notify: Condvar::new(),
            event_sequence: AtomicU64::new(0),
            input_busy: Mutex::new(HashSet::new()),
        });
        let server = NativeAgentServer::start(backend.clone(), discovery)
            .map_err(|e| rdesktop_core::RdesktopError::Config(e.to_string()))?;
        Ok(Self {
            backend,
            server,
            headless: std::env::var("RDESKTOP_HEADLESS").is_ok_and(|v| v == "1"),
        })
    }

    pub fn attach(&self, id: u64, webview: &wry::WebView) {
        self.backend.ids.lock().unwrap().insert(id);
        // CDP events come from the actual engine, including uncaught errors and
        // failed network loads. Retain at most 512 events and 16 KiB per event.
        for method in [
            "Runtime.consoleAPICalled",
            "Runtime.exceptionThrown",
            "Log.entryAdded",
            "Network.loadingFailed",
            "Accessibility.loadComplete",
            "Accessibility.nodesUpdated",
            "Input.dragIntercepted",
        ] {
            let backend = self.backend.clone();
            let result = (|| -> windows::core::Result<()> {
                let receiver = unsafe {
                    webview
                        .webview()
                        .GetDevToolsProtocolEventReceiver(&HSTRING::from(method))?
                };
                let handler =
                    DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                        if let Some(args) = args {
                            let mut pointer = PWSTR::null();
                            unsafe {
                                args.ParameterObjectAsJson(&mut pointer)?;
                            }
                            let payload = webview2_com::take_pwstr(pointer);
                            backend.event(id, method, &payload);
                        }
                        Ok(())
                    }));
                let mut token = 0;
                unsafe { receiver.add_DevToolsProtocolEventReceived(&handler, &mut token) }
            })();
            if let Err(error) = result {
                self.backend.event(
                    id,
                    "rdesktop.agent.error",
                    &json!({"error":error.to_string()}).to_string(),
                );
            }
        }
        for method in [
            "Runtime.enable",
            "Log.enable",
            "Network.enable",
            "Accessibility.enable",
        ] {
            let callback =
                CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
            let _ = unsafe {
                webview.webview().CallDevToolsProtocolMethod(
                    &HSTRING::from(method),
                    &HSTRING::from("{}"),
                    &callback,
                )
            };
        }
    }

    pub fn detach(&self, id: u64) {
        self.backend.ids.lock().unwrap().remove(&id);
    }
    pub fn finish(&mut self) {
        self.server.finish();
    }
}

impl Backend {
    fn event(&self, id: u64, method: &str, payload: &str) {
        let params = if payload.len() <= 16 * 1024 {
            serde_json::from_str(payload).unwrap_or(Value::Null)
        } else {
            json!({"truncated":true,"bytes":payload.len()})
        };
        let mut events = self.events.lock().unwrap();
        if events.len() >= 512 {
            events.pop_front();
        }
        let sequence = self.event_sequence.fetch_add(1, Ordering::Relaxed) + 1;
        events.push_back(json!({"sequence":sequence.to_string(),"window":id.to_string(),"method":method,"params":params}));
        drop(events);
        self.event_notify.notify_all();
    }

    fn window(&self, params: &Value) -> Result<u64, String> {
        let ids = self.ids.lock().unwrap();
        if let Some(id) = params.get("window") {
            let id = id
                .as_str()
                .and_then(|value| value.parse::<u64>().ok())
                .or_else(|| id.as_u64())
                .ok_or("window must be an unsigned integer string")?;
            if ids.contains(&id) {
                return Ok(id);
            }
            return Err("native window no longer exists".into());
        }
        if ids.len() != 1 {
            return Err("select a window from /windows; there is no unique target".into());
        }
        Ok(*ids.iter().next().unwrap())
    }

    fn cdp(&self, id: u64, method: &str, params: Value) -> Result<Value, String> {
        if method.is_empty()
            || method.len() > 160
            || !method
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.')
            || !params.is_object()
        {
            return Err("invalid CDP method or parameters".into());
        }
        let method = method.to_string();
        let params = params.to_string();
        let (send, receive) = mpsc::sync_channel(1);
        let deadline = Instant::now() + Duration::from_secs(15);
        self.runtime
            .with_window(id, move |_, webview| {
                if Instant::now() >= deadline {
                    return Err("CDP request expired before dispatch".to_string());
                }
                let callback = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                    move |status, output| {
                        let result = status
                            .map_err(|e| e.to_string())
                            .and_then(|_| {
                                if output.len() > 32 * 1024 * 1024 {
                                    Err("CDP response exceeds 32 MiB".into())
                                } else {
                                    serde_json::from_str::<Value>(&output)
                                        .map_err(|e| e.to_string())
                                }
                            })
                            .and_then(|value| {
                                if value.get("error").is_some() {
                                    Err(value.to_string())
                                } else {
                                    Ok(value)
                                }
                            });
                        let _ = send.send(result);
                        Ok(())
                    },
                ));
                unsafe {
                    webview.webview().CallDevToolsProtocolMethod(
                        &HSTRING::from(method),
                        &HSTRING::from(params),
                        &callback,
                    )
                }
                .map_err(|e| e.to_string())
            })
            .map_err(|e| e.to_string())??;
        receive
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|e| format!("CDP completion unavailable: {e}"))?
    }

    fn evaluate(&self, id: u64, expression: &str) -> Result<Value, String> {
        let result = self.cdp(
            id,
            "Runtime.evaluate",
            json!({"expression":expression,
            "returnByValue":true,"awaitPromise":true,"userGesture":true,"allowUnsafeEvalBlockedByCSP":true}),
        )?;
        if result.get("exceptionDetails").is_some() {
            return Err(result["exceptionDetails"].to_string());
        }
        Ok(result["result"]["value"].clone())
    }

    fn point(&self, id: u64, selector: &str) -> Result<(f64, f64), String> {
        let point = self.evaluate(id, &format!(r#"(() => {{
            const nodes = document.querySelectorAll({});
            if (nodes.length !== 1) throw Error('selector must match exactly one element');
            const el = nodes[0]; el.scrollIntoView({{block:'nearest',inline:'nearest'}});
            const r = el.getBoundingClientRect();
            if (!r.width || !r.height || getComputedStyle(el).visibility === 'hidden' || el.disabled || el.getAttribute('aria-disabled')==='true' || el.closest('[inert]')) throw Error('element is not actionable');
            const x=r.x+r.width/2,y=r.y+r.height/2,hit=document.elementFromPoint(x,y);
            if (!hit || !(hit===el || el.contains(hit))) throw Error('element is occluded');
            return {{x,y}};
        }})()"#, json!(selector)))?;
        Ok((
            point["x"].as_f64().ok_or("missing x")?,
            point["y"].as_f64().ok_or("missing y")?,
        ))
    }

    fn accessibility(&self, id: u64, params: &Value) -> Result<Value, String> {
        let method = params["method"].as_str().unwrap_or("getFullAXTree");
        if !matches!(
            method,
            "enable"
                | "disable"
                | "getFullAXTree"
                | "getPartialAXTree"
                | "getRootAXNode"
                | "getAXNodeAndAncestors"
                | "getChildAXNodes"
                | "queryAXTree"
        ) {
            return Err("unknown Accessibility method".into());
        }
        let result = self.cdp(
            id,
            &format!("Accessibility.{method}"),
            params.get("params").cloned().unwrap_or(json!({})),
        )?;
        Ok(json!({"source":"webview2-cdp","window":id.to_string(),"method":method,"result":result}))
    }

    fn target(&self, id: u64, params: &Value) -> Result<String, String> {
        if let Some(selector) = params["selector"].as_str() {
            let result=self.cdp(id,"Runtime.evaluate",json!({"expression":format!("(()=>{{const matches=document.querySelectorAll({});if(matches.length!==1)throw Error('selector must match exactly one element');return matches[0];}})()",json!(selector)),"returnByValue":false}))?;
            if result.get("exceptionDetails").is_some() {
                return Err(result["exceptionDetails"].to_string());
            }
            return result["result"]["objectId"]
                .as_str()
                .map(str::to_string)
                .ok_or("target object unavailable".into());
        }
        let role = string(params, "role")?;
        let name = string(params, "name")?;
        let root = self.cdp(id, "DOM.getDocument", json!({"depth":0,"pierce":true}))?;
        let tree = self.cdp(
            id,
            "Accessibility.queryAXTree",
            json!({"nodeId":root["root"]["nodeId"],"role":role,"accessibleName":name}),
        )?;
        let matches: Vec<_> = tree["nodes"]
            .as_array()
            .ok_or("AX query returned no nodes")?
            .iter()
            .filter(|node| {
                node["ignored"] != true
                    && node["role"]["value"] == role
                    && node["name"]["value"] == name
            })
            .collect();
        if matches.len() != 1 {
            return Err("role and name must match exactly one accessible node".into());
        }
        let resolved = self.cdp(
            id,
            "DOM.resolveNode",
            json!({"backendNodeId":matches[0]["backendDOMNodeId"]}),
        )?;
        resolved["object"]["objectId"]
            .as_str()
            .map(str::to_string)
            .ok_or("accessible node has no DOM target".into())
    }

    fn target_point(&self, id: u64, params: &Value) -> Result<(f64, f64), String> {
        if let Some(selector) = params["selector"].as_str() {
            return self.point(id, selector);
        }
        let target = self.target(id, params)?;
        let result=self.cdp(id,"Runtime.callFunctionOn",json!({"objectId":target,"functionDeclaration":r#"function(){
            this.scrollIntoView({block:'nearest',inline:'nearest'});const r=this.getBoundingClientRect();
            if(!r.width||!r.height||this.disabled||this.getAttribute('aria-disabled')==='true'||this.closest('[inert]')||getComputedStyle(this).visibility==='hidden')throw Error('element is not actionable');
            const x=r.x+r.width/2,y=r.y+r.height/2;let hit=this.ownerDocument.elementFromPoint(x,y);
            while(hit?.shadowRoot){const inner=hit.shadowRoot.elementFromPoint(x,y);if(!inner||inner===hit)break;hit=inner;}
            if(!hit||!(hit===this||this.contains(hit)))throw Error('element is occluded');return {x,y};
        }"#,"returnByValue":true}));
        self.cdp(id, "Runtime.releaseObject", json!({"objectId":target}))
            .ok();
        let result = result?;
        if result.get("exceptionDetails").is_some() {
            return Err(result["exceptionDetails"].to_string());
        }
        let value = &result["result"]["value"];
        Ok((
            value["x"].as_f64().ok_or("missing target x")?,
            value["y"].as_f64().ok_or("missing target y")?,
        ))
    }

    fn drag(&self, id: u64, params: &Value) -> Result<(), String> {
        let source = string(params, "selector")?;
        let destination = string(params, "target_selector")?;
        self.point(id, destination)?;
        self.point(id, source)?;
        // Resolve both points before pressing. Do not drag through an overlay
        // or scroll the source out of view while a mouse button is held.
        let points=self.evaluate(id,&format!(r#"[{},{}].map(selector=>{{const el=document.querySelector(selector),r=el.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2,hit=document.elementFromPoint(x,y);if(!hit||!(hit===el||el.contains(hit)))throw Error('drag endpoints must be visible together');return {{x,y}};}})"#,json!(source),json!(destination)))?;
        let x = points[0]["x"].as_f64().ok_or("invalid drag source")?;
        let y = points[0]["y"].as_f64().ok_or("invalid drag source")?;
        let tx = points[1]["x"].as_f64().ok_or("invalid drag target")?;
        let ty = points[1]["y"].as_f64().ok_or("invalid drag target")?;
        let start_sequence = self.event_sequence.load(Ordering::Relaxed);
        let window_id = id.to_string();
        self.cdp(id, "Input.setInterceptDrags", json!({"enabled":true}))?;
        let movement = (|| -> Result<(), String> {
            self.cdp(
                id,
                "Input.dispatchMouseEvent",
                json!({"type":"mouseMoved","x":x,"y":y}),
            )?;
            self.cdp(id,"Input.dispatchMouseEvent",json!({"type":"mousePressed","button":"left","buttons":1,"clickCount":1,"x":x,"y":y}))?;
            for step in 1..=16 {
                self.cdp(id,"Input.dispatchMouseEvent",json!({"type":"mouseMoved","button":"left","buttons":1,"x":x+(tx-x)*step as f64/16.0,"y":y+(ty-y)*step as f64/16.0}))?;
            }
            let data = {
                let deadline = Instant::now() + Duration::from_secs(3);
                let mut events = self.events.lock().unwrap();
                loop {
                    if let Some(event) = events.iter().rev().find(|event| {
                        event["method"] == "Input.dragIntercepted"
                            && event["window"].as_str() == Some(window_id.as_str())
                            && event["sequence"]
                                .as_str()
                                .and_then(|v| v.parse::<u64>().ok())
                                .is_some_and(|v| v > start_sequence)
                    }) {
                        break event["params"]["data"].clone();
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err("native engine did not start a draggable item".into());
                    }
                    events = self
                        .event_notify
                        .wait_timeout(events, remaining)
                        .map_err(|_| "event queue poisoned")?
                        .0;
                }
            };
            if !data.is_object() {
                return Err("native drag data unavailable or exceeds event limit".into());
            }
            for kind in ["dragEnter", "dragOver", "drop"] {
                self.cdp(
                    id,
                    "Input.dispatchDragEvent",
                    json!({"type":kind,"x":tx,"y":ty,"data":data}),
                )?;
            }
            Ok(())
        })();
        if movement.is_err() {
            self.cdp(id, "Input.cancelDragging", json!({})).ok();
        }
        let released=self.cdp(id,"Input.dispatchMouseEvent",json!({"type":"mouseReleased","button":"left","buttons":0,"clickCount":1,"x":tx,"y":ty}));
        self.cdp(id, "Input.setInterceptDrags", json!({"enabled":false}))
            .ok();
        movement?;
        released?;
        Ok(())
    }

    fn action(&self, id: u64, params: &Value) -> Result<Value, String> {
        let action = string(params, "action")?;
        if matches!(action, "type" | "fill" | "clear") {
            let target = self.target(id, params)?;
            let result=self.cdp(id,"Runtime.callFunctionOn",json!({"objectId":target,"functionDeclaration":r#"function(){
                const editable=this.isContentEditable||this instanceof HTMLTextAreaElement||(this instanceof HTMLInputElement&&['text','search','url','tel','email','password','number'].includes(this.type));
                if(!editable||this.readOnly||this.disabled||this.getAttribute('aria-readonly')==='true'||this.getAttribute('aria-disabled')==='true')throw Error('target is not editable');return true;
            }"#,"returnByValue":true}));
            self.cdp(id, "Runtime.releaseObject", json!({"objectId":target}))
                .ok();
            let result = result?;
            if result.get("exceptionDetails").is_some() {
                return Err(result["exceptionDetails"].to_string());
            }
        }
        match action {
            "drag" => self.drag(id, params)?,
            "focus" => {
                let target = self.target(id, params)?;
                let result = self.cdp(id, "DOM.focus", json!({"objectId":target}));
                self.cdp(id, "Runtime.releaseObject", json!({"objectId":target}))
                    .ok();
                result?;
            }
            "click" | "double_click" | "right_click" | "hover" | "type" | "fill" | "clear" => {
                let (x, y) = self.target_point(id, params)?;
                self.cdp(
                    id,
                    "Input.dispatchMouseEvent",
                    json!({"type":"mouseMoved","x":x,"y":y}),
                )?;
                if action == "hover" {
                    return Ok(json!({"success":true,"input":"cdp"}));
                }
                let button = if action == "right_click" {
                    "right"
                } else {
                    "left"
                };
                let count = if action == "double_click" { 2 } else { 1 };
                for click_count in 1..=count {
                    self.cdp(id,"Input.dispatchMouseEvent",json!({"type":"mousePressed","button":button,"buttons":if button=="right" {2} else {1},"clickCount":click_count,"x":x,"y":y}))?;
                    self.cdp(id,"Input.dispatchMouseEvent",json!({"type":"mouseReleased","button":button,"clickCount":click_count,"x":x,"y":y}))?;
                }
                if matches!(action, "fill" | "clear") {
                    self.key(id, "a", 65, 2)?;
                    self.key(id, "Backspace", 8, 0)?;
                }
                if matches!(action, "fill" | "type") {
                    self.cdp(
                        id,
                        "Input.insertText",
                        json!({"text":string(params,"value")?}),
                    )?;
                }
            }
            "press" => {
                let key = string(params, "value")?;
                let code = match key { "Enter"=>13,"Tab"=>9,"Escape"=>27,"Backspace"=>8,"ArrowLeft"=>37,"ArrowUp"=>38,"ArrowRight"=>39,"ArrowDown"=>40,"Delete"=>46,"Home"=>36,"End"=>35," "=>32,_=>return Err("unsupported press key; use /cdp Input.dispatchKeyEvent for full key parameters".into()) };
                let modifiers = params
                    .get("modifiers")
                    .map(|value| {
                        value
                            .as_u64()
                            .filter(|v| *v <= 15)
                            .ok_or("modifiers must be a bitmask from 0 to 15")
                    })
                    .transpose()?
                    .unwrap_or(0) as u32;
                self.key(id, key, code, modifiers)?;
            }
            "scroll" => {
                let (x, y) = self.target_point(id, params)?;
                self.cdp(id,"Input.dispatchMouseEvent",json!({"type":"mouseWheel","x":x,"y":y,
                    "deltaX":params["delta_x"].as_f64().unwrap_or(0.0),"deltaY":params["delta_y"].as_f64().unwrap_or(400.0)}))?;
            }
            "select" => {
                let target = self.target(id, params)?;
                let result=self.cdp(id,"Runtime.callFunctionOn",json!({"objectId":target,"arguments":[{"value":string(params,"value")?}],"functionDeclaration":r#"function(value){
                    if(this.tagName!=='SELECT'||this.disabled||this.getAttribute('aria-disabled')==='true')throw Error('enabled select element required');
                    if(!Array.from(this.options).some(option=>option.value===value&&!option.disabled&&!option.parentElement.disabled))throw Error('enabled option does not exist');
                    this.value=value;this.dispatchEvent(new Event('input',{bubbles:true}));this.dispatchEvent(new Event('change',{bubbles:true}));return this.value;
                }"#,"returnByValue":true}));
                self.cdp(id, "Runtime.releaseObject", json!({"objectId":target}))
                    .ok();
                let result = result?;
                if result.get("exceptionDetails").is_some() {
                    return Err(result["exceptionDetails"].to_string());
                }
                return Ok(json!({"success":true,"input":"dom-select"}));
            }
            _ => return Err("unknown action".into()),
        }
        Ok(json!({"success":true,"input":"cdp"}))
    }

    fn key(&self, id: u64, key: &str, code: u32, modifiers: u32) -> Result<(), String> {
        let text = if modifiers == 0 {
            match key {
                "Enter" => "\r",
                " " => " ",
                _ => "",
            }
        } else {
            ""
        };
        self.cdp(id,"Input.dispatchKeyEvent",json!({"type":if text.is_empty() {"rawKeyDown"} else {"keyDown"},"key":key,"text":text,"unmodifiedText":text,"windowsVirtualKeyCode":code,"modifiers":modifiers}))?;
        self.cdp(
            id,
            "Input.dispatchKeyEvent",
            json!({"type":"keyUp","key":key,"windowsVirtualKeyCode":code,"modifiers":modifiers}),
        )?;
        Ok(())
    }
}

impl NativeAgentBackend for Backend {
    fn capabilities(&self) -> Value {
        json!({"uia":{"source":"windows-uia","operations":crate::native_accessibility::OPERATIONS,
            "scope":"selected registered native window and its descendants", "target_fields":["runtime_id","name","automation_id","control_type","class_name"],
            "pattern_support":"provider-dependent; unsupported patterns fail without a DOM fallback"}})
    }
    fn request(&self, command: &str, params: Value) -> Result<Value, String> {
        if command == "exit" {
            self.runtime.exit().map_err(|e| e.to_string())?;
            return Ok(json!({"accepted":true}));
        }
        if command == "windows" {
            let ids: Vec<_> = self.ids.lock().unwrap().iter().copied().collect();
            let mut windows = Vec::new();
            for id in ids {
                if let Ok(info) = self.runtime.with_window(id,move |window,webview| {
                    let size=window.inner_size();let outer=window.outer_size();let pos=window.outer_position().ok();
                    json!({"id":id.to_string(),"title":window.title(),"url":webview.url().ok(),"visible":window.is_visible(),
                        "focused":window.is_focused(),"minimized":window.is_minimized(),"maximized":window.is_maximized(),
                        "scale_factor":window.scale_factor(),"client":{"width":size.width,"height":size.height},
                        "outer":{"width":outer.width,"height":outer.height,"x":pos.map(|p|p.x),"y":pos.map(|p|p.y)}})
                }) { windows.push(info); }
            }
            windows.sort_by_key(|v| {
                v["id"]
                    .as_str()
                    .unwrap_or_default()
                    .parse::<u64>()
                    .unwrap_or_default()
            });
            return Ok(json!({"windows":windows}));
        }
        if command == "events" {
            let after = params["window"]
                .as_str()
                .map(str::to_string)
                .or_else(|| params["window"].as_u64().map(|id| id.to_string()));
            let events: Vec<_> = self
                .events
                .lock()
                .unwrap()
                .iter()
                .filter(|v| after.is_none() || v["window"].as_str() == after.as_deref())
                .cloned()
                .collect();
            return Ok(json!({"events":events,"capacity":512}));
        }
        let id = self.window(&params)?;
        match command {
            "uia" => {
                use tao::platform::windows::WindowExtWindows;
                let hwnd=self.runtime.with_window(id,|window,_|window.hwnd()).map_err(|e|e.to_string())?;
                let _permit = if crate::native_accessibility::is_mutating(&params) {
                    let mut busy=self.input_busy.lock().map_err(|_|"input queue poisoned")?;
                    if !busy.insert(id){return Err("another input action is active for this window".into());}
                    Some(InputPermit{busy:&self.input_busy,window:id})
                } else { None };
                crate::native_accessibility::handle(hwnd,&params,||{
                    let current=self.runtime.with_window(id,|window,_|window.hwnd()).map_err(|e|e.to_string())?;
                    if current!=hwnd {Err("native accessibility window was replaced".into())} else {Ok(())}
                })
            }
            "accessibility" => self.accessibility(id,&params),
            "audit" => self.evaluate(id,&format!(r#"(async()=>{{
                if(!window.axe?.run)throw Error('axe-core is not loaded; use the CLI --axe-source option');
                const result=await axe.run(({}??document),{});
                return {{engine:result.testEngine,environment:result.testEnvironment,url:result.url,timestamp:result.timestamp,
                    violations:result.violations,passes:result.passes,incomplete:result.incomplete,inapplicable:result.inapplicable,
                    requiresManualReview:true}};
            }})()"#,params.get("context").unwrap_or(&Value::Null),params.get("options").unwrap_or(&json!({})))),
            "cdp" => self.cdp(id,string(&params,"method")?,params.get("params").cloned().unwrap_or(json!({}))),
            "evaluate" => self.evaluate(id,string(&params,"expression")?),
            "dom" => self.evaluate(id,"({html:document.documentElement.outerHTML,url:location.href,title:document.title,readyState:document.readyState})"),
            "state" => self.evaluate(id,"({url:location.href,title:document.title,readyState:document.readyState,focused:document.activeElement?.outerHTML,state:window.__RDESKTOP_AGENT_STATE__??null,viewport:{width:innerWidth,height:innerHeight,dpr:devicePixelRatio}})"),
            "elements" => self.evaluate(id,&format!(r#"Array.from(document.querySelectorAll({})).slice(0,500).map(el=>{{const r=el.getBoundingClientRect(),s=getComputedStyle(el);return {{tag:el.tagName,id:el.id,text:(el.textContent||'').slice(0,2048),role:el.getAttribute('role'),label:el.getAttribute('aria-label'),value:el.value,visible:!!(r.width&&r.height&&s.visibility!=='hidden'&&s.display!=='none'),enabled:!el.disabled,bounds:{{x:r.x,y:r.y,width:r.width,height:r.height}},style:{{font:s.font,color:s.color,background:s.backgroundColor,display:s.display}}}};}})"#,json!(params["selector"].as_str().unwrap_or("button,input,[role]")))),
            "action" => {
                let mut busy=self.input_busy.lock().map_err(|_|"input queue poisoned")?;
                if !busy.insert(id){return Err("another input action is active for this window".into());}
                drop(busy);
                let _permit=InputPermit{busy:&self.input_busy,window:id};
                self.action(id,&params)
            },
            "screenshot" => self.cdp(id,"Page.captureScreenshot",json!({"format":"png","captureBeyondViewport":false})),
            "reload" => self.cdp(id,"Page.reload",json!({"ignoreCache":true})),
            "ipc" => self.evaluate(id,&format!("window.__RDESKTOP_INVOKE__({}, {})",json!(string(&params,"cmd")?),params.get("payload").unwrap_or(&Value::Null))),
            "window" => {
                let operation=string(&params,"operation")?.to_string();
                if operation=="close" { self.runtime.close_window(id).map_err(|e|e.to_string())?;return Ok(json!({"accepted":true})); }
                let width=params["width"].as_u64();let height=params["height"].as_u64();
                if operation=="zoom" {
                    let factor=params["factor"].as_f64().filter(|v|v.is_finite() && (0.25..=5.0).contains(v)).ok_or("zoom factor must be 0.25..5")?;
                    return self.runtime.with_window(id,move |_,view|view.zoom(factor).map(|_|json!({"success":true})).map_err(|e|e.to_string())).map_err(|e|e.to_string())?;
                }
                if operation=="resize" && (!width.is_some_and(|v|(64..=8192).contains(&v)) || !height.is_some_and(|v|(64..=8192).contains(&v))) {return Err("resize dimensions must be 64..8192 logical pixels".into());}
                self.runtime.with_window(id,move |window,_| {
                    match operation.as_str() {
                        "show"=>window.set_visible(true),"hide"=>window.set_visible(false),
                        "focus"=>window.set_focus(),"minimize"=>window.set_minimized(true),"restore"=>{window.set_minimized(false);window.set_maximized(false);},"maximize"=>window.set_maximized(true),
                        "resize"=>window.set_inner_size(tao::dpi::LogicalSize::new(width.unwrap() as u32,height.unwrap() as u32)),
                        _=>return Err("unknown window operation".to_string()),
                    }
                    Ok(json!({"success":true}))
                }).map_err(|e|e.to_string())?
            }
            _ => Err("unknown native agent command".into()),
        }
    }
}

fn string<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params[key]
        .as_str()
        .ok_or_else(|| format!("{key} must be a string"))
}
