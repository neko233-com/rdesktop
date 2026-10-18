use rdesktop_node::{renderer::RendererBridge, NodeConfig, NodeError, NodeRuntime};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
    time::{Duration, Instant},
};

fn config(root: &std::path::Path) -> NodeConfig {
    NodeConfig {
        executable: PathBuf::from(std::env::var_os("RDESKTOP_TEST_NODE").unwrap()),
        executable_sha256: std::env::var("RDESKTOP_TEST_NODE_SHA256").unwrap(),
        package_root: root.into(),
        preload: root.join("preload.cjs"),
        timeout: Duration::from_secs(3),
    }
}
fn get(node: &NodeRuntime, object: &Value, key: &str) -> Value {
    node.module_request(json!({"operation":"get","id":object["id"],"key":key}))
        .unwrap()["value"]
        .clone()
}
fn call(node: &NodeRuntime, function: &Value, args: Value) -> Result<Value, NodeError> {
    node.module_request(
        json!({"operation":"call","id":function["id"],"receiver":function["receiver"],"args":args}),
    )
}
fn request(url: &str, method: &str, target: &str, origin: &str, token: &str, body: &str) -> String {
    let address = url
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let mut socket = TcpStream::connect(address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(socket,"{method} {target} HTTP/1.1\r\nHost: {address}\r\nOrigin: {origin}\r\nX-Rdesktop-Node: {token}\r\nX-Rdesktop-Document: 11111111111111111111111111111111\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    let mut result = String::new();
    socket.read_to_string(&mut result).unwrap();
    result
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn renderer_preload_is_not_executed_in_node_and_module_methods_are_real() {
    let root = tempfile::tempdir().unwrap();
    let preload = root.path().join("preload.cjs");
    fs::write(
        &preload,
        "window.actualDom=document.querySelector('#actual');",
    )
    .unwrap();
    let node = NodeRuntime::start_modules(config(root.path())).unwrap();
    assert!(node.metadata().functions.is_empty());
    assert!(node.metadata().electron.is_none());
    let source = node.module_request(json!({"operation":"preload"})).unwrap();
    assert!(source["source"]
        .as_str()
        .unwrap()
        .contains("document.querySelector"));
    let native = node
        .module_request(json!({"operation":"resolve","name":"node:fs","parent":preload}))
        .unwrap()["value"]
        .clone();
    let file = root.path().join("真实 中文.txt");
    call(
        &node,
        &get(&node, &native, "writeFileSync"),
        json!([file, "真实同步文件"]),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "真实同步文件");
    let stat =
        call(&node, &get(&node, &native, "statSync"), json!([file])).unwrap()["value"].clone();
    assert_eq!(
        call(&node, &get(&node, &stat, "isFile"), json!([])).unwrap()["value"],
        true
    );
    let error = call(
        &node,
        &get(&node, &native, "statSync"),
        json!([root.path().join("missing")]),
    );
    assert!(
        matches!(error,Err(NodeError::Preload(error)) if error.code.as_deref()==Some("ENOENT"))
    );
    node.close().unwrap();
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn real_async_callbacks_and_promises_use_the_scoped_event_queue() {
    let root = tempfile::tempdir().unwrap();
    let preload = root.path().join("preload.cjs");
    fs::write(&preload, "").unwrap();
    let file = root.path().join("callback.txt");
    fs::write(&file, "callback 中文").unwrap();
    let node = NodeRuntime::start_modules(config(root.path())).unwrap();
    let fs = node
        .module_request(json!({"operation":"resolve","name":"fs","parent":preload}))
        .unwrap()["value"]
        .clone();
    call(
        &node,
        &get(&node, &fs, "readFile"),
        json!([file,{"$rdesktop":"callback","id":"read-1"}]),
    )
    .unwrap();
    let promised = call(
        &node,
        &get(&node, &get(&node, &fs, "promises"), "readFile"),
        json!([file]),
    )
    .unwrap()["value"]
        .clone();
    assert_eq!(promised["kind"], "promise");
    let end = Instant::now() + Duration::from_secs(2);
    let mut events = vec![];
    while Instant::now() < end {
        events.extend(
            node.module_request(json!({"operation":"events"})).unwrap()["events"]
                .as_array()
                .unwrap()
                .clone(),
        );
        if events.len() >= 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let callback = events.iter().find(|e| e["kind"] == "callback").unwrap();
    assert_eq!(callback["id"], "read-1");
    assert_eq!(callback["args"][0], Value::Null);
    assert_eq!(callback["args"][1]["$rdesktop"], "bytes");
    assert!(events
        .iter()
        .any(|e| e["kind"] == "promise" && e["id"] == promised["id"] && e["success"] == true));
    node.close().unwrap();
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn renderer_memory_preserves_native_aliases_errors_and_bounds() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("preload.cjs"), "").unwrap();
    let node = NodeRuntime::start_modules(config(root.path())).unwrap();
    let vm = node.module_request(json!({"operation":"resolve","name":"node:vm","parent":root.path().join("preload.cjs")})).unwrap()["value"].clone();
    let mutate = call(
        &node,
        &get(&node, &vm, "runInNewContext"),
        json!(["(a,b)=>{a[0]=9;b[0]=8;return a;}"]),
    )
    .unwrap()["value"]
        .clone();
    let view = |id, offset, length| json!({"$rdesktop":"memory-view","id":id,"backing":"b1","offset":offset,"length":length,"byteLength":length,"type":"Buffer"});
    let initial =
        json!([{"id":"b1","byteLength":8,"offset":2,"base64":"AQIDBA==","writes":[[0,4]]}]);
    let result = node.module_request(json!({"operation":"call","rendererMemory":true,"id":mutate["id"],"args":[view("v2",2,4),view("v3",3,2)],"memory":initial})).unwrap();
    assert_eq!(result["value"]["id"], "v2");
    assert_eq!(result["value"]["base64"], "CQgDBA==");
    assert_eq!(result["memory"]["changes"][0]["offset"], 2);
    assert_eq!(result["memory"]["changes"][0]["base64"], "CQg=");
    let failed = call(
        &node,
        &get(&node, &vm, "runInNewContext"),
        json!(["b=>{b[2]=7;throw new TypeError('after-write');}"]),
    )
    .unwrap()["value"]
        .clone();
    let result = node.module_request(json!({"operation":"call","rendererMemory":true,"id":failed["id"],"args":[view("v2",2,4)],"memory":[{"id":"b1","byteLength":8,"offset":2,"base64":"CQgDBA==","writes":[]}]})).unwrap();
    assert_eq!(result["thrown"]["name"], "TypeError");
    assert_eq!(result["memory"]["changes"][0]["offset"], 4);
    assert_eq!(result["memory"]["changes"][0]["base64"], "Bw==");
    let bounded = node.module_request(json!({"operation":"events","rendererMemory":true,"memory":[{"id":"b99","byteLength":67108865,"offset":0,"base64":"","writes":[]}]})).unwrap();
    assert_eq!(bounded["thrown"]["name"], "TypeError");
    let invalid = node.module_request(json!({"operation":"call","rendererMemory":true,"id":mutate["id"],"args":[view("v2",2,100)],"memory":[]})).unwrap();
    assert_eq!(invalid["thrown"]["name"], "RangeError");
    let large = call(
        &node,
        &get(&node, &vm, "runInNewContext"),
        json!(["b=>{globalThis.kept=b;b.fill(65);return b;}"]),
    )
    .unwrap()["value"]
        .clone();
    let timers = node.module_request(json!({"operation":"resolve","name":"node:timers","parent":root.path().join("preload.cjs")})).unwrap()["value"].clone();
    call(
        &node,
        &get(&node, &timers, "setTimeout"),
        json!([{"$rdesktop":"callback","id":"oversize-preserves-callback"},0]),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let size = 2 * 1024 * 1024;
    let zeros = "AAAA".repeat(size / 3) + "AAA=";
    let oversized = node.module_request(json!({"operation":"call","rendererMemory":true,"id":large["id"],"args":[{"$rdesktop":"memory-view","id":"v99","backing":"b99","offset":0,"length":size,"byteLength":size,"type":"Buffer"}],"memory":[{"id":"b99","byteLength":size,"offset":0,"base64":zeros,"writes":[[0,size]]}]}));
    assert!(
        matches!(oversized, Err(NodeError::Preload(error)) if error.name == "RangeError" && error.message.contains("frame limit"))
    );
    let pending = node
        .module_request(json!({"operation":"events","rendererMemory":true}))
        .unwrap();
    assert!(pending["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["id"] == "oversize-preserves-callback"));
    let change = pending["memory"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|change| change["id"] == "b99")
        .unwrap();
    assert!(change["base64"].as_str().unwrap().starts_with("QUFB"));
    assert_eq!(change["offset"], 0);
    node.close().unwrap();
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn copied_buffer_commands_preserve_aliases_and_leave_callbacks_for_polling() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("preload.cjs"), "").unwrap();
    fs::write(root.path().join("callback.txt"), "buffer-callback").unwrap();
    let node = NodeRuntime::start_modules(config(root.path())).unwrap();
    let view = |id, offset, length| json!({"$buffer":"view","backing":0,"id":id,"offset":offset,"length":length,"type":"Buffer"});
    let result = node.module_request(json!({"operation":"buffer","kind":"method","name":"copy","receiver":view(0,0,4),"args":[view(1,2,4)],"backings":["AQIDBAUG"]})).unwrap();
    assert_eq!(result["value"], 4);
    assert_eq!(result["changes"][0]["base64"], "AQIBAgME");
    let fs_module = node
        .module_request(
            json!({"operation":"resolve","name":"fs","parent":root.path().join("preload.cjs")}),
        )
        .unwrap()["value"]
        .clone();
    call(&node, &get(&node, &fs_module, "readFile"), json!([root.path().join("callback.txt"),{"$rdesktop":"callback","id":"buffer-preserves-callback"}])).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let result = node.module_request(json!({"operation":"buffer","kind":"method","name":"fill","receiver":view(0,0,4),"args":[7],"backings":["AQIDBA=="]})).unwrap();
    assert_eq!(result["value"], json!({"$buffer":"same","id":0}));
    assert_eq!(result["changes"][0]["base64"], "BwcHBw==");
    assert_eq!(result["events"], json!([]));
    let events = node.module_request(json!({"operation":"events"})).unwrap();
    assert!(events["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["id"] == "buffer-preserves-callback"));
    for invalid in [
        json!({"$rdesktop":"callback","id":"rejected"}),
        json!({"$rdesktop":"reference","id":"1"}),
    ] {
        let rejected = node.module_request(json!({"operation":"buffer","kind":"method","name":"fill","receiver":view(0,0,4),"args":[invalid],"backings":["AQIDBA=="]}));
        assert!(matches!(rejected, Err(NodeError::Preload(error)) if error.name == "TypeError"));
    }
    node.close().unwrap();
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn private_renderer_origin_rejects_other_origins_methods_and_paths_and_closes() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("preload.cjs"), "").unwrap();
    fs::write(root.path().join("index.html"), "<h1>real package</h1>").unwrap();
    let bridge = RendererBridge::start(config(root.path()), "index.html").unwrap();
    let url = bridge.url().to_owned();
    let address = url
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let origin = format!("http://{address}");
    let prefix = url
        .split_once(address)
        .unwrap()
        .1
        .trim_end_matches("index.html");
    let token = prefix.trim_matches('/');
    let endpoint = format!("{prefix}__node");
    assert!(request(
        &url,
        "GET",
        &format!("{prefix}index.html"),
        &origin,
        token,
        ""
    )
    .starts_with("HTTP/1.1 200"));
    let body = r#"{"operation":"events"}"#;
    assert!(request(&url, "POST", &endpoint, &origin, token, body).starts_with("HTTP/1.1 403"));
    assert!(request(
        &url,
        "POST",
        &endpoint,
        &origin,
        token,
        r#"{"operation":"attach"}"#
    )
    .starts_with("HTTP/1.1 200"));
    assert!(request(&url, "POST", &endpoint, &origin, token, body).starts_with("HTTP/1.1 200"));
    assert!(request(
        &url,
        "POST",
        &endpoint,
        "https://unrelated.example",
        token,
        body
    )
    .starts_with("HTTP/1.1 403"));
    assert!(request(&url, "POST", &endpoint, &origin, "wrong", body).starts_with("HTTP/1.1 403"));
    assert!(request(&url, "GET", &endpoint, &origin, token, "").starts_with("HTTP/1.1 403"));
    assert!(request(
        &url,
        "GET",
        &format!("{prefix}%2E%2E/private"),
        &origin,
        token,
        ""
    )
    .starts_with("HTTP/1.1 403"));
    let outside = tempfile::tempdir().unwrap();
    let other = outside.path().join("outside-preload.cjs");
    fs::write(&other, "module.exports=1").unwrap();
    let escape =
        json!({"operation":"resolve","name":other,"parent":root.path().join("preload.cjs")})
            .to_string();
    assert!(request(&url, "POST", &endpoint, &origin, token, &escape)
        .contains("outside its package root"));
    fs::remove_file(other).unwrap();
    drop(bridge);
    assert!(TcpStream::connect(address).is_err());
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn accepted_windows_sockets_wait_for_split_headers_instead_of_aborting() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("preload.cjs"), "").unwrap();
    fs::write(root.path().join("index.html"), "split request passed").unwrap();
    let bridge = RendererBridge::start(config(root.path()), "index.html").unwrap();
    let url = bridge.url().strip_prefix("http://").unwrap();
    let (address, target) = url.split_once('/').unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    // A real browser can connect before its headers/body become available.
    std::thread::sleep(Duration::from_millis(100));
    write!(stream, "GET /{target} HTTP/1.1\r\nHost: {address}\r\n").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    stream.write_all(b"Connection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.ends_with("split request passed"));
}
