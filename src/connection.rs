//! Installation-local credentials, never stored in the course content repository.
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub fn home() -> PathBuf {
    env::var_os("CRUISE_HOME")
        .or_else(|| env::var_os("COURSE_CAPTAIN_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

pub fn key_path() -> PathBuf {
    home().join("workspace/connection-key")
}

pub fn read(path: &Path) -> io::Result<String> {
    let key = fs::read_to_string(path)?;
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::other(
            "Invalid connection key file; run cruise reset-connection-key",
        ));
    }
    Ok(key)
}

pub fn load_or_create(path: &Path) -> io::Result<String> {
    match read(path) {
        Ok(key) => Ok(key),
        Err(e) if e.kind() == io::ErrorKind::NotFound => save(path, false),
        Err(e) => Err(e),
    }
}

pub fn reset(path: &Path) -> io::Result<String> {
    save(path, true)
}

fn save(path: &Path, replace: bool) -> io::Result<String> {
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(io::Error::other)?;
    let key: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| io::Error::other("Missing key directory"))?,
    )?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(key.as_bytes())?;
        file.sync_all()?;
        drop(file);
        if replace {
            fs::rename(&temporary, path)?;
        } else {
            // Publish a complete key without overwriting a concurrent first start.
            match fs::hard_link(&temporary, path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return read(path),
                Err(e) => return Err(e),
            }
        }
        Ok(key)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_persists_until_reset_and_corruption_fails_closed() {
        let root = env::temp_dir().join(format!("cruise-key-{}", uuid::Uuid::new_v4()));
        let path = root.join("connection-key");
        let first = load_or_create(&path).unwrap();
        assert_eq!(first, load_or_create(&path).unwrap());
        let second = reset(&path).unwrap();
        assert_ne!(first, second);
        assert_eq!(second, load_or_create(&path).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::write(&path, "broken").unwrap();
        assert!(load_or_create(&path).is_err());
        reset(&path).unwrap();
        assert!(read(&path).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
}
