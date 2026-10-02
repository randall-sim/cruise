mod assets;

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::any,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::json;
use std::{collections::HashMap, env, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

const MAX_BODY: usize = 18_000_000;
struct Worker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}
struct App {
    home: PathBuf,
    workspace: PathBuf,
    token: String,
    port: u16,
    origins: Vec<String>,
    assets: assets::Assets,
    // Serialize course operations; image reads bypass this worker.
    worker: Mutex<Option<Worker>>,
}

fn error(status: StatusCode, message: impl ToString) -> Response {
    (status, axum::Json(json!({"error":message.to_string()}))).into_response()
}

async fn guard(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if ![
        format!("127.0.0.1:{}", app.port),
        format!("localhost:{}", app.port),
    ]
    .contains(&host.to_string())
    {
        return error(StatusCode::FORBIDDEN, "Invalid loopback Host");
    }
    let origin = headers.get("origin").cloned();
    if let Some(value) = &origin {
        if !app
            .origins
            .iter()
            .any(|allowed| value.as_bytes() == allowed.as_bytes())
        {
            return error(
                StatusCode::FORBIDDEN,
                "This frontend origin is not allowed. Start with --origin <exact URL>.",
            );
        }
    }
    let preflight = request.method() == Method::OPTIONS;
    let mut response = if preflight {
        if origin.is_none() {
            return error(StatusCode::BAD_REQUEST, "Preflight needs an Origin");
        }
        StatusCode::NO_CONTENT.into_response()
    } else {
        let supplied = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if supplied != format!("Bearer {}", app.token) {
            error(
                StatusCode::UNAUTHORIZED,
                "Enter the connection key printed by course-captain run.",
            )
        } else {
            next.run(request).await
        }
    };
    let output = response.headers_mut();
    output.insert("cache-control", HeaderValue::from_static("no-store"));
    output.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    output.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    output.insert("vary", HeaderValue::from_static("Origin"));
    if let Some(origin) = origin {
        output.insert("access-control-allow-origin", origin);
        output.insert(
            "access-control-expose-headers",
            HeaderValue::from_static("Content-Disposition, Server-Timing"),
        );
        if preflight {
            output.insert(
                "access-control-allow-methods",
                HeaderValue::from_static("GET, POST, OPTIONS"),
            );
            output.insert(
                "access-control-allow-headers",
                HeaderValue::from_static("Authorization, Content-Type"),
            );
            output.insert(
                "access-control-allow-private-network",
                HeaderValue::from_static("true"),
            );
        }
    }
    response
}

impl Worker {
    fn start(app: &App) -> Result<Self, String> {
        let mut child = Command::new("node")
            .arg(app.home.join("node_modules/tsx/dist/cli.mjs"))
            .arg(app.home.join("scripts/api-worker.ts"))
            .current_dir(&app.home)
            .env("COURSE_CAPTAIN_WORKSPACE", &app.workspace)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Cannot start course worker: {e}. Run npm ci in backend."))?;
        Ok(Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
        })
    }
    async fn call(&mut self, request: serde_json::Value) -> Result<WireResponse, String> {
        let mut bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|e| e.to_string())?;
        self.input.flush().await.map_err(|e| e.to_string())?;
        let mut line = String::new();
        if self
            .output
            .read_line(&mut line)
            .await
            .map_err(|e| e.to_string())?
            == 0
        {
            return Err("Course worker exited; check its error above".into());
        }
        serde_json::from_str(&line).map_err(|e| format!("Invalid course worker response: {e}"))
    }
}

#[derive(Deserialize)]
struct WireResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}

async fn api(State(app): State<Arc<App>>, request: Request) -> Response {
    if request.uri().path() == "/api/health" && request.method() == Method::GET {
        return axum::Json(json!({"service":"course-captain", "version":env!("CARGO_PKG_VERSION"), "apiVersion":1})).into_response();
    }
    if !request.uri().path().starts_with("/api/") {
        return error(StatusCode::NOT_FOUND, "Unknown API route");
    }
    if ![Method::GET, Method::POST].contains(request.method()) {
        return error(StatusCode::METHOD_NOT_ALLOWED, "Method not allowed");
    }
    if request.method() == Method::GET {
        if let Some(response) = app.assets.serve(&app.workspace, request.uri()).await {
            return response;
        }
    }
    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "Request body is too large"),
    };
    let wire = json!({"method":parts.method.as_str(), "url":parts.uri.to_string(), "contentType":parts.headers.get("content-type").and_then(|v|v.to_str().ok()), "body":STANDARD.encode(bytes)});
    // Finish consuming the reply even if a refresh cancels the HTTP handler.
    // Otherwise the next request can receive this request's (possibly partial) reply.
    tokio::spawn(worker_request(app, wire))
        .await
        .unwrap_or_else(|_| error(StatusCode::BAD_GATEWAY, "Course worker request failed"))
}

async fn worker_request(app: Arc<App>, wire: serde_json::Value) -> Response {
    let mut slot = app.worker.lock().await;
    if slot.is_none() {
        match Worker::start(&app) {
            Ok(worker) => *slot = Some(worker),
            Err(message) => return error(StatusCode::SERVICE_UNAVAILABLE, message),
        }
    }
    let reply =
        tokio::time::timeout(Duration::from_secs(120), slot.as_mut().unwrap().call(wire)).await;
    match reply {
        Ok(Ok(reply)) => {
            let bytes = match STANDARD.decode(reply.body) {
                Ok(bytes) => bytes,
                Err(_) => return error(StatusCode::BAD_GATEWAY, "Invalid worker body"),
            };
            let mut response = Response::new(Body::from(bytes));
            *response.status_mut() =
                StatusCode::from_u16(reply.status).unwrap_or(StatusCode::BAD_GATEWAY);
            for (name, value) in reply.headers {
                if let (Ok(name), Ok(value)) = (
                    name.parse::<axum::http::HeaderName>(),
                    value.parse::<HeaderValue>(),
                ) {
                    response.headers_mut().insert(name, value);
                }
            }
            response
        }
        failure => {
            if let Some(mut worker) = slot.take() {
                let _ = worker.child.kill().await;
            }
            error(
                StatusCode::BAD_GATEWAY,
                match failure {
                    Ok(Err(message)) => message,
                    _ => {
                        "Course worker timed out; check the result before retrying a change".into()
                    }
                },
            )
        }
    }
}

fn router(app: Arc<App>) -> Router {
    Router::new()
        .fallback(any(api))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app)
}

fn valid_origin(value: &str) -> bool {
    value.parse::<axum::http::Uri>().is_ok_and(|uri| {
        uri.authority().is_some()
            && !value.contains('@')
            && uri.path_and_query().is_none_or(|p| p.as_str() == "/")
            && !value.ends_with('/')
            && (uri.scheme_str() == Some("https")
                || (uri.scheme_str() == Some("http")
                    && matches!(uri.host(), Some("localhost" | "127.0.0.1"))))
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = env::var_os("COURSE_CAPTAIN_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let _ = dotenvy::from_path(home.join(".env.local"));
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("run") => {}
        None | Some("--help" | "-h") => {
            println!(
                "course-captain run [--port 4321] [--origin https://your-app.vercel.app] [--workspace PATH]\n\nStarts the loopback API. Ctrl+C stops it. Requires backend npm ci.\nSet COURSE_CAPTAIN_HOME if the backend checkout moves after installation."
            );
            return Ok(());
        }
        _ => return Err("Expected: course-captain run (or --help)".into()),
    }
    let mut port = 4321u16;
    let mut workspace = env::var_os("COURSE_CAPTAIN_WORKSPACE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("workspace/content"));
    let mut origins: Vec<String> = env::var("COURSE_CAPTAIN_ALLOWED_ORIGINS")
        .unwrap_or_else(|_| "http://localhost:3000,http://127.0.0.1:3000".into())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    while let Some(arg) = args.next() {
        let value = args.next().ok_or("Missing option value")?;
        match arg.as_str() {
            "--port" => port = value.parse()?,
            "--workspace" => workspace = value.into(),
            "--origin" => origins.push(value),
            _ => return Err(format!("Unknown option: {arg}").into()),
        }
    }
    if port == 0 {
        return Err("Port must be between 1 and 65535".into());
    }
    if origins.iter().any(|o| !valid_origin(o)) {
        return Err("Origins must be exact HTTPS origins (or HTTP loopback), without a path or trailing slash".into());
    }
    if workspace.is_relative() {
        workspace = home.join(workspace);
    }
    if !home.join("node_modules/tsx/dist/cli.mjs").is_file() {
        return Err("Missing course worker dependencies. Run npm ci in backend.".into());
    }
    // The key is process-local and never appears in a URL, build artifact or Git file.
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(|e| e.to_string())?;
    let token = secret
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    println!(
        "Course Captain API: http://127.0.0.1:{port}\nConnection key: {token}\nWorkspace: {}\nAllowed frontends: {}\nPress Ctrl+C to stop.",
        workspace.display(),
        origins.join(", ")
    );
    let app = Arc::new(App {
        home,
        workspace,
        token,
        port,
        origins,
        assets: assets::Assets::default(),
        worker: Mutex::new(None),
    });
    axum::serve(listener, router(app.clone()))
        .with_graceful_shutdown(async {
            #[cfg(unix)]
            {
                let mut terminate =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("SIGTERM handler");
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
        })
        .await?;
    if let Some(mut worker) = app.worker.lock().await.take() {
        let _ = worker.child.kill().await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;
    #[tokio::test]
    async fn loopback_api_requires_key_and_exact_origin_and_handles_preflight() {
        let app = Arc::new(App {
            home: PathBuf::new(),
            workspace: PathBuf::new(),
            token: "secret".into(),
            port: 4321,
            origins: vec!["https://course.vercel.app".into()],
            assets: assets::Assets::default(),
            worker: Mutex::new(None),
        });
        for (host, origin, auth, method, status) in [
            (
                "127.0.0.1:4321",
                "https://course.vercel.app",
                "Bearer secret",
                "GET",
                200,
            ),
            (
                "127.0.0.1:4321",
                "https://course.vercel.app",
                "",
                "GET",
                401,
            ),
            (
                "127.0.0.1:4321",
                "https://evil.example",
                "Bearer secret",
                "GET",
                403,
            ),
            (
                "evil.example:4321",
                "https://course.vercel.app",
                "Bearer secret",
                "GET",
                403,
            ),
            (
                "127.0.0.1:4321",
                "https://course.vercel.app",
                "",
                "OPTIONS",
                204,
            ),
        ] {
            let request = Request::builder()
                .uri("/api/health")
                .method(method)
                .header("host", host)
                .header("origin", origin)
                .header("authorization", auth)
                .body(Body::empty())
                .unwrap();
            let response = router(app.clone()).oneshot(request).await.unwrap();
            assert_eq!(response.status().as_u16(), status);
            if status != 403 {
                assert_eq!(response.headers()["access-control-allow-origin"], origin);
            } else {
                assert!(
                    !response
                        .headers()
                        .contains_key("access-control-allow-origin")
                );
            }
        }
        assert!(!valid_origin("https://example.com/path"));
        assert!(!valid_origin("https://user@example.com"));
        assert!(!valid_origin("http://example.com"));
        assert!(valid_origin("https://course.vercel.app"));
    }

    #[tokio::test]
    async fn cancelled_request_does_not_shift_or_truncate_worker_responses() {
        for partial_response in [false, true] {
            let mut child = Command::new("node")
                .args([
                    "--input-type=module",
                    "--eval",
                    r#"
import { createInterface } from 'node:readline';
for await (const line of createInterface({ input: process.stdin })) {
    const { url } = JSON.parse(line);
    const response = JSON.stringify({ status: 200, headers: {}, body: Buffer.from(url).toString('base64') }) + '\n';
    if (url === '/api/first') {
        const split = process.env.PARTIAL_RESPONSE === 'true' ? 15 : 0;
        process.stdout.write(response.slice(0, split));
        process.stderr.write('started\n');
        await new Promise(resolve => setTimeout(resolve, 500));
        process.stdout.write(response.slice(split));
    } else process.stdout.write(response);
}
"#,
                ])
                .env("PARTIAL_RESPONSE", partial_response.to_string())
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let mut started = BufReader::new(child.stderr.take().unwrap());
            let worker = Worker {
                input: child.stdin.take().unwrap(),
                output: BufReader::new(child.stdout.take().unwrap()),
                child,
            };
            let app = Arc::new(App {
                home: PathBuf::new(),
                workspace: PathBuf::new(),
                token: "secret".into(),
                port: 4321,
                origins: vec![],
                assets: assets::Assets::default(),
                worker: Mutex::new(Some(worker)),
            });
            let request = |path| {
                Request::builder()
                    .uri(path)
                    .header("host", "127.0.0.1:4321")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap()
            };
            let first = tokio::spawn(router(app.clone()).oneshot(request("/api/first")));
            let mut signal = String::new();
            tokio::time::timeout(Duration::from_secs(5), started.read_line(&mut signal))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(signal, "started\n");
            first.abort();
            assert!(first.await.unwrap_err().is_cancelled());
            for path in ["/api/second", "/api/third"] {
                let response = tokio::time::timeout(
                    Duration::from_secs(5),
                    router(app.clone()).oneshot(request(path)),
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let body = to_bytes(response.into_body(), MAX_BODY).await.unwrap();
                assert_eq!(body.as_ref(), path.as_bytes());
            }
        }
    }
}
