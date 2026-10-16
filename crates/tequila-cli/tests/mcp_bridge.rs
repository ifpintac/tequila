//! The built-in MCP bridge drives a real Tequila: a headless session
//! (`--serve`) holds a scene, and `--mcp`, started as an AI app starts it,
//! reads the scene, renames a part and takes it back, over JSON-RPC on its
//! standard input and output.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn an_ai_app_works_in_tequila_through_the_bridge() {
    let exe = env!("CARGO_BIN_EXE_tequila");
    let folder = std::env::temp_dir().join(format!("tequila-mcp-bridge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let discovery = folder.join("channel.json");

    // A headless Tequila with a part in it: a cube from an OBJ.
    let model = folder.join("cube.obj");
    let mut obj = String::new();
    for i in 0..8 {
        obj += &format!("v {} {} {}\n", i & 1, (i >> 1) & 1, (i >> 2) & 1);
    }
    for face in ["1 3 4 2", "5 6 8 7", "1 2 6 5", "3 7 8 4", "1 5 7 3", "2 4 8 6"] {
        obj += &format!("f {face}\n");
    }
    std::fs::write(&model, obj).unwrap();
    let mut session = Command::new(exe)
        .args(["--serve", "--attached", "--discovery"])
        .arg(&discovery)
        .arg("-i")
        .arg(&model)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("starts the headless session");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !discovery.exists() {
        assert!(Instant::now() < deadline, "the headless session never announced itself");
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut bridge = Command::new(exe)
        .arg("--mcp")
        .arg("--discovery")
        .arg(&discovery)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("starts the bridge");
    let mut to = bridge.stdin.take().unwrap();
    let mut from = BufReader::new(bridge.stdout.take().unwrap());
    let mut ask = |message: &str| -> tequila_format::json::Value {
        writeln!(to, "{message}").unwrap();
        to.flush().unwrap();
        let mut line = String::new();
        from.read_line(&mut line).expect("an answer");
        tequila_format::json::parse(line.as_bytes()).unwrap_or_else(|e| panic!("{e}: {line}"))
    };
    let said = |answer: &tequila_format::json::Value| -> String {
        let result = answer.get("result").unwrap_or_else(|| panic!("{answer:?}"));
        assert_eq!(result.get("isError").and_then(|v| v.as_bool()), Some(false), "{answer:?}");
        result.get("content").unwrap().as_array()[0].get("text").and_then(|v| v.as_str()).unwrap().to_string()
    };

    let hello = ask(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#);
    assert!(hello.get("result").is_some(), "{hello:?}");

    let scene = said(&ask(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tequila_scene"}}"#));
    assert!(!scene.is_empty());

    let nodes = ask(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tequila_nodes","arguments":{"parts_only":true}}}"#);
    let listed = tequila_format::json::parse(said(&nodes).as_bytes()).unwrap();
    let part = &listed.get("nodes").unwrap().as_array()[0];
    let id = part.get("id").and_then(|v| v.as_f64()).unwrap() as u64;
    assert!(part.get("size").is_some(), "the slim list keeps each part's size: {part:?}");

    let rename = format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"tequila_apply","arguments":{{"command":{{"command":"scene.rename","node":{id},"name":"Renamed by AI"}}}}}}}}"#
    );
    said(&ask(&rename));
    let after = said(&ask(r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"tequila_nodes","arguments":{"filter":"renamed by ai"}}}"#));
    assert!(after.contains("Renamed by AI"), "the rename did not happen: {after}");

    said(&ask(r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"tequila_undo"}}"#));
    let undone = said(&ask(r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"tequila_nodes","arguments":{"filter":"renamed by ai"}}}"#));
    assert!(undone.contains("\"count\": 0"), "undo did not take it back: {undone}");

    // A refusal comes back as a result to read, not a broken connection.
    let refused = ask(r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"tequila_apply","arguments":{"command":{"command":"no.such_command"}}}}"#);
    assert_eq!(refused.get("result").unwrap().get("isError").and_then(|v| v.as_bool()), Some(true));

    drop(to);
    let _ = bridge.wait();
    drop(session.stdin.take());
    let _ = session.wait();
    let _ = std::fs::remove_dir_all(&folder);
}
