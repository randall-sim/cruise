use crate::{core::text, dispatch, store::*};
use axum::{
    body::Body,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

fn json_response(value: Value) -> Response {
    axum::Json(value).into_response()
}
pub fn error(message: impl ToString) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(json!({"error":message.to_string()})),
    )
        .into_response()
}
fn asset(file: Value, download: bool, snapshot: Option<&str>) -> Result<Response> {
    let bytes = STANDARD
        .decode(file["bytes"].as_str().ok_or("Missing file bytes")?)
        .map_err(|e| e.to_string())?;
    let path = file["path"]
        .as_str()
        .or_else(|| file["name"].as_str())
        .unwrap_or("download");
    let name = path.rsplit('/').next().unwrap_or("download");
    let name = if let Some(side) = snapshot {
        format!("{side}-{name}")
    } else {
        name.into()
    };
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    let ty = if snapshot.is_some() {
        "application/octet-stream"
    } else {
        match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "svg" => "image/svg+xml",
            _ => "text/plain; charset=utf-8",
        }
    };
    let encoded = url::form_urlencoded::byte_serialize(name.as_bytes())
        .collect::<String>()
        .replace('+', "%20");
    Response::builder()
        .header(header::CONTENT_TYPE, ty)
        .header(
            header::CONTENT_DISPOSITION,
            format!(
                "{}; filename*=UTF-8''{}",
                if download { "attachment" } else { "inline" },
                encoded
            ),
        )
        .header("Content-Security-Policy", "default-src 'none'; sandbox")
        .header("X-Content-Type-Options", "nosniff")
        .header("Cache-Control", "no-store")
        .body(Body::from(bytes))
        .map_err(|e| e.to_string())
}
pub fn route(
    store: &Store,
    method: &str,
    path: &str,
    query: &str,
    body: &[u8],
) -> Result<Response> {
    let mut q = json!({});
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        let key = k.into_owned();
        q[&key] = if ["offset", "diffOffset", "limit"].contains(&key.as_str()) {
            json!(v.parse::<u64>().map_err(|_| format!("Invalid {key}"))?)
        } else {
            json!(v)
        };
    }
    if method == "GET" {
        return match path {
            "/api/workspace" => {
                let mut state = store.read()?;
                for key in ["tasks", "settings", "sync"] {
                    state.as_object_mut().unwrap().remove(key);
                }
                array_mut(&mut state, "lectures")?.retain(|l| l["hiddenFromUi"] != true);
                for j in array_mut(&mut state, "jobs")? {
                    let context = array(j, "context").to_vec();
                    let used = array(&j["result"], "citations");
                    j["context"] = json!(
                        context
                            .iter()
                            .filter(|c| used.contains(&c["id"]))
                            .collect::<Vec<_>>()
                    );
                    j["evidenceCount"] = json!(context.len());
                }
                state["policy"] = json!({"assignmentDrafts":true,"submissions":false,"downloads":true,"execution":true});
                Ok(json_response(state))
            }
            "/api/context" => Ok(json_response(crate::context::read(store, q)?)),
            "/api/course-files" | "/api/assignments" => {
                if path == "/api/course-files" {
                    q.as_object_mut().unwrap().remove("assignmentId");
                }
                if path == "/api/assignments" && q.get("assignmentId").is_none() {
                    return Ok(json_response(dispatch(store, "list_assignments", q)?));
                }
                let mode = text(&q, "mode");
                let tool = match mode.as_str() {
                    "directory" => "list_assignment_directory",
                    "revision" => "assignment_path_revision",
                    "probe" => "probe_assignment_files",
                    "asset" => "assignment_bytes",
                    _ => {
                        if q.get("path").is_some() {
                            "read_assignment_file"
                        } else {
                            "list_assignment_files"
                        }
                    }
                };
                let result = dispatch(store, tool, q.clone())?;
                if mode == "asset" {
                    asset(result, q["download"] == "1", None)
                } else {
                    Ok(json_response(result))
                }
            }
            "/api/assignment-timeline" => {
                let mode = text(&q, "mode");
                let is_asset = mode == "asset" || q["asset"] == "1";
                let tool = if is_asset {
                    "assignment_step_bytes"
                } else {
                    match mode.as_str() {
                        "parts" => "get_assignment_parts",
                        "changes" => "get_assignment_part_changes",
                        "directory" | "file" => "read_assignment_snapshot",
                        _ => "get_assignment_timeline",
                    }
                };
                let result = dispatch(store, tool, q.clone())?;
                if is_asset {
                    asset(result, q["download"] == "1", None)
                } else {
                    Ok(json_response(result))
                }
            }
            "/api/file-history" => {
                let side = text(&q, "snapshot");
                if ["before", "after"].contains(&side.as_str()) {
                    q["side"] = json!(side);
                    asset(
                        dispatch(store, "download_file_revision", q)?,
                        true,
                        Some(&side),
                    )
                } else {
                    Ok(json_response(dispatch(store, "get_file_history", q)?))
                }
            }
            "/api/course-commands" => Ok(json_response(dispatch(
                store,
                if q.get("runId").is_some() {
                    "read_course_command"
                } else {
                    "list_course_commands"
                },
                q,
            )?)),
            _ => Ok((
                StatusCode::NOT_FOUND,
                axum::Json(json!({"error":"Unknown API route"})),
            )
                .into_response()),
        };
    }
    if method != "POST" {
        return Ok((
            StatusCode::METHOD_NOT_ALLOWED,
            axum::Json(json!({"error":"Method not allowed"})),
        )
            .into_response());
    }
    let v: Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    if path == "/api/file-explorer" {
        return Ok(json_response(dispatch(store, "open_file_explorer", v)?));
    }
    let action = arg(&v, "action")?;
    let mut data = v.get("data").cloned().ok_or("Missing action data")?;
    if !data.is_object() {
        return Err("Expected action data object".into());
    }
    if path == "/api/action" {
        let tool = match action {
            "semester.create" => "create_semester",
            "course.create" => "create_course",
            "course.delete" => "course.delete",
            "course.update" => "update_course",
            "lecture.import" => "lecture.import",
            "guide.delete" => "delete_lecture_guide",
            "lecture.hide" => "lecture.hide",
            "capture.add" => "save_lecture_capture",
            "lecture.review" => "lecture.review",
            "job.create" => "job.create",
            "job.retry" => "job.retry",
            "search" => "search_course",
            "source.import" => "save_course_source",
            "concept.toggle" => "concept.toggle",
            "demo" => "demo",
            _ => return Err("Unknown action".into()),
        };
        return Ok(json_response(json!({"result":dispatch(store,tool,data)?})));
    }
    let tool = match path {
        "/api/assignments" | "/api/course-files" => {
            data["_actor"] = json!("user");
            if path == "/api/course-files" {
                data.as_object_mut().unwrap().remove("assignmentId");
            } else {
                valid_id(arg(&data, "assignmentId")?)?;
            }
            match action {
                "reference" => "reference_course_path",
                "delete-assignment" => "delete_assignment",
                "create" => "create_assignment_file",
                "write" => "write_assignment_file",
                "edit" => "edit_assignment_file",
                "mkdir" => "create_assignment_directory",
                "delete" => "delete_assignment_path",
                "move" => "move_assignment_path",
                "reorder" => "reorder_assignment_files",
                "learn" => "record_assignment_learning",
                _ => return Err("Unknown assignment operation".into()),
            }
        }
        "/api/course-commands" => match action {
            "start" => "run_course_command",
            "stop" => "stop_course_command",
            _ => return Err("Unknown command operation".into()),
        },
        _ => {
            return Ok((
                StatusCode::NOT_FOUND,
                axum::Json(json!({"error":"Unknown API route"})),
            )
                .into_response());
        }
    };
    Ok(json_response(dispatch(store, tool, data)?))
}
