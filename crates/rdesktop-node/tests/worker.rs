//! Explicit local integration: choose Node and its expected SHA-256 yourself.
//! RDESKTOP_TEST_NODE=... RDESKTOP_TEST_NODE_SHA256=... cargo test -p rdesktop-node -- --ignored
use rdesktop_node::{NodeConfig, NodeError, NodeRuntime};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::TempDir;

fn config(root: &TempDir) -> NodeConfig {
    NodeConfig {
        executable: PathBuf::from(
            std::env::var_os("RDESKTOP_TEST_NODE").expect("choose RDESKTOP_TEST_NODE"),
        ),
        executable_sha256: std::env::var("RDESKTOP_TEST_NODE_SHA256")
            .expect("pin RDESKTOP_TEST_NODE_SHA256"),
        package_root: root.path().into(),
        preload: root.path().join("preload.cjs"),
        timeout: Duration::from_secs(5),
    }
}
fn runtime(source: &str) -> (TempDir, NodeRuntime) {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("preload.cjs"), source).unwrap();
    let worker = NodeRuntime::start(config(&root)).unwrap();
    (root, worker)
}

#[test]
fn invalid_configuration_fails_before_launch() {
    let root = tempfile::tempdir().unwrap();
    let bad = NodeConfig {
        executable: PathBuf::new(),
        executable_sha256: String::new(),
        package_root: root.path().into(),
        preload: root.path().join("missing.cjs"),
        timeout: Duration::ZERO,
    };
    assert!(matches!(
        NodeRuntime::start(bad),
        Err(NodeError::Configuration(_))
    ));
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn commonjs_dependencies_and_real_filesystem_without_electron() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("helper.cjs"),
        "module.exports = require('node:path').join;",
    )
    .unwrap();
    fs::write(root.path().join("preload.cjs"), r#"
const fs=require('node:fs'),join=require('./helper.cjs');
let counter=0;
window.create=folder=>{const target=join(folder,'真实 Node 目录');fs.mkdirSync(target);return target;};
window.count=()=>++counter;
window.identity=()=>({node:process.versions.node,electron:process.versions.electron??null});
process.on('exit',code=>fs.writeFileSync('normal-exit.txt',String(code)));
"#).unwrap();
    let worker = NodeRuntime::start(config(&root)).unwrap();
    assert!(worker.metadata().electron.is_none());
    assert!(!worker.metadata().node.is_empty());
    assert!(worker.process_id() > 0);
    let result = worker.call("create", &[json!(root.path())]).unwrap();
    assert!(PathBuf::from(result.as_str().unwrap()).is_dir());
    assert_eq!(worker.call("count", &[]).unwrap(), 1);
    assert_eq!(worker.call("count", &[]).unwrap(), 2);
    worker.close().unwrap();
    assert_eq!(
        fs::read_to_string(root.path().join("normal-exit.txt")).unwrap(),
        "0"
    );
    assert!(matches!(worker.call("count", &[]), Err(NodeError::Closed)));
    worker.close().unwrap();
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn typed_preload_errors_do_not_poison_the_next_call() {
    let (_root, worker)=runtime("window.fail=()=>{const e=new TypeError('expected failure');e.code='OWNED_FIXTURE';throw e;};window.ok=()=>42;");
    match worker.call("fail", &[]) {
        Err(NodeError::Preload(error)) => {
            assert_eq!(error.name, "TypeError");
            assert_eq!(error.code.as_deref(), Some("OWNED_FIXTURE"));
            assert_eq!(error.message, "expected failure");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(worker.call("ok", &[]).unwrap(), 42);
    assert!(matches!(
        worker.call("constructor", &[]),
        Err(NodeError::Configuration(_))
    ));
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn promise_results_are_awaited_and_values_are_unambiguous() {
    let (_root, worker) = runtime(
        r#"
window.result=async()=>{await new Promise(r=>setTimeout(r,20));return [undefined,13n,Buffer.from('中文'),{$rdesktop:'buffer',name:'kept'}];};
"#,
    );
    let result = worker.call("result", &[]).unwrap();
    assert_eq!(
        result,
        json!([
            {"$rdesktop":"undefined"},{"$rdesktop":"bigint","value":"13"},
            {"$rdesktop":"buffer","base64":"5Lit5paH"},
            {"$rdesktop":"object","entries":[["$rdesktop","buffer"],["name","kept"]]}
        ])
    );
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn cyclic_and_oversized_responses_are_errors_and_leave_the_worker_usable() {
    let (_root,worker)=runtime("window.cycle=()=>{const a={};a.self=a;return a;};window.large=()=> 'a'.repeat(5*1024*1024);window.ok=()=>true;");
    assert!(matches!(
        worker.call("cycle", &[]),
        Err(NodeError::Preload(_))
    ));
    assert!(matches!(
        worker.call("large", &[]),
        Err(NodeError::Preload(_))
    ));
    assert_eq!(worker.call("ok", &[]).unwrap(), true);
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn request_bounds_do_not_dispatch_and_are_recoverable() {
    let (_root, worker) = runtime("let n=0;window.count=()=>++n;");
    assert!(matches!(
        worker.call("count", &[json!("a".repeat(5 * 1024 * 1024))]),
        Err(NodeError::Configuration(_))
    ));
    assert_eq!(worker.call("count", &[]).unwrap(), 1);
    assert!(matches!(
        worker.call_with_timeout("count", &[], Duration::ZERO),
        Err(NodeError::Configuration(_))
    ));
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn plugin_console_and_stdout_cannot_corrupt_the_transport() {
    let (_root,worker)=runtime("console.log('preload log');process.stdout.write('preload stdout');window.ok=()=>{console.error('call log');process.stdout.write('call stdout');return 'ok';};");
    assert_eq!(worker.call("ok", &[]).unwrap(), "ok");
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn pending_promise_deadline_closes_the_owned_worker_without_retrying() {
    let (_root, worker) = runtime("window.never=()=>new Promise(()=>{});");
    let start = Instant::now();
    assert!(matches!(
        worker.call_with_timeout("never", &[], Duration::from_millis(50)),
        Err(NodeError::Timeout)
    ));
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(matches!(worker.call("never", &[]), Err(NodeError::Closed)));
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn busy_workers_reject_concurrent_requests_instead_of_queueing_them() {
    let (root,worker)=runtime("const fs=require('node:fs');window.slow=async file=>{fs.writeFileSync(file,'started');await new Promise(r=>setTimeout(r,200));return 1;};window.fast=()=>2;");
    let worker = Arc::new(worker);
    let calling = worker.clone();
    let marker = root.path().join("started");
    let worker_marker = marker.clone();
    let thread = std::thread::spawn(move || calling.call("slow", &[json!(worker_marker)]));
    let deadline = Instant::now() + Duration::from_secs(2);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(marker.exists());
    assert!(matches!(worker.call("fast", &[]), Err(NodeError::Busy)));
    assert_eq!(thread.join().unwrap().unwrap(), 1);
    assert_eq!(worker.call("fast", &[]).unwrap(), 2);
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn stalled_node_input_cannot_block_the_calling_thread_past_its_deadline() {
    let (root,worker)=runtime("const fs=require('node:fs');window.spin=file=>{fs.writeFileSync(file,'started');setImmediate(()=>{for(;;){}});};window.echo=value=>value;");
    let marker = root.path().join("spinning");
    worker.call("spin", &[json!(marker)]).unwrap();
    // The next request is larger than a pipe buffer; its write must run on an
    // owned I/O worker, not synchronously before recv_timeout starts.
    let start = Instant::now();
    assert!(matches!(
        worker.call_with_timeout(
            "echo",
            &[Value::String("x".repeat(512 * 1024))],
            Duration::from_millis(50)
        ),
        Err(NodeError::Timeout)
    ));
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn wrong_executable_hash_and_outside_preloads_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("preload.cjs"), "window.ok=()=>true;").unwrap();
    let mut bad = config(&root);
    bad.executable_sha256 = "0".repeat(64);
    assert!(matches!(
        NodeRuntime::start(bad),
        Err(NodeError::Configuration(_))
    ));
    let outside = tempfile::NamedTempFile::new().unwrap();
    let mut bad = config(&root);
    bad.preload = outside.path().into();
    assert!(matches!(
        NodeRuntime::start(bad),
        Err(NodeError::Configuration(_))
    ));
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn failed_startup_does_not_leave_a_preload_worker_running() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("preload.cjs"),
        "process.on('exit',code=>require('node:fs').writeFileSync('failed-startup-exit.txt',String(code)));throw new Error('owned startup failure');",
    )
    .unwrap();
    match NodeRuntime::start(config(&root)) {
        Err(NodeError::Preload(error)) => assert_eq!(error.message, "owned startup failure"),
        _ => panic!("startup must return the original preload error"),
    }
    assert_eq!(
        fs::read_to_string(root.path().join("failed-startup-exit.txt")).unwrap(),
        "0"
    );
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn malformed_response_identity_closes_the_worker() {
    let (_root, worker) =
        runtime("window.bad=()=>{require('node:fs').writeSync(1,'{}\\n');return true;};");
    assert!(matches!(
        worker.call("bad", &[]),
        Err(NodeError::Protocol(_))
    ));
    assert!(matches!(worker.call("bad", &[]), Err(NodeError::Closed)));
}

#[test]
#[ignore = "requires an explicitly pinned local Node executable"]
fn oversized_unterminated_response_is_bounded_without_waiting_for_a_newline() {
    let (_root, worker) =
        runtime("window.bad=()=>require('node:fs').writeSync(1,Buffer.alloc(5*1024*1024,65));");
    let start = Instant::now();
    assert!(matches!(
        worker.call("bad", &[]),
        Err(NodeError::Protocol(_))
    ));
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
#[ignore = "requires pinned Node and immutable fugary/utools-file-to-folder checkout"]
fn unchanged_public_plugin_preload_creates_a_controlled_directory_without_electron() {
    let plugin = PathBuf::from(
        std::env::var_os("RDESKTOP_TEST_PUBLIC_PLUGIN")
            .expect("choose the documented immutable public plugin checkout"),
    );
    // ISC fugary/utools-file-to-folder, commit 8a3f257591935cf4d2dd4baa599f3c20fc1b7e5d.
    for (name, expected) in [
        (
            "preload.js",
            "87ff30f9890916209c73cf040da30771b4fb94e79bb39dfbbbf60ff3b45522a8",
        ),
        (
            "utils.js",
            "f77a04ef280103ce904f8582d5f4cac8ffc1d73a6fe44c502cf2ec20e974acd2",
        ),
    ] {
        assert_eq!(
            format!("{:x}", Sha256::digest(fs::read(plugin.join(name)).unwrap())),
            expected
        );
    }
    let output = tempfile::tempdir().unwrap();
    let mut settings = config(&output);
    settings.package_root = plugin.clone();
    settings.preload = plugin.join("preload.js");
    let worker = NodeRuntime::start(settings).unwrap();
    assert!(worker.metadata().electron.is_none());
    assert_eq!(worker.metadata().functions.len(), 5);
    let destination = output.path().join("原始插件 preload 验收");
    let result = worker
        .call("createCurrentFolder", &[json!(destination)])
        .unwrap();
    assert_eq!(result, json!({"$rdesktop":"undefined"}));
    assert!(destination.is_dir());
    worker.close().unwrap();
    // This proves the original CommonJS/Node code, not its HTML, DOM, uTools
    // lifecycle or screen-reader behavior. Those remain separate native gates.
}
