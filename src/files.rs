//! Course files, shared assignment views, immutable history, and teaching timelines.
use crate::store::{Result, Store, arg, id, now, require_course, valid_id};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 10_000_000;
pub fn hash(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}
fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn arr(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}
fn integer(v: &Value, key: &str, default: usize, max: usize) -> Result<usize> {
    match v.get(key).filter(|x| !x.is_null()) {
        None => Ok(default),
        Some(x) => x
            .as_u64()
            .filter(|x| *x <= max as u64)
            .map(|x| x as usize)
            .ok_or_else(|| format!("Invalid {key}")),
    }
}
pub fn read_json(store: &Store, path: &str, fallback: Value) -> Result<Value> {
    match fs::read(store.path(path)?) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("Invalid {path}: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(fallback),
        Err(e) => Err(e.to_string()),
    }
}
pub fn write_json(store: &Store, path: &str, value: &Value) -> Result<()> {
    store.write(
        path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
}
pub fn valid_path(path: &str) -> Result<()> {
    let parts: Vec<_> = path.split('/').collect();
    if path.is_empty()
        || path.len() > 400
        || parts.len() > 20
        || parts.iter().any(|p| {
            let stem = p.split('.').next().unwrap_or("").to_ascii_lowercase();
            p.is_empty()
                || *p == "."
                || *p == ".."
                || p.ends_with(['.', ' '])
                || p.chars()
                    .any(|c| c.is_control() || "\\:<>\"|?*".contains(c))
                || matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
                || (stem.len() == 4
                    && (stem.starts_with("com") || stem.starts_with("lpt"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        })
    {
        return Err("Use a relative assignment path without traversal or reserved names".into());
    }
    Ok(())
}
pub fn excluded(path: &str) -> bool {
    path.split('/').any(|p| {
        let lower = p.to_ascii_lowercase();
        matches!(
            p,
            ".git"
                | ".hg"
                | ".svn"
                | "node_modules"
                | ".venv"
                | "venv"
                | "__pycache__"
                | ".next"
                | "dist"
                | "build"
                | "target"
                | "coverage"
                | ".cache"
                | ".runtime"
                | ".secrets"
                | ".ssh"
                | ".npmrc"
                | ".pypirc"
                | ".netrc"
                | ".git-credentials"
                | "id_rsa"
                | "id_ed25519"
        ) || p == ".env"
            || p.starts_with(".env.")
            || matches!(lower.as_str(), "auth.json" | "storage-state.json")
            || [".pem", ".key", ".p12", ".pfx"]
                .iter()
                .any(|e| lower.ends_with(e))
    })
}
pub fn assignments(state: &Value, course: &str) -> Result<Vec<Value>> {
    require_course(state, course)?;
    let mut out: Vec<Value> = arr(&state["assignments"])
        .into_iter()
        .filter(|a| s(a, "courseId") == course)
        .collect();
    for job in arr(&state["jobs"]) {
        if s(&job, "courseId") == course
            && s(&job, "kind") == "assignment"
            && !out.iter().any(|a| a["id"] == job["id"])
        {
            out.push(json!({"id":job["id"],"courseId":course,"title":s(&job,"prompt").chars().take(200).collect::<String>(),"description":job["prompt"],"createdAt":job["createdAt"],"legacyJobId":job["id"]}));
        }
    }
    out.retain(|a| !arr(&state["hiddenAssignments"]).contains(&a["id"]));
    out.sort_by(|a, b| s(b, "createdAt").cmp(s(a, "createdAt")));
    Ok(out)
}
pub fn require_scope(state: &Value, course: &str, assignment: Option<&str>) -> Result<()> {
    valid_id(course)?;
    require_course(state, course)?;
    if let Some(a) = assignment {
        valid_id(a)?;
        if !assignments(state, course)?.iter().any(|v| s(v, "id") == a) {
            return Err("Assignment not found in this course".into());
        }
    }
    Ok(())
}
fn base(course: &str, assignment: Option<&str>) -> String {
    assignment
        .map(|a| format!("courses/{course}/files/assignments/{a}"))
        .unwrap_or_else(|| format!("courses/{course}/files"))
}
fn manifest(course: &str, a: &str, kind: &str) -> String {
    format!("courses/{course}/agent/assignments/{a}-{kind}.json")
}
fn order_path(course: &str, a: Option<&str>) -> String {
    a.map(|a| manifest(course, a, "files"))
        .unwrap_or_else(|| format!("courses/{course}/agent/course-files.json"))
}
pub fn references(store: &Store, course: &str, a: Option<&str>) -> Result<Vec<Value>> {
    let refs = match a {
        Some(a) => arr(&read_json(
            store,
            &manifest(course, a, "references"),
            json!([]),
        )?),
        None => vec![],
    };
    if refs.len() > 2000 {
        return Err("An assignment supports up to 2000 displayed roots".into());
    }
    for r in &refs {
        valid_path(arg(r, "path")?)?;
        valid_path(arg(r, "targetPath")?)?;
        if s(r, "path").contains('/') || !matches!(s(r, "type"), "file" | "directory") {
            return Err("Invalid assignment reference".into());
        }
        for p in arr(&r["excludedPaths"]) {
            valid_path(p.as_str().ok_or("Invalid excluded path")?)?;
        }
    }
    Ok(refs)
}
fn contains(root: &str, p: &str) -> bool {
    root == p || p.starts_with(&format!("{root}/"))
}
pub fn resolve(
    store: &Store,
    course: &str,
    a: Option<&str>,
    path: &str,
    allow_missing: bool,
) -> Result<Value> {
    valid_path(path)?;
    let refs = references(store, course, a)?;
    let reference = refs.iter().find(|r| contains(s(r, "path"), path));
    if let Some(r) = reference {
        if s(r, "type") == "file" && s(r, "path") != path {
            return Err("Cannot create children inside a file reference".into());
        }
        if !allow_missing
            && arr(&r["excludedPaths"])
                .iter()
                .any(|p| contains(p.as_str().unwrap_or(""), path))
        {
            return Err(
                "Path is not displayed in this assignment; add it again from course Files".into(),
            );
        }
    }
    let shared = reference
        .map(|r| format!("{}{}", s(r, "targetPath"), &path[s(r, "path").len()..]))
        .unwrap_or_else(|| {
            a.map(|a| format!("assignments/{a}/{path}"))
                .unwrap_or_else(|| path.to_string())
        });
    if excluded(&shared) {
        return Err("This path is excluded from course file tools".into());
    }
    let workspace = format!("courses/{course}/files/{shared}");
    let absolute = store.path(&workspace)?;
    let mut value = json!({"workspacePath":workspace,"absolutePath":absolute});
    if let Some(r) = reference {
        if !allow_missing {
            let m = fs::metadata(
                store.path(&format!("courses/{course}/files/{}", s(r, "targetPath")))?,
            )
            .map_err(|e| e.to_string())?;
            if (s(r, "type") == "directory") != m.is_dir() {
                return Err(
                    "Shared reference target changed type; remove and recreate the reference"
                        .into(),
                );
            }
        }
        value["shared"] = json!({"path":shared,"referencePath":r["path"],"root":s(r,"path")==path});
    }
    Ok(value)
}
pub fn show_created(store: &Store, course: &str, a: Option<&str>, path: &str) -> Result<()> {
    let Some(a) = a else { return Ok(()) };
    let mut refs = references(store, course, Some(a))?;
    let root = path.split('/').next().ok_or("Invalid path")?;
    if refs.iter().any(|r| s(r, "path") == root) {
        return Ok(());
    }
    let target = format!("assignments/{a}/{root}");
    let m = fs::metadata(store.path(&format!("courses/{course}/files/{target}"))?)
        .map_err(|e| e.to_string())?;
    refs.push(json!({"path":root,"targetPath":target,"type":if m.is_dir(){"directory"}else{"file"},"createdAt":now(),"excludedPaths":[]}));
    write_json(store, &manifest(course, a, "references"), &json!(refs))
}
pub fn media_type(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "html" | "htm" => "text/html",
        "md" => "text/markdown",
        _ => "text/plain",
    }
}
fn metadata_revision(m: &fs::Metadata) -> String {
    hash(format!("{}:{:?}:{:?}", m.len(), m.modified(), m.created()))
}
fn stat_revision(m: &fs::Metadata) -> String {
    format!("stat:{}", metadata_revision(m))
}
fn file_revision(path: &Path, m: &fs::Metadata, shallow: bool) -> Result<String> {
    if shallow {
        Ok(stat_revision(m))
    } else if m.len() > MAX_BYTES {
        Ok(metadata_revision(m))
    } else {
        Ok(hash(fs::read(path).map_err(|e| e.to_string())?))
    }
}
fn modified(m: &fs::Metadata) -> String {
    m.modified()
        .map(|t| {
            chrono::DateTime::<chrono::Utc>::from(t)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        })
        .unwrap_or_default()
}
fn walk(
    store: &Store,
    course: &str,
    path: &str,
    shallow: bool,
    include: bool,
    out: &mut Vec<Value>,
) -> Result<String> {
    if !path.is_empty() && excluded(path) {
        return Ok(String::new());
    }
    let full = store.path(&format!(
        "courses/{course}/files{}",
        if path.is_empty() {
            String::new()
        } else {
            format!("/{path}")
        }
    ))?;
    let m = fs::metadata(&full).map_err(|e| e.to_string())?;
    if !m.is_file() && !m.is_dir() {
        return Err("Only regular files and directories are supported".into());
    }
    let index = out.len();
    if include {
        if out.len() >= 20000 {
            return Err("File workspace is too large to display; move generated output into an excluded build directory".into());
        }
        out.push(json!({"path":path,"name":path.rsplit('/').next(),"type":if m.is_dir(){"directory"}else{"file"},"size":m.len(),"modifiedAt":modified(&m),"revision":""}));
    }
    let revision = if m.is_file() {
        file_revision(&full, &m, shallow)?
    } else if shallow && include {
        String::new()
    } else {
        let mut children = fs::read_dir(&full)
            .map_err(|e| e.to_string())?
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        children.sort_by_key(|c| c.file_name());
        let mut revisions = vec![];
        for child in children {
            if child.file_type().map_err(|e| e.to_string())?.is_symlink() {
                continue;
            }
            let name = child.file_name().to_string_lossy().to_string();
            let next = if path.is_empty() {
                name
            } else {
                format!("{path}/{name}")
            };
            if excluded(&next) {
                continue;
            }
            valid_path(&next)?;
            revisions.push(format!(
                "{next}:{}",
                walk(store, course, &next, shallow, true, out)?
            ));
        }
        hash(revisions.join("\n"))
    };
    if include {
        out[index]["revision"] = json!(revision)
    }
    Ok(revision)
}
pub fn tree(store: &Store, course: &str, a: Option<&str>, options: &Value) -> Result<Value> {
    let refs = references(store, course, a)?;
    let directory = s(options, "directory");
    let target = s(options, "target");
    let shallow = options["shallow"].as_bool().unwrap_or(false);
    if !directory.is_empty() {
        valid_path(directory)?
    }
    if !target.is_empty() {
        valid_path(target)?
    }
    let mut entries = vec![];
    let mut contents = hash("");
    if a.is_none() {
        if store.path(&base(course, None))?.exists() {
            contents = walk(
                store,
                course,
                if !target.is_empty() {
                    target
                } else {
                    directory
                },
                shallow,
                !target.is_empty(),
                &mut entries,
            )?;
        }
    } else {
        if (!directory.is_empty() || !target.is_empty())
            && !refs.iter().any(|r| {
                contains(
                    s(r, "path"),
                    if !directory.is_empty() {
                        directory
                    } else {
                        target
                    },
                )
            })
        {
            return Err("Path is not displayed in this assignment".into());
        }
        for r in &refs {
            let alias = s(r, "path");
            let canonical = s(r, "targetPath");
            if !target.is_empty() && !contains(alias, target)
                || !directory.is_empty() && !contains(alias, directory)
            {
                continue;
            }
            if !directory.is_empty()
                && arr(&r["excludedPaths"])
                    .iter()
                    .any(|p| contains(p.as_str().unwrap_or(""), directory))
            {
                return Err("Path is not displayed in this assignment".into());
            }
            let requested = if !directory.is_empty() {
                directory
            } else if !target.is_empty() {
                target
            } else {
                alias
            };
            let source = format!("{canonical}{}", &requested[alias.len()..]);
            let mut shared = vec![];
            let full = store.path(&format!("courses/{course}/files/{canonical}"))?;
            let missing = match fs::metadata(full) {
                Ok(m) => (s(r, "type") == "directory") != m.is_dir(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
                Err(e) => return Err(e.to_string()),
            };
            if missing {
                if !directory.is_empty() || !target.is_empty() && target != alias {
                    return Err("Shared reference target not found".into());
                }
                entries.push(json!({"path":alias,"name":alias,"type":r["type"],"size":0,"modifiedAt":r["createdAt"],"revision":hash(format!("{r}:missing")),"shared":{"path":canonical,"referencePath":alias,"root":true,"missing":true}}));
                continue;
            }
            walk(
                store,
                course,
                &source,
                shallow,
                directory.is_empty(),
                &mut shared,
            )?;
            for mut entry in shared {
                let original = s(&entry, "path").to_string();
                let display = format!("{alias}{}", &original[canonical.len()..]);
                if arr(&r["excludedPaths"])
                    .iter()
                    .any(|p| contains(p.as_str().unwrap_or(""), &display))
                {
                    continue;
                }
                entry["path"] = json!(display);
                entry["name"] = json!(display.rsplit('/').next());
                entry["absolutePath"] =
                    json!(store.path(&format!("courses/{course}/files/{original}"))?);
                entry["shared"] =
                    json!({"path":original,"referencePath":alias,"root":display==alias});
                entries.push(entry);
            }
        }
    }
    let order = arr(&read_json(store, &order_path(course, a), json!([]))?);
    entries.sort_by(|x, y| {
        order
            .iter()
            .position(|p| p == &x["path"])
            .unwrap_or(usize::MAX)
            .cmp(
                &order
                    .iter()
                    .position(|p| p == &y["path"])
                    .unwrap_or(usize::MAX),
            )
            .then_with(|| s(x, "path").cmp(s(y, "path")))
    });
    let revision = hash(format!(
        "{contents}{}{}{}",
        json!(order),
        json!(refs),
        json!(
            entries
                .iter()
                .map(|e| json!([e["path"], e["revision"]]))
                .collect::<Vec<_>>()
        )
    ));
    let mut result = json!({"courseId":course,"root":base(course,a),"absoluteRoot":store.path(&base(course,a))?,"entries":entries,"revision":revision});
    if let Some(a) = a {
        result["assignmentId"] = json!(a)
    }
    Ok(result)
}
fn read_file(
    store: &Store,
    course: &str,
    a: Option<&str>,
    path: &str,
    bytes: bool,
) -> Result<Value> {
    if a.is_some()
        && !references(store, course, a)?
            .iter()
            .any(|r| contains(s(r, "path"), path))
    {
        return Err("Path is not displayed in this assignment".into());
    }
    let mut out = resolve(store, course, a, path, false)?;
    let full = PathBuf::from(arg(&out, "absolutePath")?);
    let m = fs::metadata(&full).map_err(|e| e.to_string())?;
    if !m.is_file() || m.len() > MAX_BYTES {
        return Err("Expected a regular file under 10 MB".into());
    }
    let content = fs::read(&full).map_err(|e| e.to_string())?;
    out["path"] = json!(path);
    out["revision"] = json!(hash(&content));
    out["metadataRevision"] = json!(stat_revision(&m));
    out["size"] = json!(content.len());
    out["mediaType"] = json!(media_type(path));
    if let Ok(text) = std::str::from_utf8(&content)
        && !text.contains('\0')
    {
        out["content"] = json!(text)
    }
    if bytes {
        out["bytes"] = json!(STANDARD.encode(content))
    }
    Ok(out)
}
fn check_revision(
    store: &Store,
    course: &str,
    a: Option<&str>,
    path: &str,
    revision: &str,
) -> Result<Value> {
    let listing = tree(
        store,
        course,
        a,
        &json!({"target":path,"shallow":revision.starts_with("stat:")}),
    )?;
    let entry = arr(&listing["entries"])
        .into_iter()
        .find(|e| s(e, "path") == path)
        .ok_or("Assignment path not found")?;
    if s(&entry, "revision") != revision {
        return Err("File changed since it was read. Reload before applying this change.".into());
    }
    Ok(entry)
}
fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    let m = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if m.file_type().is_symlink() {
        return Err("Symlinks are not allowed in recovery copies".into());
    }
    if destination.exists() {
        return Err("Recovery destination already exists".into());
    }
    if m.is_dir() {
        fs::create_dir_all(destination).map_err(|e| e.to_string())?;
        for child in fs::read_dir(source).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            copy_tree(&child.path(), &destination.join(child.file_name()))?;
        }
    } else if m.is_file() {
        fs::create_dir_all(destination.parent().ok_or("Invalid recovery path")?)
            .map_err(|e| e.to_string())?;
        fs::copy(source, destination).map_err(|e| e.to_string())?;
    } else {
        return Err("Only regular files and directories are supported".into());
    }
    Ok(())
}
fn backup(store: &Store, course: &str, a: Option<&str>, path: &str) -> Result<String> {
    let source = resolve(store, course, a, path, false)?;
    let to = format!(".trash/assignments/{}/{course}/{path}", id());
    copy_tree(Path::new(arg(&source, "absolutePath")?), &store.path(&to)?)?;
    Ok(to)
}
fn context(args: &Value, tool: &str) -> Value {
    let mut c = json!({"actor":args["_actor"].as_str().unwrap_or("agent"),"explanation":args["explanation"].as_str().unwrap_or("Update the assignment workspace."),"tool":tool});
    for key in ["context", "assignmentId", "command", "runId"] {
        if !args[key].is_null() {
            c[key] = args[key].clone()
        }
    }
    c
}
fn explanation(args: &Value) -> Result<()> {
    let e = arg(args, "explanation")?.trim();
    if e.len() < 10 || e.len() > 6000 {
        return Err("explanation must contain 10 to 6000 characters".into());
    }
    if s(args, "context").len() > 8000 {
        return Err("context is too long".into());
    }
    Ok(())
}
fn append_memory(
    store: &Store,
    course: &str,
    action: &str,
    args: &Value,
    paths: &[String],
) -> Result<()> {
    let path = format!("courses/{course}/memory/files/activity.md");
    let full = store.path(&path)?;
    let prior = match fs::read_to_string(full) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            "# File workspace activity\n\nGenerated work history, not instructor evidence.\n".into()
        }
        Err(e) => return Err(e.to_string()),
    };
    store.write(
        &path,
        format!(
            "{prior}\n## {} — {action}\n\n{}\n\nFiles: {}\n",
            now(),
            s(args, "explanation"),
            paths.join(", ")
        )
        .as_bytes(),
    )
}

fn history_root(course: &str) -> String {
    format!("courses/{course}/history")
}
fn history_path(course: &str, key: &str) -> String {
    format!("{}/files/{key}.json", history_root(course))
}
fn read_index(store: &Store, course: &str) -> Result<Value> {
    let mut index = read_json(
        store,
        &format!("{}/index.json", history_root(course)),
        json!({"paths":{},"archived":{},"ids":[]}),
    )?;
    let mut ids = arr(&index["ids"]);
    for key in ["paths", "archived"] {
        if let Some(m) = index[key].as_object() {
            for value in m.values() {
                if !ids.contains(value) {
                    ids.push(value.clone())
                }
            }
        }
    }
    index["ids"] = json!(ids);
    Ok(index)
}
fn save_index(store: &Store, course: &str, index: &mut Value) -> Result<()> {
    let mut ids = arr(&index["ids"]);
    for key in ["paths", "archived"] {
        if let Some(m) = index[key].as_object() {
            for value in m.values() {
                if !ids.contains(value) {
                    ids.push(value.clone())
                }
            }
        }
    }
    index["ids"] = json!(ids);
    write_json(
        store,
        &format!("{}/index.json", history_root(course)),
        index,
    )
}
fn snapshot(store: &Store, course: &str, bytes: &[u8]) -> Result<Value> {
    let digest = hash(bytes);
    let retained = bytes.len() <= MAX_BYTES as usize;
    if retained {
        let path = format!("{}/blobs/{digest}", history_root(course));
        if !store.path(&path)?.exists() {
            store.write(&path, bytes)?;
        }
    }
    Ok(json!({"hash":digest,"size":bytes.len(),"retained":retained}))
}
fn history_valid(path: &str) -> Result<()> {
    if let Some(p) = path.strip_prefix("files/") {
        valid_path(p)?
    } else if let Some(p) = path.strip_prefix("assignments/") {
        valid_path(p)?
    } else {
        return Err("History is only available for course and assignment files".into());
    }
    if excluded(path) {
        return Err("Excluded path cannot be stored in file history".into());
    }
    Ok(())
}
pub fn current_snapshot(store: &Store, course: &str, path: &str) -> Result<Value> {
    history_valid(path)?;
    let full = store.path(&format!("courses/{course}/{path}"))?;
    match fs::metadata(&full) {
        Ok(m) => {
            if !m.is_file() {
                return Err("Select a file to view its history".into());
            }
            if m.len() > MAX_BYTES {
                Ok(json!({"hash":metadata_revision(&m),"size":m.len(),"retained":false}))
            } else {
                snapshot(store, course, &fs::read(full).map_err(|e| e.to_string())?)
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(e) => Err(e.to_string()),
    }
}
fn same(a: &Value, b: &Value) -> bool {
    a["hash"] == b["hash"] && a["retained"] == b["retained"]
}
#[allow(clippy::too_many_arguments)] // Mirrors the persisted revision's independent fields.
fn append_history(
    store: &Store,
    course: &str,
    key: &str,
    entries: &mut Vec<Value>,
    path: &str,
    before: Value,
    after: Value,
    ctx: &Value,
    action: &str,
    previous: Option<&str>,
) -> Result<()> {
    let mut entry = ctx.clone();
    entry["id"] = json!(id());
    entry["sequence"] = json!(entries.len() + 1);
    entry["timestamp"] = json!(now());
    entry["action"] = json!(action);
    entry["path"] = json!(path);
    entry["before"] = before;
    entry["after"] = after;
    if let Some(p) = previous {
        entry["previousPath"] = json!(p)
    }
    entries.push(entry);
    write_json(store, &history_path(course, key), &json!(entries))
}
fn map_history(index: &mut Value, path: &str, key: &str, present: bool) {
    let (put, remove) = if present {
        ("paths", "archived")
    } else {
        ("archived", "paths")
    };
    index[put][path] = json!(key);
    if let Some(m) = index[remove].as_object_mut() {
        m.remove(path);
    }
}
fn observe(
    store: &Store,
    course: &str,
    path: &str,
    value: Value,
    ctx: &Value,
) -> Result<Option<(String, Vec<Value>, Value)>> {
    let mut index = read_index(store, course)?;
    let mut key = index["paths"][path]
        .as_str()
        .or(index["archived"][path].as_str())
        .unwrap_or("")
        .to_string();
    if key.is_empty() && value.is_null() {
        return Ok(None);
    }
    if key.is_empty() {
        key = id()
    }
    let mut entries = arr(&read_json(store, &history_path(course, &key), json!([]))?);
    if index["paths"][path].is_null()
        && !entries.is_empty()
        && s(entries.last().unwrap(), "path") != path
    {
        if value.is_null() {
            return Ok(Some((key, entries, index)));
        }
        key = id();
        entries.clear();
    }
    let last = entries
        .last()
        .map(|v| v["after"].clone())
        .unwrap_or(Value::Null);
    if entries.is_empty() || !same(&last, &value) {
        let action = if entries.is_empty() {
            "baseline"
        } else if value.is_null() {
            "deleted"
        } else {
            "observed"
        };
        append_history(
            store,
            course,
            &key,
            &mut entries,
            path,
            last,
            value.clone(),
            ctx,
            action,
            None,
        )?;
    }
    map_history(&mut index, path, &key, !value.is_null());
    save_index(store, course, &mut index)?;
    Ok(Some((key, entries, index)))
}
fn record_change(
    store: &Store,
    course: &str,
    path: &str,
    before: Option<&[u8]>,
    after: Option<&[u8]>,
    ctx: &Value,
) -> Result<()> {
    history_valid(path)?;
    let b = before
        .map(|v| snapshot(store, course, v))
        .transpose()?
        .unwrap_or(Value::Null);
    let a = after
        .map(|v| snapshot(store, course, v))
        .transpose()?
        .unwrap_or(Value::Null);
    let prior = observe(
        store,
        course,
        path,
        b.clone(),
        &json!({"actor":"external","explanation":"State observed before a tracked save. Earlier changes and their context are unknown."}),
    )?;
    let (key, mut entries, mut index) = match prior {
        Some((key, e, index)) if e.last().is_some_and(|v| s(v, "path") == path) => (key, e, index),
        Some((_, _, index)) => (id(), vec![], index),
        None => (id(), vec![], read_index(store, course)?),
    };
    append_history(
        store,
        course,
        &key,
        &mut entries,
        path,
        b,
        a.clone(),
        ctx,
        if before.is_none() {
            "created"
        } else if after.is_none() {
            "deleted"
        } else {
            "updated"
        },
        None,
    )?;
    map_history(&mut index, path, &key, !a.is_null());
    save_index(store, course, &mut index)
}
fn scan_paths(store: &Store, course: &str, roots: &[String]) -> Result<(Vec<String>, Vec<String>)> {
    fn scan(
        store: &Store,
        course: &str,
        p: &str,
        depth: usize,
        visited: &mut usize,
        files: &mut Vec<String>,
        skipped: &mut Vec<String>,
    ) -> Result<()> {
        if excluded(p) {
            return Ok(());
        }
        *visited += 1;
        if *visited > 20000 || depth > 24 {
            skipped.push(p.to_string());
            return Ok(());
        }
        let relative = format!("courses/{course}/{p}");
        let parent = Path::new(&relative)
            .parent()
            .ok_or("Invalid path")?
            .to_string_lossy()
            .to_string();
        store.path(&parent)?;
        let full = store.root.join(&relative);
        let m = match fs::symlink_metadata(&full) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        if m.file_type().is_symlink() {
            skipped.push(p.to_string());
            return Ok(());
        }
        if m.is_dir() {
            let mut names = fs::read_dir(&full)
                .map_err(|e| e.to_string())?
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            names.sort_by_key(|e| e.file_name());
            for entry in names {
                scan(
                    store,
                    course,
                    &format!("{p}/{}", entry.file_name().to_string_lossy()),
                    depth + 1,
                    visited,
                    files,
                    skipped,
                )?
            }
        } else if m.is_file() {
            if history_valid(p).is_ok() {
                files.push(p.to_string())
            } else {
                skipped.push(p.to_string())
            }
        }
        Ok(())
    }
    let (mut files, mut skipped, mut visited) = (vec![], vec![], 0);
    for root in roots {
        scan(
            store,
            course,
            root,
            0,
            &mut visited,
            &mut files,
            &mut skipped,
        )?
    }
    Ok((files, skipped))
}
fn track_before(store: &Store, course: &str, path: &str) -> Result<Vec<String>> {
    let (files, _) = scan_paths(store, course, &[path.into()])?;
    for p in &files {
        observe(
            store,
            course,
            p,
            current_snapshot(store, course, p)?,
            &json!({"actor":"external","explanation":"Existing file observed before a tracked operation; earlier history is unavailable."}),
        )?;
    }
    Ok(files)
}
fn record_operation(
    store: &Store,
    course: &str,
    paths: &[String],
    ctx: &Value,
    from: &str,
    to: Option<&str>,
) -> Result<()> {
    let mut index = read_index(store, course)?;
    for old in paths {
        let Some(key) = index["paths"][old].as_str().map(String::from) else {
            continue;
        };
        let mut entries = arr(&read_json(store, &history_path(course, &key), json!([]))?);
        let before = entries
            .last()
            .map(|e| e["after"].clone())
            .unwrap_or(Value::Null);
        let new = to
            .map(|to| format!("{to}{}", &old[from.len()..]))
            .unwrap_or_else(|| old.clone());
        append_history(
            store,
            course,
            &key,
            &mut entries,
            &new,
            before.clone(),
            if to.is_some() { before } else { Value::Null },
            ctx,
            if to.is_some() { "moved" } else { "deleted" },
            to.map(|_| old.as_str()),
        )?;
        map_history(&mut index, old, &key, false);
        if to.is_some() {
            map_history(&mut index, &new, &key, true)
        }
    }
    save_index(store, course, &mut index)
}
pub fn checkpoint(store: &Store, course: &str, ctx: &Value) -> Result<Value> {
    store.mutate(|state|{require_scope(state,course,None)?;let (files,skipped)=scan_paths(store,course,&["files".into(),"assignments".into()])?;let index=read_index(store,course)?;let mut paths:BTreeSet<String>=files.into_iter().collect();if let Some(m)=index["paths"].as_object(){for p in m.keys(){if !skipped.iter().any(|skip|contains(skip,p)){paths.insert(p.clone());}}}let mut changed=0;for p in paths{let value=current_snapshot(store,course,&p)?;let key=s(&index["paths"],&p);let entries=if key.is_empty(){vec![]}else{arr(&read_json(store,&history_path(course,key),json!([]))?)};if !same(&entries.last().map(|e|e["after"].clone()).unwrap_or(Value::Null),&value){changed+=1}observe(store,course,&p,value,ctx)?;}checkpoint_assignments(store,state,course,Some(ctx))?;Ok(json!({"changed":changed,"skipped":skipped,"coverage":"Observed snapshots only. Intermediate writes between checkpoints are not captured. Excluded dependencies, credentials and symlinks are omitted; files over 10 MB retain metadata only."}))})
}
pub fn load_snapshot(store: &Store, course: &str, snap: &Value) -> Result<Option<Vec<u8>>> {
    if snap.is_null() || snap["retained"] != true {
        return Ok(None);
    }
    let digest = arg(snap, "hash")?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err("Invalid snapshot hash".into());
    }
    let bytes = fs::read(store.path(&format!("{}/blobs/{digest}", history_root(course)))?)
        .map_err(|e| e.to_string())?;
    if hash(&bytes) != digest {
        return Err("History snapshot failed its integrity check".into());
    }
    Ok(Some(bytes))
}

pub fn diff(before: Option<&[u8]>, after: Option<&[u8]>) -> Value {
    fn decode(v: Option<&[u8]>) -> Option<&str> {
        let t = std::str::from_utf8(v.unwrap_or_default()).ok()?;
        (!t.contains('\0')).then_some(t)
    }
    let (Some(old), Some(new)) = (decode(before), decode(after)) else {
        return json!({"lines":[],"message":"Binary change. Download the before and after versions to compare.","coarse":false});
    };
    if old == new {
        return json!({"lines":[],"message":"No content changes.","coarse":false});
    }
    if old.len() + new.len() > 1_000_000 {
        return json!({"lines":[],"message":"Large text change. Full versions are retained; download them to compare.","coarse":true});
    }
    let a: Vec<_> = old.split_inclusive('\n').collect();
    let b: Vec<_> = new.split_inclusive('\n').collect();
    if a.len() + b.len() > 30000 {
        return json!({"lines":[],"message":"Too many lines for an inline diff. Download the full versions to compare.","coarse":true});
    }
    let mut prefix = 0;
    while prefix < a.len() && prefix < b.len() && a[prefix] == b[prefix] {
        prefix += 1
    }
    let mut suffix = 0;
    while suffix < a.len() - prefix
        && suffix < b.len() - prefix
        && a[a.len() - suffix - 1] == b[b.len() - suffix - 1]
    {
        suffix += 1
    }
    let (mut old_line, mut new_line) = (prefix.saturating_sub(3) + 1, prefix.saturating_sub(3) + 1);
    let mut lines = vec![];
    let mut emit = |kind: &str, text: &str| {
        let mut line = json!({"kind":kind,"text":text});
        if kind != "add" {
            line["oldLine"] = json!(old_line);
            old_line += 1
        }
        if kind != "remove" {
            line["newLine"] = json!(new_line);
            new_line += 1
        }
        lines.push(line);
    };
    for line in &a[prefix.saturating_sub(3)..prefix] {
        emit("context", line)
    }
    let aa = &a[prefix..a.len() - suffix];
    let bb = &b[prefix..b.len() - suffix];
    let width = bb.len() + 1;
    let coarse = (aa.len() + 1) * width > 2_000_000;
    if coarse {
        for l in aa {
            emit("remove", l)
        }
        for l in bb {
            emit("add", l)
        }
    } else {
        let mut table = vec![0u32; (aa.len() + 1) * width];
        for i in (0..aa.len()).rev() {
            for j in (0..bb.len()).rev() {
                table[i * width + j] = if aa[i] == bb[j] {
                    1 + table[(i + 1) * width + j + 1]
                } else {
                    table[(i + 1) * width + j].max(table[i * width + j + 1])
                }
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < aa.len() || j < bb.len() {
            if i < aa.len() && j < bb.len() && aa[i] == bb[j] {
                emit("context", aa[i]);
                i += 1;
                j += 1
            } else if i < aa.len()
                && (j == bb.len() || table[(i + 1) * width + j] >= table[i * width + j + 1])
            {
                emit("remove", aa[i]);
                i += 1
            } else {
                emit("add", bb[j]);
                j += 1
            }
        }
    }
    for l in &a[a.len() - suffix..a.len() - suffix + suffix.min(3)] {
        emit("context", l)
    }
    json!({"lines":lines,"coarse":coarse,"message":if coarse{"Large changed region shown as a replacement."}else{""}})
}
fn page_diff(mut d: Value, offset: usize, total: bool) -> Value {
    let lines = arr(&d["lines"]);
    d["lines"] = json!(lines.iter().skip(offset).take(200).collect::<Vec<_>>());
    d["nextOffset"] = if offset + 200 < lines.len() {
        json!(offset + 200)
    } else {
        Value::Null
    };
    if total {
        d["total"] = json!(lines.len());
        d["offset"] = json!(offset)
    }
    d
}
fn compare_snapshots(store: &Store, course: &str, a: &Value, b: &Value) -> Result<Value> {
    if [a, b].iter().any(|v| !v.is_null() && v["retained"] != true) {
        return Ok(
            json!({"lines":[],"message":"Only metadata was retained for files over 10 MB.","coarse":true}),
        );
    }
    Ok(diff(
        load_snapshot(store, course, a)?.as_deref(),
        load_snapshot(store, course, b)?.as_deref(),
    ))
}
fn history(store: &Store, course: &str, a: Option<&str>, args: &Value) -> Result<Value> {
    let offset = integer(args, "offset", 0, usize::MAX)?;
    let limit = integer(args, "limit", 50, 200)?.max(1);
    let diff_offset = integer(args, "diffOffset", 0, usize::MAX)?;
    let mut relative = None;
    if let Some(path) = args["path"].as_str() {
        let resolved = resolve(store, course, a, path, true)?;
        let canonical = arg(&resolved, "workspacePath")?
            .strip_prefix(&format!("courses/{course}/"))
            .ok_or("Invalid course path")?
            .to_string();
        observe(
            store,
            course,
            &canonical,
            current_snapshot(store, course, &canonical)?,
            &json!({"actor":"external","explanation":"Current file observed when history was opened. Any changes outside tracked tools have unknown author and context."}),
        )?;
        relative = Some(canonical);
    }
    let index = read_index(store, course)?;
    let key = args["historyId"].as_str().map(String::from).or_else(|| {
        relative
            .as_ref()
            .and_then(|p| index["paths"][p].as_str().or(index["archived"][p].as_str()))
            .map(String::from)
    });
    let Some(key) = key else {
        if let Some(p) = relative {
            return Ok(json!({"path":p,"historyId":null,"entries":[],"total":0,"nextOffset":null}));
        }
        let mut files = vec![];
        for key in arr(&index["ids"]) {
            let key = key.as_str().ok_or("Invalid history ID")?;
            valid_id(key)?;
            let entries = arr(&read_json(store, &history_path(course, key), json!([]))?);
            if let Some(last) = entries.last() {
                files.push(json!({"historyId":key,"path":last["path"],"timestamp":last["timestamp"],"revisions":entries.len(),"deleted":last["after"].is_null()}));
            }
        }
        files.sort_by(|a, b| s(b, "timestamp").cmp(s(a, "timestamp")));
        return Ok(
            json!({"files":files.iter().skip(offset).take(limit).collect::<Vec<_>>(),"total":files.len(),"nextOffset":if offset+limit<files.len(){json!(offset+limit)}else{Value::Null}}),
        );
    };
    valid_id(&key)?;
    if !arr(&index["ids"]).contains(&json!(key)) {
        return Err("History is outside this course or does not exist".into());
    }
    let mut entries = arr(&read_json(store, &history_path(course, &key), json!([]))?);
    if let Some(revision) = args["revisionId"].as_str() {
        valid_id(revision)?;
        let entry = entries
            .iter()
            .find(|e| s(e, "id") == revision)
            .ok_or("Revision does not belong to this file")?;
        let d = compare_snapshots(store, course, &entry["before"], &entry["after"])?;
        return Ok(json!({"historyId":key,"revision":entry,"diff":page_diff(d,diff_offset,true)}));
    }
    if let Some(path) = entries.last().and_then(|e| e["path"].as_str())
        && index["paths"][path] == key
    {
        observe(
            store,
            course,
            path,
            current_snapshot(store, course, path)?,
            &json!({"actor":"external","explanation":"Current file observed when history was opened. Changes outside tracked tools have unknown author and context."}),
        )?;
        entries = arr(&read_json(store, &history_path(course, &key), json!([]))?);
    }
    Ok(
        json!({"historyId":key,"path":entries.last().map(|e|&e["path"]),"entries":entries.iter().rev().skip(offset).take(limit).collect::<Vec<_>>(),"total":entries.len(),"nextOffset":if offset+limit<entries.len(){json!(offset+limit)}else{Value::Null}}),
    )
}

pub fn steps(store: &Store, course: &str, a: &str) -> Result<Vec<Value>> {
    Ok(arr(&read_json(
        store,
        &manifest(course, a, "timeline"),
        json!([]),
    )?))
}
fn save_step(
    store: &Store,
    course: &str,
    a: &str,
    mut note: Value,
    only_changed: bool,
) -> Result<Value> {
    let mut history = steps(store, course, a)?;
    let listing = tree(store, course, Some(a), &json!({}))?;
    let mut files = vec![];
    for e in arr(&listing["entries"]) {
        let original = format!("files/{}", s(&e["shared"], "path"));
        let missing = e["shared"]["missing"] == true;
        files.push(json!({"path":e["path"],"type":e["type"],"originalPath":original,"missing":missing,"snapshot":if e["type"]=="file"&&!missing{current_snapshot(store,course,&original)?}else{Value::Null}}));
    }
    let prior = history.last();
    if only_changed && prior.is_some_and(|p| p["files"] == json!(files)) {
        return Ok(prior.unwrap().clone());
    }
    let before = prior.map(|p| arr(&p["files"])).unwrap_or_default();
    let mut changes = vec![];
    for file in &files {
        let old = before.iter().find(|p| p["path"] == file["path"]);
        if old != Some(file) {
            changes.push(
                json!({"path":file["path"],"action":if old.is_some(){"changed"}else{"added"}}),
            );
        }
    }
    for file in before {
        if !files.iter().any(|f| f["path"] == file["path"]) {
            changes.push(json!({"path":file["path"],"action":"removed"}));
        }
    }
    if note["part"].is_null()
        && let Some(p) = prior.filter(|p| !p["part"].is_null())
    {
        note["part"] = p["part"].clone();
    }
    note["id"] = json!(id());
    note["timestamp"] = json!(now());
    note["files"] = json!(files);
    note["changes"] = json!(changes);
    history.push(note.clone());
    write_json(store, &manifest(course, a, "timeline"), &json!(history))?;
    Ok(note)
}
pub fn checkpoint_assignments(
    store: &Store,
    state: &Value,
    course: &str,
    ctx: Option<&Value>,
) -> Result<()> {
    for assignment in assignments(state, course)? {
        let a = arg(&assignment, "id")?;
        let old = steps(store, course, a)?;
        let refs = references(store, course, Some(a))?;
        if refs.is_empty() && old.is_empty() && ctx.is_none_or(|c| s(c, "assignmentId") != a) {
            continue;
        }
        let note = if let Some(c) = ctx {
            let mut note = json!({"title":c["tool"].as_str().unwrap_or("Command checkpoint").replace('_'," "),"markdown":format!("{}{}{}",s(c,"explanation"),if s(c,"context").is_empty(){String::new()}else{format!("\n\n{}",s(c,"context"))},if s(c,"command").is_empty(){String::new()}else{format!("\n\nCommand: {}",s(c,"command"))}),"actor":c["actor"]});
            if !c["runId"].is_null() {
                note["runId"] = c["runId"].clone()
            }
            note
        } else {
            json!({"title":"Observed workspace","markdown":"Workspace observed before a tracked change. Earlier decisions and intermediate outside edits are unknown.","actor":"external"})
        };
        save_step(
            store,
            course,
            a,
            note,
            ctx.is_none_or(|c| s(c, "assignmentId") != a),
        )?;
    }
    Ok(())
}
fn parts(history: &[Value]) -> Vec<Value> {
    let numbered = history.iter().any(|s| s["part"].is_object());
    let mut groups: BTreeMap<u64, Vec<&Value>> = BTreeMap::new();
    for step in history {
        if numbered && !step["part"].is_object() {
            continue;
        }
        groups
            .entry(step["part"]["order"].as_u64().unwrap_or(1))
            .or_default()
            .push(step);
    }
    groups.into_iter().map(|(order,group)|{let last=group.last().unwrap();let mut teaching=vec![];for step in &group{if let Some(md)=step["teachingMarkdown"].as_str(){teaching.push(json!({"markdown":md,"timestamp":step["timestamp"],"sourceIds":arr(&step["sourceIds"])}));}teaching.extend(arr(&step["teachingRevisions"]));}teaching.sort_by(|a,b|s(a,"timestamp").cmp(s(b,"timestamp")));let t=teaching.last();json!({"order":order,"title":last["part"]["title"].as_str().unwrap_or("Assignment solution"),"stepId":last["id"],"teachingMarkdown":t.map(|t|s(t,"markdown")).unwrap_or(""),"sourceIds":t.map(|t|arr(&t["sourceIds"])).unwrap_or_default(),"timestamp":last["timestamp"]})}).collect()
}
fn baseline(history: &[Value], index: usize) -> &Value {
    let step = &history[index];
    let ps = parts(&history[..=index]);
    let order = step["part"]["order"].as_u64().unwrap_or(1);
    let i = ps.iter().position(|p| p["order"] == order).unwrap_or(0);
    if i > 0
        && let Some(s) = history.iter().find(|s| s["id"] == ps[i - 1]["stepId"])
    {
        return s;
    }
    if step["part"].is_object()
        && let Some(s) = history.iter().find(|s| s["part"]["order"] == order)
    {
        return s;
    }
    &history[0]
}
fn learning(store: &Store, state: &Value, course: &str, a: &str, args: &Value) -> Result<Value> {
    let title = arg(args, "title")?.trim();
    let markdown = arg(args, "markdown")?.trim();
    if title.is_empty() || title.len() > 200 || markdown.len() < 20 || markdown.len() > 50000 {
        return Err("Learning needs a title and 20–50000 characters of markdown".into());
    }
    let teaching = args["teachingMarkdown"].as_str();
    if teaching.is_some_and(|t| t.trim().len() < 200 || t.len() > 50000) {
        return Err("teachingMarkdown must contain 200–50000 characters".into());
    }
    let phase = args["phase"].as_str().unwrap_or("work");
    if !matches!(phase, "plan" | "work" | "check" | "blocked" | "completed") {
        return Err("Invalid learning phase".into());
    }
    if args["part"].is_object() {
        if !(1..=1000).contains(&args["part"]["order"].as_u64().unwrap_or(0))
            || s(&args["part"], "title").trim().is_empty()
        {
            return Err("Part requires a number and title".into());
        }
        if phase == "completed" && teaching.is_none() {
            return Err("Complete this part with teachingMarkdown: teach the concepts, worked solution and why it works. Operational checkpoint markdown is not a student lesson.".into());
        }
    }
    let target = args["teachingForStepId"].as_str();
    if target.is_some() && teaching.is_none() {
        return Err(
            "teachingForStepId requires a student-facing teachingMarkdown explanation.".into(),
        );
    }
    let history = steps(store, course, a)?;
    if let Some(target) = target {
        valid_id(target)?;
        let anchor = history
            .iter()
            .find(|v| s(v, "id") == target)
            .ok_or("Teaching target must belong to this assignment and part.")?;
        if args["part"].is_object() && anchor["part"]["order"] != args["part"]["order"] {
            return Err("Teaching target must belong to this assignment and part.".into());
        }
    } else if args["part"].is_object()
        && let Some(latest) = history
            .iter()
            .rev()
            .find(|v| v["part"].is_object() && v["phase"].is_string())
        && args["part"]["order"].as_u64() > latest["part"]["order"].as_u64()
        && latest["phase"] != "completed"
    {
        return Err(
            "Complete the current part with a learning checkpoint before starting the next part."
                .into(),
        );
    }
    let sources = arr(&args["sourceIds"]);
    let gaps = arr(&args["gaps"]);
    if sources.is_empty() && gaps.is_empty() {
        return Err("Provide course sources or an explicit evidence gap".into());
    }
    let evidence = crate::core::evidence(store, state, course)?;
    let mut cited = vec![];
    for source in &sources {
        cited.push(
            evidence
                .iter()
                .find(|e| &e["id"] == source)
                .ok_or("Source is outside this course or no longer exists")?,
        )
    }
    let mut paths = vec![];
    for p in arr(&args["paths"]) {
        let resolved = resolve(
            store,
            course,
            Some(a),
            p.as_str().ok_or("Invalid related path")?,
            false,
        )?;
        if !Path::new(arg(&resolved, "absolutePath")?).exists() {
            return Err(format!("Assignment path not found: {p}"));
        }
        paths.push(arg(&resolved, "workspacePath")?.to_string());
    }
    let assignment = assignments(state, course)?
        .into_iter()
        .find(|v| s(v, "id") == a)
        .ok_or("Assignment not found")?;
    let file = format!("courses/{course}/memory/assignments/{}.md", id());
    let body = format!(
        "# {} — {title}\n\nAssignment ID: {a}\nPart: {}\nPhase: {phase}\nNext action: {}\n\nGenerated assignment learning record — an explanation of work, not primary instructor evidence.\nRecorded: {}\n\n{markdown}\n\n{}## Related files\n\n{}\n\n## Course evidence\n\n{}\n\n## Uncertainty and unverified checks\n\n{}\n",
        s(&assignment, "title"),
        if args["part"].is_object() {
            format!("{} — {}", args["part"]["order"], s(&args["part"], "title"))
        } else {
            "Unspecified".into()
        },
        args["nextAction"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("See checkpoint"),
        now(),
        teaching
            .map(|t| format!("## Teaching explanation\n\n{t}\n\n"))
            .unwrap_or_default(),
        paths
            .iter()
            .map(|p| format!("- {p}"))
            .collect::<Vec<_>>()
            .join("\n"),
        cited
            .iter()
            .map(|e| format!(
                "- [{}] {}{}\n  {}",
                s(e, "id"),
                s(e, "title"),
                if s(e, "url").is_empty() {
                    String::new()
                } else {
                    format!(" — {}", s(e, "url"))
                },
                s(e, "text")
            ))
            .collect::<Vec<_>>()
            .join("\n"),
        gaps.iter()
            .map(|g| format!("- {}", g.as_str().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("\n")
    );
    store.write(&file, body.as_bytes())?;
    if let Some(target) = target {
        let mut history = history;
        let anchor = history.iter_mut().find(|v| s(v, "id") == target).unwrap();
        let mut revisions = arr(&anchor["teachingRevisions"]);
        revisions.push(json!({"markdown":teaching.unwrap(),"timestamp":now(),"memoryPath":file,"sourceIds":sources}));
        anchor["teachingRevisions"] = json!(revisions);
        write_json(store, &manifest(course, a, "timeline"), &json!(history))?;
        return Ok(json!({"path":file,"indexed":true,"stepId":target}));
    }
    let mut note = json!({"title":title,"markdown":markdown,"actor":"agent","phase":phase,"nextAction":args["nextAction"].as_str().unwrap_or(""),"memoryPath":file,"sourceIds":sources,"gaps":gaps});
    if args["part"].is_object() {
        note["part"] = args["part"].clone()
    }
    if let Some(t) = teaching {
        note["teachingMarkdown"] = json!(t)
    }
    let saved = save_step(store, course, a, note, false)?;
    Ok(json!({"path":file,"indexed":true,"stepId":saved["id"]}))
}
fn snapshot_change<'a>(
    before: &'a [Value],
    after: &'a [Value],
    file: &Value,
) -> Option<&'static str> {
    let old = before.iter().find(|v| v["path"] == file["path"]);
    if !after.iter().any(|v| v["path"] == file["path"])
        || file["missing"] == true && old.is_some_and(|v| v["missing"] != true)
    {
        return Some("removed");
    }
    if file["type"] != "file" || file["missing"] == true {
        return None;
    }
    if old.is_none_or(|v| v["missing"] == true) {
        return Some("added");
    }
    let old = old.unwrap();
    if old["snapshot"]["hash"] != file["snapshot"]["hash"]
        || old["originalPath"] != file["originalPath"]
    {
        Some("changed")
    } else {
        None
    }
}
fn timeline(store: &Store, course: &str, a: &str, name: &str, args: &Value) -> Result<Value> {
    let history = steps(store, course, a)?;
    if name == "get_assignment_parts" {
        return Ok(json!({"parts":parts(&history)}));
    }
    let offset = integer(args, "offset", 0, usize::MAX)?;
    let diff_offset = integer(args, "diffOffset", 0, usize::MAX)?;
    let limit = integer(args, "limit", 100, 200)?.max(1);
    if name == "get_assignment_timeline" && args["stepId"].is_null() {
        let summaries: Vec<_> = history
            .iter()
            .skip(offset)
            .take(limit)
            .map(|s| {
                let mut s = s.clone();
                s["fileCount"] = json!(arr(&s["files"]).len());
                s.as_object_mut().unwrap().remove("files");
                s.as_object_mut().unwrap().remove("markdown");
                s
            })
            .collect();
        return Ok(
            json!({"steps":summaries,"total":history.len(),"nextOffset":if offset+limit<history.len(){json!(offset+limit)}else{Value::Null}}),
        );
    }
    let step_id = arg(args, "stepId")?;
    valid_id(step_id)?;
    let index = history
        .iter()
        .position(|v| s(v, "id") == step_id)
        .ok_or("Step not found in this assignment")?;
    let step = &history[index];
    let after = arr(&step["files"]);
    let before = arr(&baseline(&history, index)["files"]);
    if name == "get_assignment_part_changes" {
        let mut names = BTreeSet::new();
        for f in before
            .iter()
            .chain(after.iter())
            .filter(|f| f["type"] == "file")
        {
            names.insert(s(f, "originalPath"));
        }
        let (mut changed, mut added, mut deleted, mut omitted, mut approx) = (0, 0, 0, 0, false);
        for p in names {
            let old = before
                .iter()
                .find(|f| s(f, "originalPath") == p && f["type"] == "file");
            let new = after
                .iter()
                .find(|f| s(f, "originalPath") == p && f["type"] == "file");
            if let (Some(o), Some(n)) = (old, new)
                && o["snapshot"]["hash"] == n["snapshot"]["hash"]
                && o["missing"] == n["missing"]
            {
                continue;
            }
            changed += 1;
            if [old, new]
                .iter()
                .flatten()
                .any(|f| f["missing"] == true || f["snapshot"]["retained"] != true)
            {
                omitted += 1;
                continue;
            }
            let d = compare_snapshots(
                store,
                course,
                &old.map(|f| f["snapshot"].clone()).unwrap_or(Value::Null),
                &new.map(|f| f["snapshot"].clone()).unwrap_or(Value::Null),
            )?;
            let lines = arr(&d["lines"]);
            added += lines.iter().filter(|l| l["kind"] == "add").count();
            deleted += lines.iter().filter(|l| l["kind"] == "remove").count();
            approx |= d["coarse"] == true;
            if d["coarse"] == true && lines.is_empty() {
                omitted += 1
            }
        }
        return Ok(
            json!({"filesChanged":changed,"linesAdded":added,"linesDeleted":deleted,"omittedFiles":omitted,"approximate":approx}),
        );
    }
    if name == "read_assignment_snapshot" && s(args, "mode") == "directory" {
        let directory = s(args, "directory");
        if !directory.is_empty() {
            valid_path(directory)?
        }
        let mut visible = after.clone();
        visible.extend(
            before
                .iter()
                .filter(|f| !after.iter().any(|a| a["path"] == f["path"]))
                .cloned(),
        );
        if !directory.is_empty()
            && !visible
                .iter()
                .any(|f| s(f, "path") == directory && f["type"] == "directory")
        {
            return Err("Folder not found in this workspace state".into());
        }
        let mut entries = vec![];
        let mut expanded = BTreeSet::new();
        for file in visible {
            let p = arg(&file, "path")?;
            valid_path(p)?;
            let change = snapshot_change(&before, &after, &file);
            if change.is_some() && file["type"] == "file" {
                let components: Vec<_> = p.split('/').collect();
                for i in 1..components.len() {
                    expanded.insert(components[..i].join("/"));
                }
            }
            if p.rsplit_once('/').map(|v| v.0).unwrap_or("") != directory {
                continue;
            }
            let original = arg(&file, "originalPath")?;
            history_valid(original)?;
            let mut entry = json!({"path":p,"name":p.rsplit('/').next(),"type":file["type"],"size":file["snapshot"]["size"].as_u64().unwrap_or(0),"revision":file["snapshot"]["hash"].as_str().unwrap_or(step_id),"modifiedAt":step["timestamp"],"absolutePath":store.path(&format!("courses/{course}/{original}"))?,"shared":{"path":original.strip_prefix("files/").ok_or("Invalid saved file path")?,"referencePath":p.split('/').next(),"root":!p.contains('/'),"missing":file["missing"]}});
            if let Some(c) = change {
                entry["change"] = json!(c)
            }
            entries.push(entry);
        }
        let root = base(course, None);
        let mut out = json!({"courseId":course,"assignmentId":a,"root":root,"absoluteRoot":store.path(&root)?,"revision":step_id,"entries":entries});
        if directory.is_empty() {
            out["expandedDirectories"] = json!(expanded)
        }
        return Ok(out);
    }
    if name == "get_assignment_timeline" && args["path"].is_null() {
        return Ok(json!({"step":step}));
    }
    let path = arg(args, "path")?;
    valid_path(path)?;
    let current = after.iter().find(|v| s(v, "path") == path);
    let prior = if name == "get_assignment_timeline" {
        if index > 0 {
            arr(&history[index - 1]["files"])
        } else {
            vec![]
        }
    } else {
        before.clone()
    };
    let previous = prior.iter().find(|v| s(v, "path") == path);
    let file = current
        .or(previous)
        .ok_or("File not found in this workspace state")?;
    if name == "assignment_step_bytes" {
        let bytes = load_snapshot(store, course, &file["snapshot"])?
            .ok_or("Snapshot bytes were not retained")?;
        return Ok(json!({"bytes":STANDARD.encode(bytes),"path":path}));
    }
    if name == "get_assignment_timeline" {
        let snapshot = current
            .map(|f| f["snapshot"].clone())
            .unwrap_or(Value::Null);
        let bytes = load_snapshot(store, course, &snapshot)?;
        let d = compare_snapshots(
            store,
            course,
            &previous
                .map(|f| f["snapshot"].clone())
                .unwrap_or(Value::Null),
            &snapshot,
        )?;
        let mut out =
            json!({"file":current,"truncated":false,"diff":page_diff(d,diff_offset,false)});
        if let Some(bytes) = bytes
            && let Ok(text) = std::str::from_utf8(&bytes)
            && !text.contains('\0')
        {
            out["content"] = json!(text.chars().take(200000).collect::<String>());
            out["truncated"] = json!(text.chars().count() > 200000);
        }
        return Ok(out);
    }
    if file["type"] != "file" {
        return Err("File not found in this workspace state".into());
    }
    let original = arg(file, "originalPath")?;
    history_valid(original)?;
    let bytes = load_snapshot(store, course, &file["snapshot"])?;
    let mut out = json!({"path":path,"workspacePath":format!("courses/{course}/{original}"),"absolutePath":store.path(&format!("courses/{course}/{original}"))?,"revision":file["snapshot"]["hash"].as_str().unwrap_or(step_id),"size":file["snapshot"]["size"].as_u64().unwrap_or(0),"mediaType":media_type(path),"shared":{"path":original.strip_prefix("files/").ok_or("Invalid saved file path")?,"referencePath":path.split('/').next(),"root":!path.contains('/'),"missing":file["missing"]}});
    if snapshot_change(&before, &after, file).is_some() {
        let b = previous
            .filter(|f| f["missing"] != true)
            .map(|f| f["snapshot"].clone())
            .unwrap_or(Value::Null);
        let a = current
            .filter(|f| f["missing"] != true)
            .map(|f| f["snapshot"].clone())
            .unwrap_or(Value::Null);
        out["diff"] = page_diff(
            compare_snapshots(store, course, &b, &a)?,
            diff_offset,
            false,
        );
    }
    if let Some(bytes) = bytes
        && let Ok(text) = std::str::from_utf8(&bytes)
        && !text.contains('\0')
    {
        out["content"] = json!(text)
    }
    if file["missing"] == true {
        out["snapshotNotice"] = json!("The original was missing when this part was recorded.")
    } else if file["snapshot"]["retained"] != true {
        out["snapshotNotice"] =
            json!("Only metadata was retained for this file; saved bytes are unavailable.")
    }
    Ok(out)
}

/// Preserve legacy assignment storage with a recovery copy before any move.
pub fn ensure_shared(store: &Store, state: &Value, course: &str) -> Result<()> {
    for assignment in assignments(state, course)? {
        let a = arg(&assignment, "id")?;
        let old = format!("courses/{course}/assignments/{a}");
        let old_full = store.path(&old)?;
        if old_full.is_dir() {
            let children = fs::read_dir(&old_full)
                .map_err(|e| e.to_string())?
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            if !children.is_empty() {
                let dest = base(course, Some(a));
                let full = store.path(&dest)?;
                if full.exists() {
                    return Err(format!(
                        "Assignment migration needs conflict resolution: {} already exists. Original files were preserved.",
                        full.display()
                    ));
                }
                let runs = store.path(&format!("courses/{course}/.runtime/commands"))?;
                if runs.exists() {
                    for entry in fs::read_dir(runs).map_err(|e| e.to_string())? {
                        let entry = entry.map_err(|e| e.to_string())?;
                        let run = read_json(
                            store,
                            &format!(
                                "courses/{course}/.runtime/commands/{}/run.json",
                                entry.file_name().to_string_lossy()
                            ),
                            json!({}),
                        )?;
                        if matches!(s(&run, "status"), "running" | "queued")
                            && Path::new(s(&run, "cwd")).starts_with(&old_full)
                        {
                            return Err(format!(
                                "Wait for command {} to finish before migrating its assignment files",
                                s(&run, "id")
                            ));
                        }
                    }
                }
                let mut refs = references(store, course, Some(a))?;
                for child in children {
                    let kind = child.file_type().map_err(|e| e.to_string())?;
                    if kind.is_symlink() || !kind.is_dir() && !kind.is_file() {
                        return Err(
                            "Assignment migration requires regular files and folders".into()
                        );
                    }
                    let name = child.file_name().to_string_lossy().to_string();
                    valid_path(&name)?;
                    let target = format!("assignments/{a}/{name}");
                    if let Some(r) = refs
                        .iter()
                        .find(|r| s(r, "path").eq_ignore_ascii_case(&name))
                    {
                        if r["targetPath"] != target {
                            return Err(format!("Assignment migration reference conflict: {name}"));
                        }
                    } else {
                        refs.push(json!({"path":name,"targetPath":target,"type":if kind.is_dir(){"directory"}else{"file"},"createdAt":now(),"excludedPaths":[]}));
                    }
                }
                let recovery = format!(".trash/assignment-storage/{}/{course}/{a}", id());
                copy_tree(&old_full, &store.path(&recovery)?)?;
                let from = format!("assignments/{a}");
                let tracked = track_before(store, course, &from)?;
                write_json(store, &manifest(course, a, "references"), &json!(refs))?;
                record_operation(
                    store,
                    course,
                    &tracked,
                    &json!({"actor":"agent","assignmentId":a,"tool":"migrate_assignment_storage","explanation":"Preserved assignment files in course Files; assignment now selects shared originals.","context":format!("Recovery copy: {recovery}")}),
                    &from,
                    Some(&format!("files/assignments/{a}")),
                )?;
                fs::create_dir_all(full.parent().ok_or("Invalid destination")?)
                    .map_err(|e| e.to_string())?;
                fs::rename(old_full, full).map_err(|e| e.to_string())?;
            }
        }
        let legacy = format!("courses/{course}/assignments/{a}.md");
        let source = store.path(&legacy)?;
        if source.exists() {
            let target = format!("assignments/{a}/legacy-draft.md");
            let destination = format!("courses/{course}/files/{target}");
            if store.path(&destination)?.exists() {
                return Err(
                    "Legacy draft migration destination already exists; original preserved".into(),
                );
            }
            let mut refs = references(store, course, Some(a))?;
            if let Some(r) = refs
                .iter()
                .find(|r| s(r, "path").eq_ignore_ascii_case("legacy-draft.md"))
            {
                if r["targetPath"] != target {
                    return Err(
                        "Legacy draft display name is already in use; original preserved".into(),
                    );
                }
            } else {
                refs.push(json!({"path":"legacy-draft.md","targetPath":target,"type":"file","createdAt":now(),"excludedPaths":[]}));
            }
            let recovery = format!(".trash/assignment-storage/{}/{course}/{a}.md", id());
            copy_tree(&source, &store.path(&recovery)?)?;
            let from = format!("assignments/{a}.md");
            let tracked = track_before(store, course, &from)?;
            write_json(store, &manifest(course, a, "references"), &json!(refs))?;
            record_operation(
                store,
                course,
                &tracked,
                &json!({"actor":"agent","assignmentId":a,"tool":"migrate_assignment_storage","explanation":"Preserved legacy assignment draft in course Files.","context":format!("Recovery copy: {recovery}")}),
                &from,
                Some(&format!("files/{target}")),
            )?;
            let full = store.path(&destination)?;
            fs::create_dir_all(full.parent().ok_or("Invalid destination")?)
                .map_err(|e| e.to_string())?;
            fs::rename(source, full).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn complete_assignment_job(
    store: &Store,
    state: &mut Value,
    job: &Value,
    answer: &Value,
) -> Result<()> {
    let course = arg(job, "courseId")?;
    let a = arg(job, "id")?;
    ensure_shared(store, state, course)?;
    let canonical = format!("files/assignments/{a}/draft.md");
    let path = format!("courses/{course}/{canonical}");
    let before = match fs::read(store.path(&path)?) {
        Ok(v) => Some(v),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    let sources = arr(&answer["citations"])
        .iter()
        .map(|v| v.as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let gaps = arr(&answer["gaps"])
        .iter()
        .map(|v| format!("- {}", v.as_str().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n");
    let content = format!(
        "# {}\n\n{}\n\nSources: {sources}\n\n{gaps}",
        s(job, "prompt"),
        s(answer, "markdown")
    );
    let ctx = json!({"actor":"agent","assignmentId":a,"explanation":"Completed the assignment study job.","context":job["prompt"],"tool":"complete_job"});
    checkpoint_assignments(store, state, course, None)?;
    store.write(&path, content.as_bytes())?;
    record_change(
        store,
        course,
        &canonical,
        before.as_deref(),
        Some(content.as_bytes()),
        &ctx,
    )?;
    show_created(store, course, Some(a), "draft.md")?;
    append_memory(
        store,
        course,
        "Completed assignment study job",
        &json!({"explanation":format!("{}\n\nSources: {sources}\n\nUncertainty: {gaps}",s(answer,"markdown"))}),
        &[canonical],
    )?;
    checkpoint_assignments(store, state, course, Some(&ctx))
}

pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "list_assignments"
            | "create_assignment"
            | "delete_assignment"
            | "reference_course_path"
            | "list_course_files"
            | "read_course_file"
            | "create_course_file"
            | "write_course_file"
            | "edit_course_file"
            | "create_course_directory"
            | "delete_course_path"
            | "move_course_path"
            | "reorder_course_files"
            | "list_assignment_files"
            | "read_assignment_file"
            | "create_assignment_file"
            | "write_assignment_file"
            | "edit_assignment_file"
            | "create_assignment_directory"
            | "delete_assignment_path"
            | "move_assignment_path"
            | "reorder_assignment_files"
            | "record_assignment_learning"
            | "get_file_history"
            | "get_assignment_timeline"
            | "get_assignment_parts"
            | "get_assignment_part_changes"
            | "read_assignment_snapshot"
            | "assignment_step_bytes"
            | "assignment_bytes"
            | "download_file_revision"
            | "list_assignment_directory"
            | "assignment_path_revision"
            | "probe_assignment_files"
            | "open_file_explorer"
    )
}
pub fn call(store: &Store, name: &str, args: Value) -> Result<Value> {
    if !handles(name) {
        return Err(format!("Unknown tool: {name}"));
    }
    let course = arg(&args, "courseId")?;
    let assignment = args["assignmentId"].as_str();
    require_scope(&store.read()?, course, assignment)?;
    if name == "list_assignments" {
        return Ok(json!(assignments(&store.read()?, course)?));
    }
    if matches!(
        name,
        "get_assignment_timeline"
            | "get_assignment_parts"
            | "get_assignment_part_changes"
            | "read_assignment_snapshot"
            | "assignment_step_bytes"
    ) {
        let a = assignment.ok_or("assignmentId is required")?;
        return timeline(store, course, a, name, &args);
    }
    if name == "list_assignment_directory" {
        return tree(
            store,
            course,
            assignment,
            &json!({"directory":args["directory"],"shallow":true}),
        );
    }
    if name == "assignment_path_revision" {
        let path = arg(&args, "path")?;
        let listing = tree(store, course, assignment, &json!({"target":path}))?;
        let e = arr(&listing["entries"])
            .into_iter()
            .find(|e| s(e, "path") == path)
            .ok_or("Assignment path not found")?;
        return Ok(json!({"revision":e["revision"]}));
    }
    if name == "probe_assignment_files" {
        let refs = references(store, course, assignment)?;
        if let Some(r) = refs.first() {
            return Ok(json!({"hasEntries":true,"firstPath":r["path"]}));
        }
        let mut roots = vec![base(course, assignment)];
        if let Some(a) = assignment {
            roots.push(format!("courses/{course}/assignments/{a}"));
        }
        for root in roots {
            let full = store.path(&root)?;
            if !full.exists() {
                continue;
            }
            for child in fs::read_dir(full).map_err(|e| e.to_string())? {
                let child = child.map_err(|e| e.to_string())?;
                let kind = child.file_type().map_err(|e| e.to_string())?;
                let name = child.file_name().to_string_lossy().to_string();
                if !excluded(&name) && (kind.is_file() || kind.is_dir()) {
                    return Ok(json!({"hasEntries":true,"firstPath":name}));
                }
            }
        }
        return Ok(json!({"hasEntries":false}));
    }
    if name == "open_file_explorer" {
        return open_explorer(store, course, assignment, &args);
    }
    store.mutate(|state| mutate_call(store, state, name, &args))
}

fn mutate_call(store: &Store, state: &mut Value, name: &str, args: &Value) -> Result<Value> {
    let course = arg(args, "courseId")?;
    let assignment = args["assignmentId"].as_str();
    require_scope(state, course, assignment)?;
    if name == "create_assignment" {
        let title = arg(args, "title")?.trim();
        if title.is_empty() || title.len() > 200 || s(args, "description").len() > 12000 {
            return Err("Invalid assignment title or description".into());
        }
        let a = id();
        let item = json!({"id":a,"courseId":course,"title":title,"description":args["description"].as_str().unwrap_or(""),"createdAt":now()});
        if !state["assignments"].is_array() {
            state["assignments"] = json!([])
        }
        state["assignments"]
            .as_array_mut()
            .unwrap()
            .push(item.clone());
        write_json(
            store,
            &format!("courses/{course}/agent/assignments/{a}.json"),
            &item,
        )?;
        let mut out = item;
        out["root"] = json!(base(course, Some(&a)));
        out["instructions"] = json!(
            "Work through actual assignment parts sequentially. Read course evidence and previous learning before drafting and throughout work. Use record_assignment_learning before work and after each meaningful edit/check; save operational markdown and a substantial separate teachingMarkdown lesson for every completed part. get_assignment_timeline preserves saved workspace states; get_assignment_parts has one student stop per real part. Files are shared course originals; reference_course_path reuses existing files. Removing assignment paths only detaches the view. Supply explanations and context for tracked file edits; use local course commands for checks. Follow docs/ASSIGNMENTS.md. Never submit coursework."
        );
        return Ok(out);
    }
    if name == "delete_assignment" {
        let a = assignment.ok_or("assignmentId is required")?;
        let item = assignments(state, course)?
            .into_iter()
            .find(|v| s(v, "id") == a)
            .ok_or("Assignment not found")?;
        let recovery = format!(".trash/assignments/{}/assignment.json", id());
        write_json(store, &recovery, &item)?;
        if !state["hiddenAssignments"].is_array() {
            state["hiddenAssignments"] = json!([])
        }
        state["hiddenAssignments"]
            .as_array_mut()
            .unwrap()
            .push(json!(a));
        return Ok(json!({"id":a,"deleted":true,"recoveryPath":recovery}));
    }
    if name == "get_file_history" {
        return history(store, course, assignment, args);
    }
    if name == "download_file_revision" {
        let result = history(store, course, assignment, args)?;
        let side = args["side"].as_str().unwrap_or("after");
        if !matches!(side, "before" | "after") {
            return Err("Invalid snapshot side".into());
        }
        if result["revision"].is_null() {
            return Err("History and revision IDs are required".into());
        }
        let bytes = load_snapshot(store, course, &result["revision"][side])?
            .ok_or("No retained snapshot for this side")?;
        return Ok(
            json!({"bytes":STANDARD.encode(bytes),"name":s(&result["revision"],"path").rsplit('/').next()}),
        );
    }
    ensure_shared(store, state, course)?;
    if matches!(name, "list_course_files" | "list_assignment_files") {
        return tree(store, course, assignment, &json!({}));
    }
    if name == "record_assignment_learning" {
        return learning(
            store,
            state,
            course,
            assignment.ok_or("assignmentId is required")?,
            args,
        );
    }
    if matches!(
        name,
        "read_course_file" | "read_assignment_file" | "assignment_bytes"
    ) {
        return read_file(
            store,
            course,
            assignment,
            arg(args, "path")?,
            name == "assignment_bytes",
        );
    }
    if name == "reference_course_path" {
        let a = assignment.ok_or("assignmentId is required")?;
        let path = arg(args, "path")?;
        valid_path(path)?;
        if path.contains('/') {
            return Err("References must be placed at the assignment root".into());
        }
        let target = arg(args, "targetPath")?;
        let resolved = resolve(store, course, None, target, false)?;
        let m = fs::metadata(arg(&resolved, "absolutePath")?).map_err(|e| e.to_string())?;
        if !m.is_file() && !m.is_dir() {
            return Err("Only files and folders can be referenced".into());
        }
        let mut refs = references(store, course, assignment)?;
        if refs.len() >= 2000 {
            return Err("An assignment supports up to 2000 displayed roots".into());
        }
        if refs.iter().any(|r| s(r, "path").eq_ignore_ascii_case(path))
            || store
                .path(&format!("{}/{path}", base(course, assignment)))?
                .exists()
        {
            return Err("Reference name already exists".into());
        }
        refs.push(json!({"path":path,"targetPath":target,"type":if m.is_dir(){"directory"}else{"file"},"createdAt":now(),"excludedPaths":[]}));
        checkpoint_assignments(store, state, course, None)?;
        write_json(store, &manifest(course, a, "references"), &json!(refs))?;
        let mut ctx = context(args, "reference_path");
        ctx["explanation"] = json!(format!(
            "Display files/{target} as {path}; reuse the shared original."
        ));
        checkpoint_assignments(store, state, course, Some(&ctx))?;
        append_memory(
            store,
            course,
            "Referenced shared path",
            &ctx,
            &[format!("files/{target}")],
        )?;
        return tree(store, course, assignment, &json!({}));
    }
    if matches!(name, "reorder_assignment_files" | "reorder_course_files") {
        let options = if args.get("directory").is_some() {
            json!({"directory":args["directory"],"shallow":true})
        } else {
            json!({})
        };
        let listing = tree(store, course, assignment, &options)?;
        if listing["revision"] != args["expectedRevision"] {
            return Err("Workspace changed. Refresh before reordering.".into());
        }
        let paths = args["paths"].as_array().ok_or("paths is required")?;
        let entries = arr(&listing["entries"]);
        let unique: BTreeSet<_> = paths.iter().map(|v| v.as_str().unwrap_or("")).collect();
        if unique.len() != paths.len()
            || entries.len() != paths.len()
            || paths
                .iter()
                .any(|p| !entries.iter().any(|e| &e["path"] == p))
        {
            return Err("Include every current path exactly once".into());
        }
        checkpoint_assignments(store, state, course, None)?;
        let old = arr(&read_json(
            store,
            &order_path(course, assignment),
            json!([]),
        )?);
        let mut next = paths.clone();
        next.extend(old.into_iter().filter(|p| !paths.contains(p)));
        write_json(store, &order_path(course, assignment), &json!(next))?;
        checkpoint_assignments(store, state, course, Some(&context(args, "reorder_files")))?;
        return tree(store, course, assignment, &options);
    }
    let path = arg(args, "path")?;
    valid_path(path)?;
    let resolved = resolve(
        store,
        course,
        assignment,
        path,
        matches!(name, "delete_assignment_path" | "move_assignment_path"),
    )?;
    let full = PathBuf::from(arg(&resolved, "absolutePath")?);
    let canonical = arg(&resolved, "workspacePath")?
        .strip_prefix(&format!("courses/{course}/"))
        .ok_or("Invalid course path")?;
    if matches!(
        name,
        "create_course_directory" | "create_assignment_directory"
    ) {
        if full.exists() {
            return Err("Path already exists".into());
        }
        if arr(&tree(store, course, assignment, &json!({}))?["entries"]).len()
            >= if assignment.is_some() { 1980 } else { 19980 }
        {
            return Err("Assignment workspace is full".into());
        }
        checkpoint_assignments(store, state, course, None)?;
        fs::create_dir_all(&full).map_err(|e| e.to_string())?;
        show_created(store, course, assignment, path)?;
        let mut ctx = context(args, "create_folder");
        ctx["explanation"] = json!(format!("Create folder {path} to organize the workspace."));
        append_memory(store, course, "Created folder", &ctx, &[canonical.into()])?;
        checkpoint_assignments(store, state, course, Some(&ctx))?;
        return tree(store, course, assignment, &json!({}));
    }
    explanation(args)?;
    if matches!(
        name,
        "create_course_file"
            | "create_assignment_file"
            | "write_course_file"
            | "write_assignment_file"
            | "edit_course_file"
            | "edit_assignment_file"
    ) {
        let create = name.starts_with("create_");
        let present = full.exists();
        if create && present {
            return Err("Path already exists".into());
        }
        if !create && !present {
            return Err("Use create_assignment_file for a new file".into());
        }
        if create
            && arr(&tree(store, course, assignment, &json!({}))?["entries"]).len()
                >= if assignment.is_some() { 1980 } else { 19980 }
        {
            return Err("Assignment workspace is full".into());
        }
        let before = if present {
            let e = check_revision(
                store,
                course,
                assignment,
                path,
                arg(args, "expectedRevision")?,
            )?;
            if e["type"] != "file" {
                return Err("Cannot write a directory".into());
            }
            Some(fs::read(&full).map_err(|e| e.to_string())?)
        } else {
            None
        };
        let bytes = if name.starts_with("edit_") {
            let mut text = String::from_utf8(before.clone().unwrap_or_default())
                .map_err(|_| "Binary files cannot be text edited")?;
            if text.contains('\0') {
                return Err("Binary files cannot be text edited".into());
            }
            let edits = args["edits"].as_array().ok_or("edits is required")?;
            if edits.is_empty() || edits.len() > 100 {
                return Err("Provide 1–100 edits".into());
            }
            for edit in edits {
                let old = arg(edit, "oldText")?;
                let new = arg(edit, "newText")?;
                if old.is_empty() || old.len() > 200000 || new.len() > 200000 {
                    return Err("Invalid edit".into());
                }
                let matches = text.match_indices(old).collect::<Vec<_>>();
                if matches.len() != 1 {
                    return Err(
                        "Each oldText must match exactly once; include more surrounding text"
                            .into(),
                    );
                }
                let start = matches[0].0;
                text.replace_range(start..start + old.len(), new);
            }
            text.into_bytes()
        } else {
            let content = arg(args, "content")?;
            match args["encoding"].as_str().unwrap_or("utf8") {
                "utf8" => content.as_bytes().to_vec(),
                "base64" => {
                    let decoded = STANDARD
                        .decode(content)
                        .map_err(|_| "Invalid base64 content")?;
                    if STANDARD.encode(&decoded) != content {
                        return Err("Invalid base64 content".into());
                    }
                    decoded
                }
                _ => return Err("Invalid encoding".into()),
            }
        };
        if bytes.len() > MAX_BYTES as usize {
            return Err("Files must be under 10 MB".into());
        }
        let recovery = if present {
            Some(backup(store, course, assignment, path)?)
        } else {
            None
        };
        checkpoint_assignments(store, state, course, None)?;
        store.write(arg(&resolved, "workspacePath")?, &bytes)?;
        let ctx = context(args, if create { "create_file" } else { "write_file" });
        record_change(
            store,
            course,
            canonical,
            before.as_deref(),
            Some(&bytes),
            &ctx,
        )?;
        append_memory(
            store,
            course,
            if create {
                "Created file"
            } else {
                "Updated file"
            },
            args,
            &[canonical.into()],
        )?;
        show_created(store, course, assignment, path)?;
        checkpoint_assignments(store, state, course, Some(&ctx))?;
        let mut out = read_file(store, course, assignment, path, false)?;
        if let Some(p) = recovery {
            out["recoveryPath"] = json!(p)
        }
        return Ok(out);
    }
    check_revision(
        store,
        course,
        assignment,
        path,
        arg(args, "expectedRevision")?,
    )?;
    let mut refs = references(store, course, assignment)?;
    if matches!(name, "delete_course_path" | "delete_assignment_path") {
        if let Some(a) = assignment
            && let Some(i) = refs.iter().position(|r| contains(s(r, "path"), path))
        {
            checkpoint_assignments(store, state, course, None)?;
            if refs[i]["path"] == path {
                refs.remove(i);
            } else {
                let mut hidden = arr(&refs[i]["excludedPaths"]);
                hidden.push(json!(path));
                refs[i]["excludedPaths"] = json!(hidden);
            }
            write_json(store, &manifest(course, a, "references"), &json!(refs))?;
            checkpoint_assignments(
                store,
                state,
                course,
                Some(&context(args, "remove_reference")),
            )?;
            append_memory(
                store,
                course,
                "Removed shared reference",
                args,
                &[canonical.into()],
            )?;
            return Ok(json!({"path":path,"detached":true}));
        }
        let tracked = track_before(store, course, canonical)?;
        let recovery = backup(store, course, assignment, path)?;
        checkpoint_assignments(store, state, course, None)?;
        if full.is_dir() {
            fs::remove_dir_all(&full).map_err(|e| e.to_string())?
        } else {
            fs::remove_file(&full).map_err(|e| e.to_string())?
        }
        let ctx = context(args, "delete_path");
        record_operation(store, course, &tracked, &ctx, canonical, None)?;
        append_memory(
            store,
            course,
            "Deleted path (recovery copy saved)",
            args,
            &[canonical.into()],
        )?;
        checkpoint_assignments(store, state, course, Some(&ctx))?;
        return Ok(json!({"path":path,"recoveryPath":recovery}));
    }
    if matches!(name, "move_course_path" | "move_assignment_path") {
        let destination = arg(args, "destination")?;
        valid_path(destination)?;
        if contains(path, destination) {
            return Err("Cannot move a path into itself".into());
        }
        let root_ref = refs.iter().position(|r| s(r, "path") == path);
        if let (Some(a), Some(i)) = (assignment, root_ref)
            && !destination.contains('/')
        {
            if refs
                .iter()
                .any(|r| s(r, "path").eq_ignore_ascii_case(destination))
                || store
                    .path(&format!("{}/{destination}", base(course, assignment)))?
                    .exists()
            {
                return Err("Destination already exists".into());
            }
            checkpoint_assignments(store, state, course, None)?;
            refs[i]["path"] = json!(destination);
            refs[i]["excludedPaths"] = json!(
                arr(&refs[i]["excludedPaths"])
                    .iter()
                    .map(|v| format!("{destination}{}", &v.as_str().unwrap_or("")[path.len()..]))
                    .collect::<Vec<_>>()
            );
            write_json(store, &manifest(course, a, "references"), &json!(refs))?;
            checkpoint_assignments(
                store,
                state,
                course,
                Some(&context(args, "rename_reference")),
            )?;
            return tree(store, course, assignment, &json!({}));
        }
        let to = resolve(store, course, assignment, destination, false)?;
        let dest = PathBuf::from(arg(&to, "absolutePath")?);
        if dest.exists() {
            return Err("Destination already exists".into());
        }
        if dest.starts_with(&full) {
            return Err("Cannot move a shared path into itself through another reference".into());
        }
        let canonical_to = arg(&to, "workspacePath")?
            .strip_prefix(&format!("courses/{course}/"))
            .ok_or("Invalid course path")?;
        let tracked = track_before(store, course, canonical)?;
        checkpoint_assignments(store, state, course, None)?;
        fs::create_dir_all(dest.parent().ok_or("Invalid destination")?)
            .map_err(|e| e.to_string())?;
        fs::rename(&full, &dest).map_err(|e| e.to_string())?;
        let ctx = context(args, "move_path");
        record_operation(store, course, &tracked, &ctx, canonical, Some(canonical_to))?;
        if let (Some(a), Some(i)) = (assignment, root_ref) {
            refs.remove(i);
            write_json(store, &manifest(course, a, "references"), &json!(refs))?;
        }
        show_created(store, course, assignment, destination)?;
        let order = arr(&read_json(
            store,
            &order_path(course, assignment),
            json!([]),
        )?);
        write_json(
            store,
            &order_path(course, assignment),
            &json!(
                order
                    .iter()
                    .map(|p| {
                        let p = p.as_str().unwrap_or("");
                        if contains(path, p) {
                            format!("{destination}{}", &p[path.len()..])
                        } else {
                            p.into()
                        }
                    })
                    .collect::<Vec<String>>()
            ),
        )?;
        append_memory(
            store,
            course,
            "Moved or renamed path",
            args,
            &[canonical.into(), canonical_to.into()],
        )?;
        checkpoint_assignments(store, state, course, Some(&ctx))?;
        return tree(store, course, assignment, &json!({}));
    }
    Err(format!("Unknown tool: {name}"))
}
fn open_explorer(store: &Store, course: &str, a: Option<&str>, args: &Value) -> Result<Value> {
    let path = arg(args, "path")?;
    valid_path(path)?;
    let resolved = if args["stepId"].is_string() {
        timeline(
            store,
            course,
            a.ok_or("assignmentId is required")?,
            "read_assignment_snapshot",
            &{
                let mut v = args.clone();
                v["mode"] = json!("file");
                v
            },
        )?
    } else {
        let directory = path.rsplit_once('/').map(|v| v.0).unwrap_or("");
        let listing = tree(
            store,
            course,
            a,
            &json!({"directory":directory,"shallow":true}),
        )?;
        if !arr(&listing["entries"])
            .iter()
            .any(|e| s(e, "path") == path)
        {
            return Err("File not found in this workspace view.".into());
        }
        resolve(store, course, a, path, true)?
    };
    let root = store.path(&base(course, None))?;
    let mut target = PathBuf::from(arg(&resolved, "absolutePath")?);
    if !target.starts_with(&root)
        || excluded(
            &target
                .strip_prefix(&root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/"),
        )
    {
        return Err("Path is outside the available course files.".into());
    }
    let mut missing = false;
    while !target.exists() {
        if target == root || !target.pop() {
            return Err("Course files not found".into());
        }
        missing = true;
    }
    let mut command;
    let wait;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command = std::process::Command::new("explorer.exe");
        wait = false;
        command.creation_flags(0x08000000);
        if !target.is_dir() {
            command.arg("/select,");
        }
        command.arg(&target);
    }
    #[cfg(target_os = "macos")]
    {
        command = std::process::Command::new("/usr/bin/open");
        wait = true;
        if !target.is_dir() {
            command.arg("-R");
        }
        command.arg(&target);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let wsl = std::env::var_os("WSL_DISTRO_NAME").is_some()
            || fs::read_to_string("/proc/sys/kernel/osrelease")
                .is_ok_and(|release| release.to_ascii_lowercase().contains("microsoft"));
        wait = !wsl;
        if wsl {
            let output = std::process::Command::new("wslpath")
                .arg("-w")
                .arg(&target)
                .output()
                .map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err("Cannot convert this file path for Windows Explorer".into());
            }
            let filename = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
            command = std::process::Command::new("explorer.exe");
            if !target.is_dir() {
                command.arg("/select,");
            }
            command.arg(filename.trim());
        } else {
            command = std::process::Command::new("xdg-open");
            command.arg(if target.is_dir() {
                target.as_path()
            } else {
                target.parent().ok_or("Invalid file path")?
            });
        }
    }
    launch_explorer(&mut command, wait, std::time::Duration::from_secs(8))?;
    Ok(json!({"missing":missing}))
}

fn launch_explorer(
    command: &mut std::process::Command,
    wait: bool,
    timeout: std::time::Duration,
) -> Result<()> {
    let failure = |message: String| {
        format!(
            "Could not open the file explorer on the daemon's computer. Ensure a desktop file manager is available. {message}"
        )
    };
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| failure(e.to_string()))?;
    if !wait {
        // Explorer can hand off to an existing window and exit nonzero, or keep running.
        // Reap the launcher independently without delaying the browser response.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return Ok(());
    }
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(failure(format!("File explorer exited with {status}.")))
                };
            }
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            outcome => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(failure(match outcome {
                    Err(error) => error.to_string(),
                    _ => format!(
                        "File explorer did not respond within {} seconds.",
                        timeout.as_secs_f64()
                    ),
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn explorer_launch_reports_failure_timeout_and_preserves_detached_handoff() {
        use std::{process::Command, time::Duration};
        let mut success = Command::new("/bin/sh");
        success.args(["-c", "exit 0"]);
        launch_explorer(&mut success, true, Duration::from_secs(1)).unwrap();
        let mut failure = Command::new("/bin/sh");
        failure.args(["-c", "exit 7"]);
        assert!(
            launch_explorer(&mut failure, true, Duration::from_secs(1))
                .unwrap_err()
                .contains("exit status: 7")
        );
        let mut timeout = Command::new("/bin/sh");
        timeout.args(["-c", "exec sleep 30"]);
        assert!(
            launch_explorer(&mut timeout, true, Duration::from_millis(30))
                .unwrap_err()
                .contains("did not respond")
        );
        let mut handoff = Command::new("/bin/sh");
        handoff.args(["-c", "exit 7"]);
        launch_explorer(&mut handoff, false, Duration::from_secs(1)).unwrap();
    }
    struct Fixture(Store);
    impl Fixture {
        fn new() -> Self {
            let s = Store {
                root: std::env::temp_dir().join(format!("cruise-files-{}", id())),
            };
            s.mutate(|v| {
                v["courses"] = json!([{"id":"course","code":"TEST","name":"Test"}]);
                Ok(())
            })
            .unwrap();
            Self(s)
        }
        fn call(&self, name: &str, args: Value) -> Value {
            call(&self.0, name, args).unwrap()
        }
        fn assignment(&self) -> String {
            self.call(
                "create_assignment",
                json!({"courseId":"course","title":"Test assignment"}),
            )["id"]
                .as_str()
                .unwrap()
                .into()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert!(self.0.root.starts_with(std::env::temp_dir()));
            let _ = fs::remove_dir_all(&self.0.root);
        }
    }
    const WHY: &str = "Implement the verified course requirement.";
    #[test]
    fn missing_shared_roots_can_be_detached_without_deleting_the_original() {
        let f = Fixture::new();
        let a = f.assignment();
        let created = f.call("create_course_file", json!({"courseId":"course","path":"shared.txt","content":"keep this","explanation":WHY}));
        f.call("reference_course_path", json!({"courseId":"course","assignmentId":a,"path":"alias.txt","targetPath":"shared.txt"}));
        f.call("move_course_path", json!({"courseId":"course","path":"shared.txt","destination":"renamed.txt","expectedRevision":created["revision"],"explanation":WHY}));
        let listing = f.call(
            "list_assignment_files",
            json!({"courseId":"course","assignmentId":a}),
        );
        assert_eq!(listing["entries"][0]["shared"]["missing"], true);
        let detached = f.call("delete_assignment_path", json!({"courseId":"course","assignmentId":a,"path":"alias.txt","expectedRevision":listing["entries"][0]["revision"],"explanation":WHY}));
        assert_eq!(detached["detached"], true);
        assert_eq!(
            f.call(
                "read_course_file",
                json!({"courseId":"course","path":"renamed.txt"})
            )["content"],
            "keep this"
        );
    }
    #[test]
    fn shared_edits_history_revisions_moves_and_detach() {
        let f = Fixture::new();
        let a = f.assignment();
        let created=f.call("create_course_file",json!({"courseId":"course","path":"src/main.py","content":"x = 1\nprint(x)\n","explanation":WHY}));
        f.call(
            "reference_course_path",
            json!({"courseId":"course","assignmentId":a,"path":"code","targetPath":"src"}),
        );
        let updated=f.call("edit_assignment_file",json!({"courseId":"course","assignmentId":a,"path":"code/main.py","expectedRevision":created["revision"],"edits":[{"oldText":"x = 1","newText":"x = 2"}],"explanation":WHY,"context":"Verified rubric"}));
        assert!(call(&f.0,"write_course_file",json!({"courseId":"course","path":"src/main.py","expectedRevision":created["revision"],"content":"stale","explanation":WHY})).is_err());
        let h = f.call(
            "get_file_history",
            json!({"courseId":"course","assignmentId":a,"path":"code/main.py"}),
        );
        assert_eq!(h["total"], 2);
        assert_eq!(h["entries"][0]["assignmentId"], a);
        let d=f.call("get_file_history",json!({"courseId":"course","historyId":h["historyId"],"revisionId":h["entries"][0]["id"]}));
        assert!(
            arr(&d["diff"]["lines"])
                .iter()
                .any(|v| v["kind"] == "remove" && v["text"] == "x = 1\n")
        );
        f.call("delete_assignment_path",json!({"courseId":"course","assignmentId":a,"path":"code/main.py","expectedRevision":updated["revision"],"explanation":WHY}));
        assert_eq!(
            f.call(
                "read_course_file",
                json!({"courseId":"course","path":"src/main.py"})
            )["content"],
            "x = 2\nprint(x)\n"
        );
        f.call("move_course_path",json!({"courseId":"course","path":"src/main.py","destination":"moved.py","expectedRevision":updated["revision"],"explanation":WHY}));
        let moved = f.call(
            "get_file_history",
            json!({"courseId":"course","path":"moved.py"}),
        );
        assert_eq!(moved["historyId"], h["historyId"]);
        assert_eq!(moved["entries"][0]["action"], "moved");
        f.call("create_course_file",json!({"courseId":"course","path":"src/main.py","content":"new identity","explanation":WHY}));
        assert_ne!(
            f.call(
                "get_file_history",
                json!({"courseId":"course","path":"src/main.py"})
            )["historyId"],
            h["historyId"]
        );
    }
    #[test]
    fn timeline_keeps_original_snapshots_and_student_parts() {
        let f = Fixture::new();
        let a = f.assignment();
        let plan=f.call("record_assignment_learning",json!({"courseId":"course","assignmentId":a,"title":"Plan","markdown":"Start at zero before processing the input.","part":{"order":1,"title":"Total"},"phase":"plan","gaps":["No instructor evidence in fixture"]}));
        let c=f.call("create_assignment_file",json!({"courseId":"course","assignmentId":a,"path":"sum.py","content":"total = 0\n","explanation":WHY}));
        let steps = f.call(
            "get_assignment_timeline",
            json!({"courseId":"course","assignmentId":a}),
        );
        assert_eq!(steps["total"], 2);
        let saved = steps["steps"][1]["id"].clone();
        f.call("write_assignment_file",json!({"courseId":"course","assignmentId":a,"path":"sum.py","content":"total = 5\n","expectedRevision":c["revision"],"explanation":WHY}));
        assert_eq!(
            f.call(
                "get_assignment_timeline",
                json!({"courseId":"course","assignmentId":a,"stepId":saved,"path":"sum.py"})
            )["content"],
            "total = 0\n"
        );
        assert!(call(&f.0,"record_assignment_learning",json!({"courseId":"course","assignmentId":a,"title":"Next","markdown":"Move onto the next assignment component.","part":{"order":2,"title":"Loop"},"phase":"plan","gaps":["No evidence"]})).is_err());
        let lesson = "A running total starts at zero because no values have been seen. Add each value to retain the sum of everything seen so far. For example, processing two and then three gives zero plus two equals two, followed by two plus three equals five. Empty input preserves zero.";
        f.call("record_assignment_learning",json!({"courseId":"course","assignmentId":a,"title":"Complete","markdown":"The initial total was checked against the input.","teachingMarkdown":lesson,"part":{"order":1,"title":"Total"},"phase":"completed","gaps":["No evidence"]}));
        let p = f.call(
            "get_assignment_parts",
            json!({"courseId":"course","assignmentId":a}),
        );
        assert_eq!(arr(&p["parts"]).len(), 1);
        assert_eq!(p["parts"][0]["teachingMarkdown"], lesson);
        assert_eq!(
            f.call(
                "get_assignment_timeline",
                json!({"courseId":"course","assignmentId":a,"stepId":plan["stepId"]})
            )["step"]["files"],
            json!([])
        );
    }
    #[test]
    fn path_binary_and_snapshot_integrity_guards() {
        let f = Fixture::new();
        for path in [
            "../state.json",
            "con.txt",
            "x/.env",
            "x/auth.json",
            "foo\\bar",
        ] {
            assert!(
                call(
                    &f.0,
                    "create_course_file",
                    json!({"courseId":"course","path":path,"content":"bad","explanation":WHY})
                )
                .is_err()
            );
        }
        f.call("create_course_file",json!({"courseId":"course","path":"blob.bin","content":"AP8R","encoding":"base64","explanation":WHY}));
        let h = f.call(
            "get_file_history",
            json!({"courseId":"course","path":"blob.bin"}),
        );
        let args = json!({"courseId":"course","historyId":h["historyId"],"revisionId":h["entries"][0]["id"],"side":"after"});
        assert_eq!(
            f.call("download_file_revision", args.clone())["bytes"],
            "AP8R"
        );
        f.0.write(
            &format!(
                "courses/course/history/blobs/{}",
                s(&h["entries"][0]["after"], "hash")
            ),
            b"tampered",
        )
        .unwrap();
        assert!(
            call(&f.0, "download_file_revision", args)
                .unwrap_err()
                .contains("integrity")
        );
    }
    #[test]
    fn diff_keeps_line_endings_and_bounds_memory() {
        let d = diff(Some(b"first\r\nlast"), Some(b"first\r\nnew\nlast\n"));
        assert!(
            arr(&d["lines"])
                .iter()
                .any(|v| v["kind"] == "context" && v["text"] == "first\r\n")
        );
        assert!(
            arr(&d["lines"])
                .iter()
                .any(|v| v["kind"] == "remove" && v["text"] == "last")
        );
        assert_eq!(
            diff(
                Some("a\n".repeat(1500).as_bytes()),
                Some("b\n".repeat(1500).as_bytes())
            )["coarse"],
            true
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape_a_reference() {
        let f = Fixture::new();
        let a = f.assignment();
        f.call(
            "create_course_directory",
            json!({"courseId":"course","path":"shared"}),
        );
        f.call(
            "reference_course_path",
            json!({"courseId":"course","assignmentId":a,"path":"alias","targetPath":"shared"}),
        );
        std::os::unix::fs::symlink(
            std::env::temp_dir(),
            f.0.root.join("courses/course/files/shared/outside"),
        )
        .unwrap();
        assert!(call(&f.0,"create_assignment_file",json!({"courseId":"course","assignmentId":a,"path":"alias/outside/escape","content":"bad","explanation":WHY})).is_err());
    }
}

#[cfg(test)]
mod legacy_format_tests {
    use super::*;
    // Captured by the former TypeScript engine; exercise native reads and edits
    // against its original state, shared reference, timeline, and history bytes.
    const FIXTURE: &str = r###"{"courseId":"07265115-8a7a-4518-8302-748fa8a60b26","assignmentId":"95430c9c-e496-45ae-9dbd-bc815434b39d","stepId":"85bd9c7b-fb05-40b8-bb57-f0f9ca6526b3","historyId":"f947892b-a375-4fe1-a9dd-c6e125afbd76","files":{"state.json":"{\n  \"version\": 1,\n  \"courses\": [\n    {\n      \"code\": \"PARITY\",\n      \"name\": \"Existing TypeScript workspace\",\n      \"term\": \"\",\n      \"color\": \"green\",\n      \"canvasUrl\": \"\",\n      \"websiteUrl\": \"\",\n      \"id\": \"07265115-8a7a-4518-8302-748fa8a60b26\",\n      \"createdAt\": \"2026-10-02T11:57:35.174Z\"\n    }\n  ],\n  \"lectures\": [],\n  \"tasks\": [],\n  \"concepts\": [],\n  \"jobs\": [],\n  \"settings\": {\n    \"notionDataSourceId\": \"\",\n    \"notionTitleProperty\": \"Name\",\n    \"notionCourseProperty\": \"Course\",\n    \"notionDateProperty\": \"Due\",\n    \"notionStatusProperty\": \"Status\",\n    \"notionDoneValues\": \"Done,Complete,Completed\",\n    \"canvasBaseUrl\": \"\",\n    \"autoSync\": false\n  },\n  \"sync\": {\n    \"errors\": []\n  },\n  \"assignments\": [\n    {\n      \"courseId\": \"07265115-8a7a-4518-8302-748fa8a60b26\",\n      \"title\": \"Saved assignment\",\n      \"description\": \"\",\n      \"id\": \"95430c9c-e496-45ae-9dbd-bc815434b39d\",\n      \"createdAt\": \"2026-10-02T11:57:35.180Z\"\n    }\n  ]\n}","courses/07265115-8a7a-4518-8302-748fa8a60b26/agent/assignments/95430c9c-e496-45ae-9dbd-bc815434b39d-references.json":"[\n  {\n    \"path\": \"alias.txt\",\n    \"targetPath\": \"shared.txt\",\n    \"type\": \"file\",\n    \"createdAt\": \"2026-10-02T11:57:35.205Z\",\n    \"excludedPaths\": []\n  }\n]","courses/07265115-8a7a-4518-8302-748fa8a60b26/agent/assignments/95430c9c-e496-45ae-9dbd-bc815434b39d-timeline.json":"[\n  {\n    \"title\": \"reference path\",\n    \"markdown\": \"Display files/shared.txt as alias.txt; reuse the shared original.\",\n    \"actor\": \"agent\",\n    \"id\": \"1d439d3e-fe47-4173-93b6-6f8dc36484ac\",\n    \"timestamp\": \"2026-10-02T11:57:35.214Z\",\n    \"files\": [\n      {\n        \"path\": \"alias.txt\",\n        \"type\": \"file\",\n        \"originalPath\": \"files/shared.txt\",\n        \"missing\": false,\n        \"snapshot\": {\n          \"hash\": \"1e9f727269d227b686efc5e0f4318acf8928de764da48937d67a45fc35cca895\",\n          \"size\": 25,\n          \"retained\": true\n        }\n      }\n    ],\n    \"changes\": [\n      {\n        \"path\": \"alias.txt\",\n        \"action\": \"added\"\n      }\n    ]\n  },\n  {\n    \"title\": \"Existing checkpoint\",\n    \"markdown\": \"Inspect the existing saved assignment and its exact original file.\",\n    \"actor\": \"agent\",\n    \"part\": {\n      \"order\": 1,\n      \"title\": \"Existing part\"\n    },\n    \"phase\": \"plan\",\n    \"nextAction\": \"\",\n    \"memoryPath\": \"courses/07265115-8a7a-4518-8302-748fa8a60b26/memory/assignments/0b08711f-4064-4efa-b083-3569ba0f0d48.md\",\n    \"sourceIds\": [],\n    \"gaps\": [\n      \"Fixture contains no primary instructor evidence.\"\n    ],\n    \"id\": \"85bd9c7b-fb05-40b8-bb57-f0f9ca6526b3\",\n    \"timestamp\": \"2026-10-02T11:57:35.236Z\",\n    \"files\": [\n      {\n        \"path\": \"alias.txt\",\n        \"type\": \"file\",\n        \"originalPath\": \"files/shared.txt\",\n        \"missing\": false,\n        \"snapshot\": {\n          \"hash\": \"1e9f727269d227b686efc5e0f4318acf8928de764da48937d67a45fc35cca895\",\n          \"size\": 25,\n          \"retained\": true\n        }\n      }\n    ],\n    \"changes\": []\n  },\n  {\n    \"title\": \"write file\",\n    \"markdown\": \"Update through the existing shared reference.\",\n    \"actor\": \"agent\",\n    \"part\": {\n      \"order\": 1,\n      \"title\": \"Existing part\"\n    },\n    \"id\": \"9c4a8423-4bb1-49cb-84b6-686494d81134\",\n    \"timestamp\": \"2026-10-02T11:57:35.276Z\",\n    \"files\": [\n      {\n        \"path\": \"alias.txt\",\n        \"type\": \"file\",\n        \"originalPath\": \"files/shared.txt\",\n        \"missing\": false,\n        \"snapshot\": {\n          \"hash\": \"651bb162bfe1e6d19ae81d9ee7e4ccbe4ca186bcce9563ba974b45d450c7a09c\",\n          \"size\": 24,\n          \"retained\": true\n        }\n      }\n    ],\n    \"changes\": [\n      {\n        \"path\": \"alias.txt\",\n        \"action\": \"changed\"\n      }\n    ]\n  }\n]","courses/07265115-8a7a-4518-8302-748fa8a60b26/history/index.json":"{\n  \"paths\": {\n    \"files/shared.txt\": \"f947892b-a375-4fe1-a9dd-c6e125afbd76\"\n  },\n  \"archived\": {},\n  \"ids\": [\n    \"f947892b-a375-4fe1-a9dd-c6e125afbd76\"\n  ]\n}","courses/07265115-8a7a-4518-8302-748fa8a60b26/history/files/f947892b-a375-4fe1-a9dd-c6e125afbd76.json":"[\n  {\n    \"actor\": \"agent\",\n    \"explanation\": \"Create an existing workspace fixture.\",\n    \"tool\": \"create_file\",\n    \"id\": \"8019b798-ad60-4328-9110-5c121deec497\",\n    \"sequence\": 1,\n    \"timestamp\": \"2026-10-02T11:57:35.193Z\",\n    \"action\": \"created\",\n    \"path\": \"files/shared.txt\",\n    \"before\": null,\n    \"after\": {\n      \"hash\": \"1e9f727269d227b686efc5e0f4318acf8928de764da48937d67a45fc35cca895\",\n      \"size\": 25,\n      \"retained\": true\n    }\n  },\n  {\n    \"actor\": \"agent\",\n    \"explanation\": \"Update through the existing shared reference.\",\n    \"assignmentId\": \"95430c9c-e496-45ae-9dbd-bc815434b39d\",\n    \"tool\": \"write_file\",\n    \"id\": \"42a0d56e-f4ac-4af0-9f76-2d612c3e33c2\",\n    \"sequence\": 2,\n    \"timestamp\": \"2026-10-02T11:57:35.265Z\",\n    \"action\": \"updated\",\n    \"path\": \"files/shared.txt\",\n    \"before\": {\n      \"hash\": \"1e9f727269d227b686efc5e0f4318acf8928de764da48937d67a45fc35cca895\",\n      \"size\": 25,\n      \"retained\": true\n    },\n    \"after\": {\n      \"hash\": \"651bb162bfe1e6d19ae81d9ee7e4ccbe4ca186bcce9563ba974b45d450c7a09c\",\n      \"size\": 24,\n      \"retained\": true\n    }\n  }\n]","courses/07265115-8a7a-4518-8302-748fa8a60b26/files/shared.txt":"updated from TypeScript\n","courses/07265115-8a7a-4518-8302-748fa8a60b26/history/blobs/1e9f727269d227b686efc5e0f4318acf8928de764da48937d67a45fc35cca895":"original from TypeScript\n","courses/07265115-8a7a-4518-8302-748fa8a60b26/history/blobs/651bb162bfe1e6d19ae81d9ee7e4ccbe4ca186bcce9563ba974b45d450c7a09c":"updated from TypeScript\n"}}"###;
    #[test]
    fn reads_and_extends_an_existing_typescript_workspace_without_losing_history() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let root = std::env::temp_dir().join(format!("cruise-ts-fixture-{}", id()));
        let store = Store { root: root.clone() };
        for (path, content) in fixture["files"].as_object().unwrap() {
            store
                .write(path, content.as_str().unwrap().as_bytes())
                .unwrap();
        }
        let course = &fixture["courseId"];
        let assignment = &fixture["assignmentId"];
        let current = call(
            &store,
            "read_assignment_file",
            json!({"courseId":course,"assignmentId":assignment,"path":"alias.txt"}),
        )
        .unwrap();
        assert_eq!(current["content"], "updated from TypeScript\n");
        let saved = call(&store, "get_assignment_timeline", json!({"courseId":course,"assignmentId":assignment,"stepId":fixture["stepId"],"path":"alias.txt"})).unwrap();
        assert_eq!(saved["content"], "original from TypeScript\n");
        let history = call(
            &store,
            "get_file_history",
            json!({"courseId":course,"assignmentId":assignment,"path":"alias.txt"}),
        )
        .unwrap();
        assert_eq!(history["historyId"], fixture["historyId"]);
        assert_eq!(history["total"], 2);
        call(&store, "write_assignment_file", json!({"courseId":course,"assignmentId":assignment,"path":"alias.txt","expectedRevision":current["revision"],"content":"extended in Rust\n","explanation":"Preserve existing TypeScript history while extending its contents."})).unwrap();
        let extended = call(
            &store,
            "get_file_history",
            json!({"courseId":course,"path":"shared.txt"}),
        )
        .unwrap();
        assert_eq!(extended["historyId"], fixture["historyId"]);
        assert_eq!(extended["total"], 3);
        assert_eq!(extended["entries"][1], history["entries"][0]);
        assert_eq!(extended["entries"][2], history["entries"][1]);
        let old_again = call(&store, "get_assignment_timeline", json!({"courseId":course,"assignmentId":assignment,"stepId":fixture["stepId"],"path":"alias.txt"})).unwrap();
        assert_eq!(old_again, saved);
        assert!(root.starts_with(std::env::temp_dir()));
        fs::remove_dir_all(root).unwrap();
    }
}
