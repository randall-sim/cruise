//! Authenticated capture bytes stay out of the serial course-engine transport.
use axum::{
    body::Body,
    extract::Query,
    http::{HeaderValue, StatusCode, Uri},
    response::Response,
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs::Metadata,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};
use tokio::{fs, sync::Mutex};

type AssetResult<T> = Result<T, (StatusCode, String)>;

#[derive(Default)]
pub struct Assets {
    index: Mutex<Option<Index>>,
}

struct Index {
    revision: Revision,
    captures: HashMap<String, Capture>,
}

#[derive(PartialEq, Eq)]
struct Revision {
    // Canonical writes replace state.json; external in-place edits must change metadata.
    len: u64,
    modified: Option<SystemTime>,
    // Atomic state replacement can preserve both length and modification time.
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl From<Metadata> for Revision {
    fn from(metadata: Metadata) -> Self {
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                )
            },
        }
    }
}

// Serde skips lecture text, transcripts and jobs; retain only the asset lookup.
#[derive(Deserialize)]
struct Workspace {
    version: u32,
    lectures: Vec<Lecture>,
}

#[derive(Deserialize)]
struct Lecture {
    captures: Vec<Capture>,
}

#[derive(Deserialize)]
struct Capture {
    id: String,
    file: String,
    #[serde(default)]
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Artifact {
    id: String,
    file: String,
    format: String,
    definition_path: Option<String>,
}

impl Assets {
    /// None means this is not a capture route. Caller must enforce authorization.
    pub async fn serve(&self, workspace: &Path, uri: &Uri) -> Option<Response> {
        let route = uri.path().strip_prefix("/api/capture/")?;
        let mut parts = route.split('/');
        let capture_id = parts.next()?;
        let artifact_id = match (parts.next(), parts.next(), parts.next()) {
            (None, None, None) => None,
            (Some("artifacts"), Some(id), None) => Some(id),
            _ => return None,
        };
        let valid_id = |id: &str| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        };
        if !valid_id(capture_id) || artifact_id.is_some_and(|id| !valid_id(id)) {
            return None;
        }
        Some(
            match self.read(workspace, uri, capture_id, artifact_id).await {
                Ok((bytes, content_type)) => {
                    let mut response = Response::new(Body::from(bytes));
                    let headers = response.headers_mut();
                    headers.insert("content-type", HeaderValue::from_static(content_type));
                    headers.insert(
                        "cache-control",
                        HeaderValue::from_static("private, no-store"),
                    );
                    headers.insert(
                        "x-content-type-options",
                        HeaderValue::from_static("nosniff"),
                    );
                    headers.insert(
                        "content-security-policy",
                        HeaderValue::from_static("default-src 'none'; sandbox"),
                    );
                    response
                }
                Err((status, message)) => super::error(status, message),
            },
        )
    }

    async fn read(
        &self,
        workspace: &Path,
        uri: &Uri,
        capture_id: &str,
        artifact_id: Option<&str>,
    ) -> AssetResult<(Vec<u8>, &'static str)> {
        let query = Query::<HashMap<String, String>>::try_from_uri(uri)
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid image query".into()))?;
        let source = query.get("view").is_some_and(|value| value == "source");
        let (relative, content_type) = {
            let mut index = self.index.lock().await;
            let state_path = checked_path(workspace, "state.json").await?;
            let revision = revision(&state_path).await?;
            if index
                .as_ref()
                .is_none_or(|index| index.revision != revision)
            {
                *index = Some(load_index(&state_path).await?);
            }
            let capture = index
                .as_ref()
                .unwrap()
                .captures
                .get(capture_id)
                .ok_or_else(|| (StatusCode::NOT_FOUND, "Capture not found".into()))?;
            match artifact_id {
                None => (capture.file.clone(), "image/png"),
                Some(id) => {
                    let artifact = capture
                        .artifacts
                        .iter()
                        .find(|artifact| artifact.id == id)
                        .ok_or_else(|| {
                            (
                                StatusCode::NOT_FOUND,
                                "Image reconstruction not found".into(),
                            )
                        })?;
                    if source {
                        (
                            artifact.definition_path.clone().ok_or_else(|| {
                                (StatusCode::NOT_FOUND, "No diagram source".into())
                            })?,
                            "text/plain; charset=utf-8",
                        )
                    } else {
                        (
                            artifact.file.clone(),
                            if artifact.format == "diagram" {
                                "image/svg+xml"
                            } else {
                                "image/png"
                            },
                        )
                    }
                }
            }
        };
        // File reads run concurrently, outside both the index and engine locks.
        let path = checked_path(workspace, &relative).await?;
        let bytes = fs::read(path)
            .await
            .map_err(|error| io_error("Image file", error))?;
        Ok((bytes, content_type))
    }
}

async fn revision(path: &Path) -> AssetResult<Revision> {
    fs::metadata(path)
        .await
        .map(Revision::from)
        .map_err(|error| io_error("Image index", error))
}

async fn load_index(path: &Path) -> AssetResult<Index> {
    // Writers normally replace state.json atomically. Retry if it changes while read.
    for _ in 0..3 {
        let before = revision(path).await?;
        let bytes = fs::read(path)
            .await
            .map_err(|error| io_error("Image index", error))?;
        let after = revision(path).await?;
        if before != after {
            continue;
        }
        let captures = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let state: Workspace = serde_json::from_slice(&bytes)
                .map_err(|error| format!("Cannot read image index: {error}"))?;
            if state.version != 1 {
                return Err(
                    "Unsupported workspace version. Back up your data before upgrading.".into(),
                );
            }
            let mut captures = HashMap::new();
            for capture in state
                .lectures
                .into_iter()
                .flat_map(|lecture| lecture.captures)
            {
                captures.entry(capture.id.clone()).or_insert(capture);
            }
            Ok(captures)
        })
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Cannot load image index: {error}"),
            )
        })?
        .map_err(|message| (StatusCode::INTERNAL_SERVER_ERROR, message))?;
        return Ok(Index {
            revision: after,
            captures,
        });
    }
    Err((
        StatusCode::SERVICE_UNAVAILABLE,
        "Image index is changing; retry the image".into(),
    ))
}

async fn checked_path(workspace: &Path, relative: &str) -> AssetResult<PathBuf> {
    let path = Path::new(relative);
    if relative.is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err((
            StatusCode::FORBIDDEN,
            "Path is outside the course workspace".into(),
        ));
    }
    let mut current = workspace.to_path_buf();
    for part in std::iter::once(None).chain(path.components().map(Some)) {
        if let Some(part) = part {
            current.push(part);
        }
        let metadata = fs::symlink_metadata(&current)
            .await
            .map_err(|error| io_error("Image path", error))?;
        if metadata.file_type().is_symlink() {
            return Err((
                StatusCode::FORBIDDEN,
                "Symlinks are not allowed in workspace paths".into(),
            ));
        }
    }
    if !fs::metadata(&current)
        .await
        .map_err(|error| io_error("Image path", error))?
        .is_file()
    {
        return Err((StatusCode::NOT_FOUND, "Image file not found".into()));
    }
    Ok(current)
}

fn io_error(label: &str, error: std::io::Error) -> (StatusCode, String) {
    let status = match error.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NOT_FOUND,
        std::io::ErrorKind::PermissionDenied => StatusCode::FORBIDDEN,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, format!("{label}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::json;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let mut random = [0u8; 8];
            getrandom::fill(&mut random).unwrap();
            let path =
                std::env::temp_dir().join(format!("cc-assets-{:x}", u64::from_ne_bytes(random)));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn state(&self, file: &str) -> Vec<u8> {
            serde_json::to_vec(&json!({"version":1,"lectures":[{"captures":[{
                "id":"capture-1", "file":file, "artifacts":[
                    {"id":"diagram-1","file":"diagram.svg","format":"diagram","definitionPath":"diagram.json"},
                    {"id":"image-1","file":"capture.png","format":"image"}
                ]
            }]}]})).unwrap()
        }
        async fn write_state(&self, file: &str) {
            fs::write(self.0.join("state.json"), self.state(file))
                .await
                .unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn get(assets: &Assets, fixture: &Fixture, url: &str) -> Response {
        assets
            .serve(&fixture.0, &url.parse().unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn serves_exact_binary_svg_and_source_bytes_with_private_headers() {
        let fixture = Fixture::new();
        fixture.write_state("capture.png").await;
        let png = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j4S8AAAAASUVORK5CYII=").unwrap();
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\"><text>π → ∑</text></svg>".as_bytes();
        let source = br#"{"width":100,"height":100,"elements":[]}"#;
        for (file, bytes) in [
            ("capture.png", png.as_slice()),
            ("diagram.svg", svg),
            ("diagram.json", source.as_slice()),
        ] {
            fs::write(fixture.0.join(file), bytes).await.unwrap();
        }
        let assets = Assets::default();
        for (url, bytes, mime) in [
            ("/api/capture/capture-1", png.as_slice(), "image/png"),
            (
                "/api/capture/capture-1/artifacts/image-1",
                png.as_slice(),
                "image/png",
            ),
            (
                "/api/capture/capture-1/artifacts/diagram-1",
                svg,
                "image/svg+xml",
            ),
            (
                "/api/capture/capture-1/artifacts/diagram-1?view=source",
                source.as_slice(),
                "text/plain; charset=utf-8",
            ),
        ] {
            let response = get(&assets, &fixture, url).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], mime);
            assert_eq!(response.headers()["cache-control"], "private, no-store");
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            assert_eq!(
                response.headers()["content-security-policy"],
                "default-src 'none'; sandbox"
            );
            assert_eq!(
                to_bytes(response.into_body(), usize::MAX).await.unwrap(),
                bytes
            );
        }
    }

    #[tokio::test]
    async fn distinguishes_missing_files_unknown_ids_and_unsafe_paths() {
        let fixture = Fixture::new();
        let assets = Assets::default();
        fixture.write_state("missing.png").await;
        for url in [
            "/api/capture/capture-1",
            "/api/capture/unknown",
            "/api/capture/capture-1/artifacts/unknown",
            "/api/capture/capture-1/artifacts/image-1?view=source",
        ] {
            assert_eq!(
                get(&assets, &fixture, url).await.status(),
                StatusCode::NOT_FOUND
            );
        }
        for path in ["../outside.png", "/etc/passwd", "", "a/../../outside.png"] {
            fixture.write_state(path).await;
            assert_eq!(
                get(&assets, &fixture, "/api/capture/capture-1")
                    .await
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        for url in [
            "/api/workspace",
            "/api/capture/",
            "/api/capture/a/extra",
            "/api/capture/a/artifacts/b/extra",
            "/api/capture/%2e%2e",
        ] {
            assert!(
                assets
                    .serve(&fixture.0, &url.parse().unwrap())
                    .await
                    .is_none()
            );
        }
    }

    #[tokio::test]
    async fn refreshes_index_on_changes_and_does_not_cache_failed_file_reads() {
        let fixture = Fixture::new();
        let assets = Assets::default();
        fixture.write_state("first.png").await;
        assert_eq!(
            get(&assets, &fixture, "/api/capture/capture-1")
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        fs::write(fixture.0.join("first.png"), b"first")
            .await
            .unwrap();
        let response = get(&assets, &fixture, "/api/capture/capture-1").await;
        assert_eq!(
            to_bytes(response.into_body(), 100).await.unwrap(),
            b"first".as_slice()
        );
        fixture.write_state("second.png").await;
        fs::write(fixture.0.join("second.png"), b"other")
            .await
            .unwrap();
        let response = get(&assets, &fixture, "/api/capture/capture-1").await;
        assert_eq!(
            to_bytes(response.into_body(), 100).await.unwrap(),
            b"other".as_slice()
        );
        fs::remove_file(fixture.0.join("state.json")).await.unwrap();
        assert_eq!(
            get(&assets, &fixture, "/api/capture/capture-1")
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn detects_atomic_replacement_with_matching_size_and_modified_time() {
        let fixture = Fixture::new();
        let assets = Assets::default();
        fixture.write_state("first.png").await;
        fs::write(fixture.0.join("first.png"), b"first")
            .await
            .unwrap();
        fs::write(fixture.0.join("other.png"), b"other")
            .await
            .unwrap();
        assert_eq!(
            get(&assets, &fixture, "/api/capture/capture-1")
                .await
                .status(),
            StatusCode::OK
        );
        let old = std::fs::metadata(fixture.0.join("state.json")).unwrap();
        let replacement = fixture.0.join("replacement.json");
        fs::write(&replacement, fixture.state("other.png"))
            .await
            .unwrap();
        std::fs::File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old.modified().unwrap()))
            .unwrap();
        assert_eq!(old.len(), std::fs::metadata(&replacement).unwrap().len());
        fs::rename(replacement, fixture.0.join("state.json"))
            .await
            .unwrap();
        let response = get(&assets, &fixture, "/api/capture/capture-1").await;
        assert_eq!(
            to_bytes(response.into_body(), 100).await.unwrap(),
            b"other".as_slice()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_file_directory_state_and_workspace_symlinks() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let outside = Fixture::new();
        fs::write(outside.0.join("private.png"), b"secret")
            .await
            .unwrap();
        symlink(outside.0.join("private.png"), fixture.0.join("link.png")).unwrap();
        symlink(&outside.0, fixture.0.join("link-dir")).unwrap();
        let assets = Assets::default();
        for path in ["link.png", "link-dir/private.png"] {
            fixture.write_state(path).await;
            assert_eq!(
                get(&assets, &fixture, "/api/capture/capture-1")
                    .await
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        fs::remove_file(fixture.0.join("state.json")).await.unwrap();
        outside.write_state("private.png").await;
        symlink(outside.0.join("state.json"), fixture.0.join("state.json")).unwrap();
        assert_eq!(
            get(&assets, &fixture, "/api/capture/capture-1")
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        let response = assets
            .serve(
                &fixture.0.join("link-dir"),
                &"/api/capture/capture-1".parse().unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn capture_routes_require_auth_and_bypass_busy_engine() {
        use crate::{App, router};
        use axum::extract::Request;
        use std::{sync::Arc, time::Duration};
        use tower::ServiceExt;

        let fixture = Fixture::new();
        fixture.write_state("capture.png").await;
        for file in ["capture.png", "diagram.svg", "diagram.json"] {
            fs::write(fixture.0.join(file), b"asset").await.unwrap();
        }
        let app = Arc::new(App {
            home: PathBuf::new(),
            workspace: fixture.0.clone(),
            token: "secret".into(),
            port: 4321,
            origins: vec!["https://course.vercel.app".into()],
            assets: Assets::default(),
            worker: Mutex::new(None),
        });
        // The old route would wait forever here, or try to start the absent Node worker.
        let worker = app.worker.lock().await;
        for url in [
            "/api/capture/capture-1",
            "/api/capture/capture-1/artifacts/diagram-1",
            "/api/capture/capture-1/artifacts/diagram-1?view=source",
        ] {
            for (authorization, expected) in [
                ("", StatusCode::UNAUTHORIZED),
                ("Bearer secret", StatusCode::OK),
            ] {
                let request = Request::builder()
                    .uri(url)
                    .header("host", "127.0.0.1:4321")
                    .header("origin", "https://course.vercel.app")
                    .header("authorization", authorization)
                    .body(Body::empty())
                    .unwrap();
                let response = tokio::time::timeout(
                    Duration::from_secs(2),
                    router(app.clone()).oneshot(request),
                )
                .await
                .expect("images must not wait for the course worker")
                .unwrap();
                assert_eq!(response.status(), expected);
                assert_eq!(response.headers()["cache-control"], "no-store");
                assert_eq!(
                    response.headers()["access-control-allow-origin"],
                    "https://course.vercel.app"
                );
            }
        }
        assert!(worker.is_none());
    }
}
