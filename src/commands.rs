//! Detached Rust command workers retain their output and before/after course history.
use crate::{
    files,
    store::{Result, Store, arg, id, now, valid_id},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn run_path(course: &str, run: &str) -> String {
    format!("courses/{course}/.runtime/commands/{run}")
}
fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let _ = command;
}
fn detached(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00000200);
    }
    let _ = command;
}
fn env(command: &mut Command, store: &Store, course: &str, assignment: Option<&str>) -> Result<()> {
    command.env_clear().env("NODE_ENV", "development");
    for key in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "TMP",
        "TEMP",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "SSH_AUTH_SOCK",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env(
            "COURSE_FILES_ROOT",
            store.path(&format!("courses/{course}/files"))?,
        )
        .env("COURSE_ROOT", store.path(&format!("courses/{course}"))?)
        .env("COURSE_CAPTAIN_WORKSPACE", &store.root);
    if let Some(a) = assignment {
        command.env(
            "ASSIGNMENT_ROOT",
            store.path(&format!("courses/{course}/files/assignments/{a}"))?,
        );
    }
    Ok(())
}
fn timeout(args: &Value, default: u64) -> Result<u64> {
    match args.get("timeoutSeconds") {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|v| (1..=3600).contains(v))
            .ok_or_else(|| "timeoutSeconds must be between 1 and 3600".into()),
    }
}
fn read_metadata(store: &Store, course: &str, run_id: &str) -> Result<Value> {
    valid_id(course)?;
    valid_id(run_id)?;
    let root = run_path(course, run_id);
    let mut run = files::read_json(store, &format!("{root}/run.json"), Value::Null)?;
    if run["id"] != run_id || run["courseId"] != course {
        return Err("Command does not belong to this course".into());
    }
    if matches!(s(&run, "status"), "queued" | "running") {
        let owner = files::read_json(store, &format!("{root}/owner.json"), json!({}))?;
        if let Some(pid) = owner["pid"].as_u64()
            && !alive(pid as u32)
        {
            run["status"] = json!("interrupted");
            run["error"] =
                json!("The command worker is no longer running. Inspect outputs before retrying.");
        }
    }
    Ok(run)
}
fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(windows)]
    {
        let mut command = Command::new("tasklist");
        hidden(&mut command);
        command
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")))
    }
}
fn read_run(store: &Store, course: &str, args: &Value) -> Result<Value> {
    let run_id = arg(args, "runId")?;
    let mut run = read_metadata(store, course, run_id)?;
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;
    let limit = args["limit"].as_u64().unwrap_or(12000) as usize;
    if !(1..=50000).contains(&limit) {
        return Err("limit must be between 1 and 50000".into());
    }
    let path = store.path(&format!("{}/output.log", run_path(course, run_id)))?;
    let text = match fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    // API offsets use UTF-16 code units, matching the established browser contract.
    let units: Vec<_> = text.encode_utf16().collect();
    let end = (offset.saturating_add(limit)).min(units.len());
    let start = offset.min(units.len());
    run["output"] = json!(String::from_utf16_lossy(&units[start..end]));
    run["offset"] = json!(offset);
    run["nextOffset"] = json!(end);
    run["totalCharacters"] = json!(units.len());
    Ok(run)
}
fn start(store: &Store, args: &Value, invocation: Option<Value>) -> Result<Value> {
    let course = arg(args, "courseId")?;
    let assignment = args["assignmentId"].as_str();
    let state = store.read()?;
    files::require_scope(&state, course, assignment)?;
    let command = arg(args, "command")?.trim();
    let purpose = arg(args, "purpose")?.trim();
    if command.is_empty() || command.len() > 12000 || purpose.len() < 5 || purpose.len() > 2000 {
        return Err(
            "Command requires a meaningful purpose and 1–12000 characters of command text".into(),
        );
    }
    let seconds = timeout(args, 300)?;
    let location = args["location"].as_str().unwrap_or("files");
    if !matches!(location, "files" | "assignment") {
        return Err("Invalid command location".into());
    }
    if location == "assignment" && assignment.is_none() {
        return Err("An assignment ID is required for this working directory".into());
    }
    store.mutate(|state| files::ensure_shared(store, state, course))?;
    let files_root = store.path(&format!("courses/{course}/files"))?;
    let assignment_root = assignment
        .map(|a| store.path(&format!("courses/{course}/files/assignments/{a}")))
        .transpose()?;
    let root = if location == "assignment" {
        assignment_root.as_ref().unwrap()
    } else {
        &files_root
    };
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let relative = args["cwd"].as_str().unwrap_or(".");
    let cwd = if relative == "." {
        root.clone()
    } else {
        files::valid_path(relative)?;
        if location == "assignment" {
            let r = files::resolve(store, course, assignment, relative, false)?;
            PathBuf::from(arg(&r, "absolutePath")?)
        } else {
            store.path(&format!("courses/{course}/files/{relative}"))?
        }
    };
    if !cwd.is_dir() {
        return Err("Working directory is not a directory".into());
    }
    let run_id = id();
    let base = run_path(course, &run_id);
    let summary = format!("courses/{course}/memory/files/runs/{run_id}.md");
    let mut run = json!({"id":run_id,"courseId":course,"command":command,"purpose":purpose,"cwd":cwd,"status":"queued","createdAt":now(),"summaryPath":summary});
    if let Some(a) = assignment {
        run["assignmentId"] = json!(a)
    }
    let roots_before = if let Some(root) = &assignment_root {
        if root.exists() {
            fs::read_dir(root)
                .map_err(|e| e.to_string())?
                .map(|e| e.map(|e| e.file_name().to_string_lossy().to_string()))
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?
        } else {
            vec![]
        }
    } else {
        vec![]
    };
    let mut request = run.clone();
    request["timeoutSeconds"] = json!(seconds);
    request["invocation"] = invocation.unwrap_or(Value::Null);
    request["assignmentRoot"] = json!(assignment_root);
    request["assignmentRootsBefore"] = json!(roots_before);
    files::write_json(store, &format!("{base}/run.json"), &run)?;
    let request_path = format!("{base}/request.json");
    files::write_json(store, &request_path, &request)?;
    let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    child
        .arg("command-worker")
        .arg("--workspace")
        .arg(&store.root)
        .arg("--request")
        .arg(&request_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detached(&mut child);
    env(&mut child, store, course, assignment)?;
    match child.spawn() {
        Ok(mut child) => {
            files::write_json(
                store,
                &format!("{base}/owner.json"),
                &json!({"pid":child.id()}),
            )?;
            thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => {
            run["status"] = json!("failed");
            run["error"] = json!(e.to_string());
            files::write_json(store, &format!("{base}/run.json"), &run)?;
            return Err(e.to_string());
        }
    }
    run["instructions"] = json!(
        "Poll read_course_command until terminal; inspect exit code and output. Use stop_course_command to cancel. Commands run on this host, not in a security sandbox. Never submit coursework."
    );
    Ok(run)
}
fn repository_url(repository: &str) -> bool {
    if repository.is_empty()
        || repository.len() > 2000
        || repository
            .chars()
            .any(|c| c == '\0' || c == '\r' || c == '\n')
    {
        return false;
    }
    if Path::new(repository).is_absolute() {
        return true;
    }
    if let Some(rest) = repository.strip_prefix("https://") {
        let host = rest.split('/').next().unwrap_or("");
        return rest.contains('/')
            && !host.is_empty()
            && !host.contains('@')
            && !host.chars().any(char::is_whitespace);
    }
    if let Some(rest) = repository.strip_prefix("ssh://") {
        let host = rest.split('/').next().unwrap_or("");
        return rest.contains('/')
            && !host.is_empty()
            && !host.chars().any(char::is_whitespace)
            && (!host.contains('@') || host.starts_with("git@") && !host[4..].contains('@'));
    }
    if let Some(rest) = repository.strip_prefix("git@") {
        return rest.split_once(':').is_some_and(|(host, p)| {
            !host.is_empty() && !host.chars().any(char::is_whitespace) && !p.is_empty()
        });
    }
    false
}
fn clone_repo(store: &Store, args: &Value) -> Result<Value> {
    let course = arg(args, "courseId")?;
    let repository = arg(args, "repository")?;
    if !repository_url(repository) {
        return Err("Use an HTTPS/SSH repository URL without embedded credentials, or a local repository path".into());
    }
    let destination = arg(args, "destination")?;
    files::valid_path(destination)?;
    let target = store.path(&format!("courses/{course}/files/{destination}"))?;
    if target.exists() {
        return Err("Destination already exists. Reuse the existing repository through course commands; cloning never overwrites files.".into());
    }
    fs::create_dir_all(target.parent().ok_or("Invalid destination")?).map_err(|e| e.to_string())?;
    let mut argv = vec!["clone".to_string(), "--progress".into()];
    if let Some(branch) = args["branch"].as_str() {
        if branch.is_empty()
            || branch.len() > 200
            || !branch.as_bytes()[0].is_ascii_alphanumeric()
            || !branch
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
        {
            return Err("Invalid branch name".into());
        }
        argv.extend(["--branch".into(), branch.into()]);
    }
    argv.extend([
        "--".into(),
        repository.into(),
        target.to_string_lossy().to_string(),
    ]);
    start(
        store,
        &json!({"courseId":course,"command":format!("git clone {} {}",json!(repository),json!(destination)),"purpose":format!("Clone reusable course repository {repository} into {destination}"),"timeoutSeconds":timeout(args,600)?}),
        Some(json!({"executable":"git","args":argv,"repository":repository})),
    )
}
pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "run_course_command"
            | "read_course_command"
            | "stop_course_command"
            | "list_course_commands"
            | "clone_course_repository"
    )
}
pub fn call(store: &Store, name: &str, args: Value) -> Result<Value> {
    if !handles(name) {
        return Err(format!("Unknown tool: {name}"));
    }
    let course = arg(&args, "courseId")?;
    files::require_scope(&store.read()?, course, args["assignmentId"].as_str())?;
    match name {
        "run_course_command" => start(store, &args, None),
        "clone_course_repository" => clone_repo(store, &args),
        "read_course_command" => read_run(store, course, &args),
        "stop_course_command" => {
            let run = read_run(store, course, &args)?;
            if !matches!(s(&run, "status"), "queued" | "running") {
                return Ok(run);
            }
            store.write(
                &format!("{}/cancel", run_path(course, arg(&args, "runId")?)),
                b"cancel requested",
            )?;
            Ok(json!({"id":run["id"],"status":run["status"],"cancellationRequested":true}))
        }
        "list_course_commands" => {
            let root = store.path(&format!("courses/{course}/.runtime/commands"))?;
            if !root.exists() {
                return Ok(json!([]));
            }
            let mut runs = vec![];
            for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().to_string();
                if valid_id(&name).is_ok() {
                    runs.push(read_metadata(store, course, &name)?);
                }
            }
            runs.sort_by(|a, b| s(b, "createdAt").cmp(s(a, "createdAt")));
            runs.truncate(30);
            Ok(json!(runs))
        }
        _ => Err(format!("Unknown tool: {name}")),
    }
}
struct Output {
    file: fs::File,
    bytes: usize,
    truncated: bool,
}
fn drain(
    mut reader: impl Read + Send + 'static,
    out: Arc<Mutex<Output>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            let n = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            let mut out = out.lock().unwrap();
            let kept = n.min(2_000_000usize.saturating_sub(out.bytes));
            if kept < n {
                out.truncated = true
            }
            if out.file.write_all(&buffer[..kept]).is_ok() {
                out.bytes += kept;
            }
        }
    })
}
fn kill_tree(child: &mut Child, force: bool) {
    #[cfg(unix)]
    {
        let _ = Command::new("/bin/kill")
            .args([
                if force { "-KILL" } else { "-TERM" },
                "--",
                &format!("-{}", child.id()),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let mut command = Command::new("taskkill");
        command.args(["/PID", &child.id().to_string(), "/T"]);
        if force {
            command.arg("/F");
        }
        hidden(&mut command);
        let _ = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    if force {
        let _ = child.kill();
    }
}
/// Invoked by the same installed Rust binary, independent of the HTTP/MCP parent.
pub fn worker(store: &Store, relative: &str) -> Result<()> {
    let request = files::read_json(store, relative, Value::Null)?;
    let course = arg(&request, "courseId")?;
    let run_id = arg(&request, "id")?;
    valid_id(course)?;
    valid_id(run_id)?;
    let base = run_path(course, run_id);
    if relative != format!("{base}/request.json") {
        return Err("Invalid command request path".into());
    }
    let assignment = request["assignmentId"].as_str();
    files::require_scope(&store.read()?, course, assignment)?;
    let cwd = PathBuf::from(arg(&request, "cwd")?);
    let files_root = store.path(&format!("courses/{course}/files"))?;
    if !cwd.starts_with(&files_root) {
        return Err("Command directory is outside course files".into());
    }
    let relative_cwd = cwd
        .strip_prefix(&store.root)
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    store.path(&relative_cwd)?;
    let mut run = files::read_json(store, &format!("{base}/run.json"), Value::Null)?;
    run["status"] = json!("running");
    run["startedAt"] = json!(now());
    run["pid"] = json!(std::process::id());
    files::write_json(
        store,
        &format!("{base}/owner.json"),
        &json!({"pid":std::process::id()}),
    )?;
    files::write_json(store, &format!("{base}/run.json"), &run)?;
    let mut before = json!({"actor":"external","explanation":"State observed before a tracked command. Earlier changes and their context are unknown.","runId":run_id,"tool":"command_before"});
    if let Some(a) = assignment {
        before["assignmentId"] = json!(a)
    }
    match files::checkpoint(store, course, &before) {
        Ok(check) => {
            run["historyBefore"] = check;
            files::write_json(store, &format!("{base}/run.json"), &run)?
        }
        Err(e) => {
            run["status"] = json!("failed");
            run["finishedAt"] = json!(now());
            run["error"] = json!(format!("Could not record pre-command history: {e}"));
            files::write_json(store, &format!("{base}/run.json"), &run)?;
            return Err(e);
        }
    }
    let invocation = &request["invocation"];
    let mut command = if !invocation.is_null() {
        let mut c = Command::new(arg(invocation, "executable")?);
        for a in invocation["args"]
            .as_array()
            .ok_or("Invalid invocation arguments")?
        {
            c.arg(a.as_str().ok_or("Invalid invocation argument")?);
        }
        c
    } else {
        #[cfg(windows)]
        {
            let mut c = Command::new("powershell.exe");
            c.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                arg(&request, "command")?,
            ]);
            c
        }
        #[cfg(unix)]
        {
            let mut c = Command::new("/bin/bash");
            c.args(["--noprofile", "--norc", "-c", arg(&request, "command")?]);
            c
        }
    };
    command
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hidden(&mut command);
    detached(&mut command);
    env(&mut command, store, course, assignment)?;
    let logfile = store.path(&format!("{base}/output.log"))?;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let output = Arc::new(Mutex::new(Output {
        file: opts.open(logfile).map_err(|e| e.to_string())?,
        bytes: 0,
        truncated: false,
    }));
    let mut error = None;
    let mut reason = None;
    let mut exit = None;
    let mut signal = None;
    match command.spawn() {
        Err(e) => error = Some(e.to_string()),
        Ok(mut child) => {
            let stdout = drain(
                child.stdout.take().ok_or("Missing command stdout")?,
                output.clone(),
            );
            let stderr = drain(
                child.stderr.take().ok_or("Missing command stderr")?,
                output.clone(),
            );
            let start = Instant::now();
            let deadline = Duration::from_secs(timeout(&request, 300)?);
            let mut stopped = None;
            let cancel = store.path(&format!("{base}/cancel"))?;
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        exit = status.code();
                        #[cfg(unix)]
                        {
                            use std::os::unix::process::ExitStatusExt;
                            signal = status.signal().map(|s| format!("SIG{s}"));
                        }
                        break;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        error = Some(e.to_string());
                        break;
                    }
                }
                if reason.is_none() {
                    if cancel.exists() {
                        reason = Some("cancelled")
                    } else if start.elapsed() >= deadline {
                        reason = Some("timed_out")
                    }
                    if reason.is_some() {
                        kill_tree(&mut child, false);
                        stopped = Some(Instant::now());
                    }
                }
                if stopped.is_some_and(|t| t.elapsed() >= Duration::from_millis(1500)) {
                    kill_tree(&mut child, true);
                }
                run["heartbeat"] = json!(now());
                run["truncated"] = json!(output.lock().unwrap().truncated);
                files::write_json(store, &format!("{base}/run.json"), &run)?;
                thread::sleep(Duration::from_millis(200));
            }
            kill_tree(&mut child, true);
            let _ = child.wait();
            let _ = stdout.join();
            let _ = stderr.join();
        }
    }
    if let Some(a) = assignment {
        let assignment_root = store.path(&format!("courses/{course}/files/assignments/{a}"))?;
        if assignment_root.exists() {
            let result = store.mutate(|_| {
                let old = request["assignmentRootsBefore"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                for entry in fs::read_dir(&assignment_root).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let name = entry.file_name().to_string_lossy().to_string();
                    let kind = entry.file_type().map_err(|e| e.to_string())?;
                    if !old.contains(&json!(name))
                        && !files::excluded(&name)
                        && (kind.is_file() || kind.is_dir())
                    {
                        files::show_created(store, course, Some(a), &name)?;
                    }
                }
                Ok(())
            });
            if let Err(e) = result {
                error = Some(format!(
                    "Command finished but assignment references failed: {e}"
                ));
            }
        }
    }
    let final_status = reason.unwrap_or(if exit == Some(0) && error.is_none() {
        "completed"
    } else {
        "failed"
    });
    let mut after = json!({"actor":"terminal","explanation":run["purpose"],"command":run["command"],"runId":run_id,"tool":"command_after","context":format!("Observed net changes across this command (exit {}, {final_status}). Concurrent external changes may also be present; intermediate writes were not captured.",exit.map(|v|v.to_string()).unwrap_or_else(||"none".into()))});
    if let Some(a) = assignment {
        after["assignmentId"] = json!(a)
    }
    match files::checkpoint(store, course, &after) {
        Ok(v) => run["historyAfter"] = v,
        Err(e) => {
            error = Some(format!(
                "Command finished but its history checkpoint failed: {e}"
            ))
        }
    }
    run["status"] = json!(final_status);
    run["exitCode"] = json!(exit);
    run["signal"] = json!(signal);
    run["finishedAt"] = json!(now());
    run["truncated"] = json!(output.lock().unwrap().truncated);
    if let Some(e) = error {
        run["error"] = json!(e)
    }
    let output = fs::read(store.path(&format!("{base}/output.log"))?).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&output);
    let tail = text
        .chars()
        .rev()
        .take(12000)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let summary = format!(
        "# Command: {}\n\nObserved local command execution, not instructor evidence.\n\n- Started: {}\n- Working directory: {}\n- Status: {}\n- Exit code: {}\n- Signal: {}\n- Assignment: {}\n\n## Command\n\n{}\n\n## Output (last 12000 characters{})\n\n{tail}\n\n{}\n\nA successful exit only verifies what this command actually checked. Record reasoning and remaining gaps separately.\n",
        s(&run, "purpose"),
        s(&run, "startedAt"),
        s(&run, "cwd"),
        s(&run, "status"),
        exit.map(|v| v.to_string()).unwrap_or_else(|| "none".into()),
        signal.as_deref().unwrap_or("none"),
        assignment.unwrap_or("Shared course files"),
        s(&run, "command"),
        if run["truncated"] == true {
            "; log reached 2 MB limit"
        } else {
            ""
        },
        s(&run, "error")
    );
    let expected_summary = format!("courses/{course}/memory/files/runs/{run_id}.md");
    store.write(&expected_summary, summary.as_bytes())?;
    files::write_json(store, &format!("{base}/run.json"), &run)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repository_arguments_reject_credentials_and_shell_protocols() {
        for url in [
            "https://user:secret@example.com/repo",
            "https://git@example.com/repo",
            "ext::sh -c bad",
            "http://example.com/repo",
            "git@bad\nname:repo",
        ] {
            assert!(!repository_url(url), "{url}")
        }
        for url in [
            "https://example.com/course/repo.git",
            "git@example.com:course/repo.git",
            "ssh://git@example.com/course/repo.git",
        ] {
            assert!(repository_url(url), "{url}")
        }
    }
    #[cfg(unix)]
    #[test]
    fn worker_records_failed_output_and_generated_assignment_files() {
        let root = std::env::temp_dir().join(format!("cruise-command-{}", id()));
        let store = Store { root: root.clone() };
        store.mutate(|v|{v["courses"]=json!([{"id":"course"}]);v["assignments"]=json!([{"id":"assignment","courseId":"course","title":"Test","createdAt":now()}]);Ok(())}).unwrap();
        let cwd = store
            .path("courses/course/files/assignments/assignment")
            .unwrap();
        fs::create_dir_all(&cwd).unwrap();
        let run = json!({"id":"run","courseId":"course","assignmentId":"assignment","command":"printf generated > result.txt; printf command-output; exit 7","purpose":"Generate a test fixture and deliberately fail","cwd":cwd,"status":"queued","createdAt":now(),"timeoutSeconds":5,"assignmentRootsBefore":[]});
        let request = "courses/course/.runtime/commands/run/request.json";
        files::write_json(&store, request, &run).unwrap();
        files::write_json(
            &store,
            "courses/course/.runtime/commands/run/run.json",
            &run,
        )
        .unwrap();
        worker(&store, request).unwrap();
        let result = call(
            &store,
            "read_course_command",
            json!({"courseId":"course","runId":"run"}),
        )
        .unwrap();
        assert_eq!(result["status"], "failed");
        assert_eq!(result["exitCode"], 7);
        assert_eq!(result["output"], "command-output");
        assert_eq!(
            files::call(
                &store,
                "read_assignment_file",
                json!({"courseId":"course","assignmentId":"assignment","path":"result.txt"})
            )
            .unwrap()["content"],
            "generated"
        );
        let history = files::steps(&store, "course", "assignment").unwrap();
        assert_eq!(history.last().unwrap()["runId"], "run");
        assert!(s(history.last().unwrap(), "markdown").contains("exit 7"));
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn worker_times_out_and_keeps_net_changes() {
        let root = std::env::temp_dir().join(format!("cruise-timeout-{}", id()));
        let store = Store { root: root.clone() };
        store
            .mutate(|v| {
                v["courses"] = json!([{"id":"course"}]);
                Ok(())
            })
            .unwrap();
        let cwd = store.path("courses/course/files").unwrap();
        fs::create_dir_all(&cwd).unwrap();
        let run = json!({"id":"run","courseId":"course","command":"printf before > result.txt; sleep 30","purpose":"Verify worker timeout cleanup","cwd":cwd,"status":"queued","createdAt":now(),"timeoutSeconds":1});
        let request = "courses/course/.runtime/commands/run/request.json";
        files::write_json(&store, request, &run).unwrap();
        files::write_json(
            &store,
            "courses/course/.runtime/commands/run/run.json",
            &run,
        )
        .unwrap();
        worker(&store, request).unwrap();
        let result = call(
            &store,
            "read_course_command",
            json!({"courseId":"course","runId":"run"}),
        )
        .unwrap();
        assert_eq!(result["status"], "timed_out");
        assert!(result["historyAfter"]["changed"].as_u64().unwrap() > 0);
        fs::remove_dir_all(root).unwrap();
    }
}
