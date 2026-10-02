use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn arg<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key}: expected a string"))
}
pub fn valid_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 100
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        Err("Invalid ID".into())
    } else {
        Ok(())
    }
}
pub fn require_course(state: &Value, id: &str) -> Result<Value> {
    valid_id(id)?;
    state["courses"]
        .as_array()
        .and_then(|a| a.iter().find(|c| c["id"] == id))
        .cloned()
        .ok_or_else(|| "Course not found".into())
}
pub fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}
pub fn array_mut<'a>(v: &'a mut Value, key: &str) -> Result<&'a mut Vec<Value>> {
    if v.get(key).is_none() {
        v[key] = json!([]);
    }
    v[key]
        .as_array_mut()
        .ok_or_else(|| format!("Invalid workspace array: {key}"))
}
pub fn empty_state() -> Value {
    json!({"version":1,"courses":[],"lectures":[],"tasks":[],"concepts":[],"jobs":[],"settings":{"notionDataSourceId":"","notionTitleProperty":"Name","notionCourseProperty":"Course","notionDateProperty":"Due","notionStatusProperty":"Status","notionDoneValues":"Done,Complete,Completed","canvasBaseUrl":"","autoSync":false},"sync":{"errors":[]}})
}
pub fn no_symlink(path: &Path) -> Result<()> {
    let mut p = PathBuf::new();
    for c in path.components() {
        p.push(c);
        match fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err("Symlinks are not allowed in workspace paths".into());
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        }
    }
    Ok(())
}
struct Lock {
    path: PathBuf,
    stop: std::sync::mpsc::Sender<()>,
    heartbeat: Option<std::thread::JoinHandle<()>>,
    healthy: std::sync::Arc<std::sync::atomic::AtomicBool>,
    identity: fs::Metadata,
}
fn same_directory(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        a.created().ok() == b.created().ok()
    }
}
impl Lock {
    fn acquired(path: PathBuf) -> Result<Self> {
        let identity = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        let expected = identity.clone();
        let thread_path = path.clone();
        let healthy = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let status = healthy.clone();
        let (stop, receiver) = std::sync::mpsc::channel();
        let heartbeat = std::thread::spawn(move || {
            while receiver.recv_timeout(Duration::from_secs(5))
                == Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            {
                if !fs::symlink_metadata(&thread_path).is_ok_and(|m| same_directory(&m, &expected))
                    || filetime::set_file_mtime(&thread_path, filetime::FileTime::now()).is_err()
                {
                    status.store(false, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
            }
        });
        Ok(Self {
            path,
            stop,
            heartbeat: Some(heartbeat),
            healthy,
            identity,
        })
    }
    fn check(&self) -> Result<()> {
        if self.healthy.load(std::sync::atomic::Ordering::SeqCst)
            && fs::symlink_metadata(&self.path).is_ok_and(|m| same_directory(&m, &self.identity))
        {
            Ok(())
        } else {
            Err("Workspace lock was lost during the operation".into())
        }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.heartbeat.take() {
            let _ = thread.join();
        }
        if self.check().is_ok() {
            let _ = fs::remove_dir(&self.path);
        }
    }
}
impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: if root.is_absolute() {
                root
            } else {
                std::env::current_dir().unwrap_or_default().join(root)
            },
        }
    }
    pub fn path(&self, relative: &str) -> Result<PathBuf> {
        if relative.is_empty()
            || relative.contains('\\')
            || relative.contains(':')
            || relative.contains('\0')
            || Path::new(relative)
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("Path is outside the course workspace".into());
        }
        let p = self.root.join(relative);
        no_symlink(&p)?;
        Ok(p)
    }
    pub fn write(&self, relative: &str, data: &[u8]) -> Result<()> {
        let p = self.path(relative)?;
        fs::create_dir_all(p.parent().ok_or("Invalid file path")?).map_err(|e| e.to_string())?;
        no_symlink(&p)?;
        let tmp = p.with_file_name(format!(
            "{}.{}.tmp",
            p.file_name().unwrap().to_string_lossy(),
            id()
        ));
        let result = (|| {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp).map_err(|e| e.to_string())?;
            file.write_all(data).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            fs::rename(&tmp, &p).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        result
    }
    pub fn read(&self) -> Result<Value> {
        no_symlink(&self.root)?;
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        match fs::read(self.path("state.json")?) {
            Ok(bytes) => {
                let state: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if state["version"] != 1 {
                    return Err(
                        "Unsupported workspace version. Back up your data before upgrading.".into(),
                    );
                }
                for key in ["courses", "lectures", "tasks", "concepts", "jobs"] {
                    if !state[key].is_array() {
                        return Err(format!("Invalid workspace array: {key}"));
                    }
                }
                Ok(state)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(empty_state()),
            Err(e) => Err(e.to_string()),
        }
    }
    pub fn mutate<T>(&self, f: impl FnOnce(&mut Value) -> Result<T>) -> Result<T> {
        no_symlink(&self.root)?;
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        // Match the previous engine's atomic directory lock, including its location.
        let lock = self.root.with_file_name(format!(
            "{}.lock",
            self.root
                .file_name()
                .ok_or("Invalid workspace root")?
                .to_string_lossy()
        ));
        let start = Instant::now();
        loop {
            match fs::create_dir(&lock) {
                Ok(()) => break,
                Err(e)
                    if e.kind() == std::io::ErrorKind::AlreadyExists
                        && start.elapsed() < Duration::from_secs(15) =>
                {
                    no_symlink(&lock)?;
                    if fs::metadata(&lock)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > Duration::from_secs(30))
                    {
                        let _ = fs::remove_dir(&lock);
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
                Err(e) => return Err(format!("Workspace is locked or unavailable: {e}")),
            }
        }
        let lock = Lock::acquired(lock)?;
        let mut state = self.read()?;
        let value = f(&mut state)?;
        lock.check()?;
        self.write(
            "state.json",
            &serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?,
        )?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_legacy_directory_locks_can_be_recovered() {
        let root = std::env::temp_dir().join(format!("cruise-stale-{}", id()));
        let lock = root.with_file_name(format!(
            "{}.lock",
            root.file_name().unwrap().to_string_lossy()
        ));
        fs::create_dir(&lock).unwrap();
        filetime::set_file_mtime(
            &lock,
            filetime::FileTime::from_system_time(
                std::time::SystemTime::now() - Duration::from_secs(60),
            ),
        )
        .unwrap();
        let store = Store::new(root.clone());
        store
            .mutate(|s| {
                s["restored"] = json!(true);
                Ok(())
            })
            .unwrap();
        assert!(!lock.exists());
        assert_eq!(store.read().unwrap()["restored"], true);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn atomic_mutations_preserve_unknown_fields_and_reject_bad_paths() {
        let root = std::env::temp_dir().join(format!("cruise-store-{}", id()));
        let s = Store::new(root.clone());
        s.mutate(|v| {
            v["futureField"] = json!({"preserved":true});
            Ok(())
        })
        .unwrap();
        let mut workers = Vec::new();
        for _ in 0..8 {
            let s = s.clone();
            workers.push(std::thread::spawn(move || {
                s.mutate(|v| {
                    let n = v["counter"].as_u64().unwrap_or(0);
                    v["counter"] = json!(n + 1);
                    Ok(())
                })
                .unwrap()
            }));
        }
        for w in workers {
            w.join().unwrap();
        }
        let state = s.read().unwrap();
        assert_eq!(state["counter"], 8);
        assert_eq!(state["futureField"]["preserved"], true);
        for path in [
            "../escape",
            "/etc/passwd",
            "x/../escape",
            "C:/outside",
            "x\\outside",
        ] {
            assert!(s.path(path).is_err());
        }
        s.write("state.json", b"{\"version\":2}").unwrap();
        assert!(s.read().is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlinks() {
        let root = std::env::temp_dir().join(format!("cruise-symlink-{}", id()));
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(std::env::temp_dir(), root.join("outside")).unwrap();
        let s = Store::new(root.clone());
        assert!(s.write("outside/test", b"no").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
