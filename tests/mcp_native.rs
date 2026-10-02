use cc_daemon::{commands, core, files, mcp, store::id, study};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn every_advertised_tool_has_a_native_handler() {
    let missing: Vec<_> = mcp::catalog()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .filter(|name| {
            !core::handles(name)
                && !files::handles(name)
                && !commands::handles(name)
                && !study::handles(name)
                && ![
                    "read_capture",
                    "get_capture_artifacts",
                    "reset_connection_key",
                ]
                .contains(name)
        })
        .collect();
    assert!(
        missing.is_empty(),
        "Tools without a native handler: {missing:?}"
    );
}

#[test]
fn executable_serves_mcp_and_mutates_courses_with_an_empty_path() {
    let root = std::env::temp_dir().join(format!("cruise-native-mcp-{}", id()));
    fs::create_dir_all(root.join("empty-path")).unwrap();
    let workspace = root.join("content");
    let mut child = Command::new(env!("CARGO_BIN_EXE_cruise"))
        .args(["mcp", "--workspace"])
        .arg(&workspace)
        .env("PATH", root.join("empty-path"))
        .env("CRUISE_HOME", &root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","clientInfo":{"name":"native-runtime-check","version":"1"},"capabilities":{}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_course","arguments":{"code":"TEST","name":"Native backend"}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_workspace_overview","arguments":{}}}),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"reset_connection_key","arguments":{}}}),
    ];
    let mut input = child.stdin.take().unwrap();
    for request in requests {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout must contain only JSON-RPC"))
        .collect();
    assert_eq!(responses.len(), 5);
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "cruise");
    assert_eq!(
        responses[1]["result"]["tools"].as_array().unwrap().len(),
        64
    );
    for response in &responses {
        assert!(response.get("error").is_none(), "{response}");
        assert_ne!(response["result"]["isError"], true, "{response}");
    }
    let overview: Value = serde_json::from_str(
        responses[3]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(overview["courses"][0]["name"], "Native backend");
    let state: Value =
        serde_json::from_slice(&fs::read(workspace.join("state.json")).unwrap()).unwrap();
    assert_eq!(state["courses"].as_array().unwrap().len(), 1);
    let reset: Value = serde_json::from_str(
        responses[4]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let key_path = root.join("workspace/connection-key");
    let key = cc_daemon::connection::read(&key_path).unwrap();
    assert_eq!(reset["connectionKey"], key);
    let output = Command::new(env!("CARGO_BIN_EXE_cruise"))
        .arg("reset-connection-key")
        .env("CRUISE_HOME", &root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let replacement = cc_daemon::connection::read(&key_path).unwrap();
    assert_ne!(key, replacement);
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains(&replacement)
    );
    fs::remove_dir_all(root).unwrap();
}
