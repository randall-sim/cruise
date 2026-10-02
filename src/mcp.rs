//! Native MCP stdio transport. stdout is reserved for newline-delimited JSON-RPC.
use crate::store::{Result, Store};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
};

const MAX_MESSAGE: u64 = 40 * 1024 * 1024;
const PROTOCOLS: &[&str] = &[
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
    "2024-10-07",
];

pub fn catalog() -> &'static Value {
    static CATALOG: OnceLock<Value> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("mcp_catalog.json")).expect("built-in MCP catalog")
    })
}

fn defaults(schema: &Value, value: &mut Value) {
    if schema["x-cruise-trim"] == true
        && let Some(text) = value.as_str()
    {
        *value = Value::String(text.trim().to_string());
    }
    if let Some(properties) = schema["properties"].as_object()
        && let Some(object) = value.as_object_mut()
    {
        if schema["x-cruise-strip-unknown"] == true {
            object.retain(|key, _| properties.contains_key(key));
        }
        for (key, property) in properties {
            if !object.contains_key(key)
                && let Some(default) = property.get("default")
            {
                object.insert(key.clone(), default.clone());
            }
            if let Some(child) = object.get_mut(key) {
                defaults(property, child);
            }
        }
    }
    if let (Some(additional), Some(object)) = (
        schema
            .get("additionalProperties")
            .filter(|schema| schema.is_object()),
        value.as_object_mut(),
    ) {
        for (key, child) in object {
            if schema["properties"].get(key).is_none() {
                defaults(additional, child);
            }
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), value.as_array_mut()) {
        for item in values {
            defaults(items, item);
        }
    }
    // Select a matching union before applying its defaults; never merge unrelated alternatives.
    if let Some(alternatives) = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
    {
        for alternative in alternatives {
            let mut candidate = value.clone();
            defaults(alternative, &mut candidate);
            if jsonschema::is_valid(alternative, &candidate) {
                *value = candidate;
                break;
            }
        }
    }
}

pub fn validate_schema(schema: &Value, mut value: Value) -> Result<Value> {
    defaults(schema, &mut value);
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(schema)
        .map_err(|e| e.to_string())?;
    if let Some(error) = validator.iter_errors(&value).next() {
        return Err(format!("Invalid input at {}: {error}", error.instance_path));
    }
    Ok(value)
}

pub fn validate_input(name: &str, value: Value) -> Result<Value> {
    let tool = catalog()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == name)
        .ok_or_else(|| format!("Unknown tool: {name}"))?;
    validate_schema(&tool["inputSchema"], value)
}

fn text_result(value: &Value) -> Value {
    json!({"content":[{"type":"text","text":serde_json::to_string_pretty(value).unwrap()}]})
}

fn call(store: &Store, name: &str, arguments: Value) -> Result<Value> {
    let arguments = validate_input(name, arguments)?;
    if name == "reset_connection_key" {
        let key =
            crate::connection::reset(&crate::connection::key_path()).map_err(|e| e.to_string())?;
        return Ok(text_result(
            &json!({"connectionKey": key, "message": "Previous key revoked. Reconnect your browsers with this key."}),
        ));
    }
    if name == "get_capture_artifacts" {
        let state = store.read()?;
        let capture = state["lectures"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|lecture| lecture["id"] == arguments["lectureId"])
            .and_then(|lecture| lecture["captures"].as_array())
            .into_iter()
            .flatten()
            .find(|capture| capture["id"] == arguments["captureId"])
            .ok_or("Capture not found in this lecture")?;
        let mut value = json!({"lectureId":arguments["lectureId"],"captureId":arguments["captureId"],"artifacts":capture.get("artifacts").cloned().unwrap_or_else(|| json!([]))});
        if let Some(review) = capture.get("readability") {
            value["review"] = review.clone();
        }
        return Ok(text_result(&value));
    }
    if name == "read_capture" {
        let state = store.read()?;
        let source = state["jobs"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|job| job["id"] == arguments["jobId"])
            .and_then(|job| job["context"].as_array())
            .into_iter()
            .flatten()
            .find(|source| source["id"] == arguments["evidenceId"] && source["kind"] == "capture")
            .ok_or("Capture is outside this job")?;
        let path = store.path(source["path"].as_str().ok_or("Capture path is missing")?)?;
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        return Ok(
            json!({"content":[{"type":"image","data":STANDARD.encode(bytes),"mimeType":"image/png"},{"type":"text","text":serde_json::to_string(source).unwrap()}]}),
        );
    }
    let value = crate::dispatch(store, name, arguments)?;
    let mut result = text_result(&value);
    if name == "read_assignment_file"
        && let Some(media_type @ ("image/png" | "image/jpeg" | "image/webp" | "image/gif")) =
            value["mediaType"].as_str()
    {
        let path = Path::new(
            value["absolutePath"]
                .as_str()
                .ok_or("File path is missing")?,
        );
        let relative = path
            .strip_prefix(&store.root)
            .map_err(|_| "File is outside the workspace")?;
        let checked = store.path(&relative.to_string_lossy().replace('\\', "/"))?;
        let bytes = fs::read(checked).map_err(|e| e.to_string())?;
        result["content"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"image","data":STANDARD.encode(bytes),"mimeType":media_type}));
    }
    Ok(result)
}

fn rpc_error(id: Value, code: i32, message: impl ToString) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message.to_string()}})
}

fn handle(store: &Store, request: Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request["method"].as_str();
    if request["jsonrpc"] != "2.0"
        || method.is_none()
        || id
            .as_ref()
            .is_some_and(|v| !(v.is_null() || v.is_number() || v.is_string()))
    {
        return Some(rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "Invalid Request",
        ));
    }
    let id = id?; // Notifications, including initialized/cancelled, never receive replies.
    let result = match method.unwrap() {
        "initialize" => {
            let requested = request["params"]["protocolVersion"]
                .as_str()
                .unwrap_or(PROTOCOLS[0]);
            let version = if PROTOCOLS.contains(&requested) {
                requested
            } else {
                PROTOCOLS[0]
            };
            json!({"protocolVersion":version,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"cruise","version":env!("CARGO_PKG_VERSION")},"instructions":catalog()["instructions"]})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools":catalog()["tools"]}),
        "tools/call" => {
            let Some(name) = request["params"]["name"].as_str() else {
                return Some(rpc_error(id, -32602, "Tool name is required"));
            };
            let arguments = request["params"]
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match call(store, name, arguments) {
                Ok(result) => result,
                Err(message) => json!({"content":[{"type":"text","text":message}],"isError":true}),
            }
        }
        _ => return Some(rpc_error(id, -32601, "Method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

fn transport(store: &Store, mut input: impl BufRead, mut output: impl Write) -> Result<()> {
    loop {
        let mut line = Vec::new();
        let count = std::io::Read::by_ref(&mut input)
            .take(MAX_MESSAGE + 1)
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(());
        }
        if count as u64 > MAX_MESSAGE {
            return Err("MCP message exceeds 40 MiB".into());
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let response = match serde_json::from_slice(&line) {
            Ok(request) => handle(store, request),
            Err(_) => Some(rpc_error(Value::Null, -32700, "Parse error")),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut output, &response).map_err(|e| e.to_string())?;
            output
                .write_all(b"\n")
                .and_then(|_| output.flush())
                .map_err(|e| e.to_string())?;
        }
    }
}

pub fn serve(store: &Store) -> Result<()> {
    transport(store, io::stdin().lock(), io::stdout().lock())
}

/// Checks the same native transport used by MCP clients, without changing course content.
pub fn check(store: &Store) -> Result<usize> {
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\"}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"get_workspace_overview\",\"arguments\":{}}}\n"
    );
    let mut output = Vec::new();
    transport(store, io::Cursor::new(input), &mut output)?;
    let responses: Vec<Value> = output
        .split(|b| *b == b'\n')
        .filter(|v| !v.is_empty())
        .map(|v| serde_json::from_slice(v).map_err(|e| e.to_string()))
        .collect::<Result<_>>()?;
    if responses.len() != 3 || responses[2]["result"]["isError"] == true {
        return Err(format!(
            "MCP workspace check failed: {}",
            responses.last().unwrap_or(&Value::Null)
        ));
    }
    Ok(responses[1]["result"]["tools"]
        .as_array()
        .ok_or("MCP catalog missing")?
        .len())
}

pub fn configure(project: &Path, windows_host: bool) -> Result<PathBuf> {
    let project = fs::canonicalize(project).map_err(|e| e.to_string())?;
    let binary = std::env::current_exe().map_err(|e| e.to_string())?;
    let quote = |value: &str| serde_json::to_string(value).unwrap();
    let (command, args, cwd) = if windows_host && cfg!(unix) {
        let distribution = std::env::var("WSL_DISTRO_NAME")
            .map_err(|_| "--windows-host requires a WSL distribution")?;
        let cwd = format!(
            "\\\\wsl.localhost\\{}{}",
            distribution,
            project.to_string_lossy().replace('/', "\\")
        );
        (
            "wsl.exe".to_string(),
            vec![
                "--distribution".into(),
                distribution,
                "--cd".into(),
                project.to_string_lossy().into_owned(),
                "--exec".into(),
                binary.to_string_lossy().into_owned(),
                "mcp".into(),
            ],
            cwd,
        )
    } else {
        (
            binary.to_string_lossy().into_owned(),
            vec!["mcp".into()],
            project.to_string_lossy().into_owned(),
        )
    };
    let begin = "# BEGIN Course Captain managed MCP";
    let end = "# END Course Captain managed MCP";
    let block = format!(
        "{begin}\n# Regenerate after moving: cruise setup-codex{}\n[mcp_servers.course-captain]\ncommand = {}\nargs = [{}]\ncwd = {}\nstartup_timeout_sec = 60\ntool_timeout_sec = 120\n{end}",
        if windows_host { " --windows-host" } else { "" },
        quote(&command),
        args.iter().map(|s| quote(s)).collect::<Vec<_>>().join(", "),
        quote(&cwd)
    );
    let dir = project.join(".codex");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join("config.toml");
    let previous = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.to_string()),
    };
    let updated = match (previous.find(begin), previous.find(end)) {
        (Some(start), Some(finish)) if finish >= start => format!("{}{}{}", &previous[..start], block, &previous[finish + end.len()..]),
        (None, None) if !previous.contains("[mcp_servers.course-captain]") && !previous.contains("[mcp_servers.\"course-captain\"]") => format!("{}{}{block}\n", previous.trim_end(), if previous.trim().is_empty() {""} else {"\n\n"}),
        _ => return Err("Existing course-captain MCP configuration is unmanaged or incomplete; review it before replacing it".into()),
    };
    fs::write(&file, updated).map_err(|e| e.to_string())?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_preserve_defaults_nested_bounds_and_patch_fields() {
        let input = validate_input("list_lectures", json!({"courseId":"cs101"})).unwrap();
        assert_eq!(input["offset"], 0);
        assert_eq!(input["limit"], 25);
        assert!(validate_input("list_lectures", json!({"courseId":"../escape"})).is_err());
        assert!(validate_input("list_lectures", json!({"courseId":"cs101","limit":101})).is_err());
        let patch = validate_input(
            "update_course",
            json!({"courseId":"cs101","changes":{"name":"New name"}}),
        )
        .unwrap();
        assert_eq!(patch["changes"], json!({"name":"New name"}));
        assert!(
            validate_input(
                "update_course",
                json!({"courseId":"cs101","changes":{"unknown":true}})
            )
            .is_err()
        );
        let input = validate_input(
            "prepare_exam",
            json!({"courseId":"cs101","prompt":"  Review trees  ","unknown":true}),
        )
        .unwrap();
        assert_eq!(input["prompt"], "Review trees");
        assert!(input.get("unknown").is_none());
        assert!(
            validate_input("prepare_exam", json!({"courseId":"cs101","prompt":"   "})).is_err()
        );
        for tool in catalog()["tools"].as_array().unwrap() {
            jsonschema::validator_for(&tool["inputSchema"]).unwrap();
        }
    }

    #[test]
    fn stdio_frames_notifications_errors_and_tool_catalog() {
        let store = Store::new(std::env::temp_dir().join("cruise-mcp-no-workspace-access"));
        let input = concat!(
            "not json\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":\"hello\",\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"list_lectures\",\"arguments\":{}}}\n"
        );
        let mut output = Vec::new();
        transport(&store, io::Cursor::new(input), &mut output).unwrap();
        let responses: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(responses.len(), 4);
        assert_eq!(responses[0]["error"]["code"], -32700);
        assert_eq!(responses[1]["id"], "hello");
        assert_eq!(responses[1]["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(
            responses[2]["result"]["tools"].as_array().unwrap().len(),
            64
        );
        assert_eq!(responses[3]["result"]["isError"], true);
    }

    #[test]
    fn project_configuration_preserves_unmanaged_settings() {
        let root = std::env::temp_dir().join(format!("cruise-config-{}", crate::store::id()));
        fs::create_dir_all(root.join(".codex")).unwrap();
        let path = root.join(".codex/config.toml");
        fs::write(&path, "model = \"keep-this\"\n# BEGIN Course Captain managed MCP\nold setting\n# END Course Captain managed MCP\n[features]\nkeep_me = true\n").unwrap();
        configure(&root, false).unwrap();
        let updated = fs::read_to_string(&path).unwrap();
        assert!(updated.starts_with("model = \"keep-this\""));
        assert!(updated.ends_with("[features]\nkeep_me = true\n"));
        assert!(updated.contains("args = [\"mcp\"]"));
        assert!(!updated.contains("old setting"));
        fs::write(
            &path,
            "[mcp_servers.course-captain]\ncommand = \"unmanaged\"\n",
        )
        .unwrap();
        assert!(configure(&root, false).is_err());
        assert!(fs::read_to_string(&path).unwrap().contains("unmanaged"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn capture_images_require_job_evidence_and_safe_workspace_paths() {
        let root = std::env::temp_dir().join(format!("cruise-mcp-image-{}", crate::store::id()));
        let store = Store::new(root.clone());
        store.write("capture.png", b"image bytes").unwrap();
        store.mutate(|state| {
            state["jobs"] = json!([{"id":"job", "context":[{"id":"capture", "kind":"capture", "path":"capture.png"}]}]);
            Ok(())
        }).unwrap();
        let result = call(
            &store,
            "read_capture",
            json!({"jobId":"job","evidenceId":"capture"}),
        )
        .unwrap();
        assert_eq!(result["content"][0]["mimeType"], "image/png");
        assert_eq!(
            result["content"][0]["data"],
            STANDARD.encode(b"image bytes")
        );
        assert!(
            call(
                &store,
                "read_capture",
                json!({"jobId":"other","evidenceId":"capture"})
            )
            .is_err()
        );
        store
            .mutate(|state| {
                state["jobs"][0]["context"][0]["path"] = json!("../outside");
                Ok(())
            })
            .unwrap();
        assert!(
            call(
                &store,
                "read_capture",
                json!({"jobId":"job","evidenceId":"capture"})
            )
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
