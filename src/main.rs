mod assets;
use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::any,
};
use cc_daemon::{capture, commands, connection, demo, http_api, mcp, store::Store};
use serde_json::json;
use std::{env, path::PathBuf, sync::Arc};
const MAX_BODY: usize = 18_000_000;
struct App {
    workspace: PathBuf,
    key_path: PathBuf,
    port: u16,
    origins: Vec<String>,
    assets: assets::Assets,
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
    if let Some(value) = &origin
        && !app
            .origins
            .iter()
            .any(|allowed| value.as_bytes() == allowed.as_bytes())
    {
        return error(
            StatusCode::FORBIDDEN,
            "This frontend origin is not allowed. Start with --origin <exact URL>.",
        );
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
        // Read the current key so a local reset revokes existing clients immediately.
        let token = match connection::read(&app.key_path) {
            Ok(token) => token,
            Err(_) => {
                return error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Connection key unavailable. Run cruise reset-connection-key locally.",
                );
            }
        };
        if supplied != format!("Bearer {token}") {
            error(
                StatusCode::UNAUTHORIZED,
                "Enter the connection key printed by cruise run.",
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

async fn api(State(app): State<Arc<App>>, request: Request) -> Response {
    if request.uri().path() == "/api/health" && request.method() == Method::GET {
        return axum::Json(
            json!({"service":"course-captain","version":env!("CARGO_PKG_VERSION"),"apiVersion":1}),
        )
        .into_response();
    }
    if request.method() == Method::GET
        && let Some(response) = app.assets.serve(&app.workspace, request.uri()).await
    {
        return response;
    }
    if request.method() == Method::POST
        && !request
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json"))
    {
        return http_api::error("Expected application/json");
    }
    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "Request body is too large"),
    };
    let store = Store::new(app.workspace.clone());
    // A disconnected browser does not cancel a save already in progress.
    match tokio::task::spawn_blocking(move || {
        http_api::route(
            &store,
            parts.method.as_str(),
            parts.uri.path(),
            parts.uri.query().unwrap_or(""),
            &bytes,
        )
    })
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(message)) => http_api::error(message),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
fn router(app: Arc<App>) -> Router {
    Router::new()
        .fallback(any(api))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = connection::home();
    let _ = dotenvy::from_path(home.join(".env.local"));
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "--help".into());
    if ["--help", "-h", "help"].contains(&command.as_str()) {
        println!(
            "cruise run [--port 4321] [--origin https://cruise.ink] [--workspace PATH]\ncruise mcp [--workspace PATH]\ncruise reset-connection-key\ncruise setup-codex [--windows-host]\ncruise check-mcp [--workspace PATH]\ncruise capture --url URL [--list | --course ID --title TITLE] [--workspace PATH]\ncruise demo [--workspace PATH]\n\nNative Rust API, course engine and MCP. Ctrl+C stops the HTTP server.\nSet CRUISE_HOME if the daemon checkout moves after installation."
        );
        return Ok(());
    }
    if ![
        "run",
        "reset-connection-key",
        "mcp",
        "setup-codex",
        "check-mcp",
        "capture",
        "demo",
        "command-worker",
    ]
    .contains(&command.as_str())
    {
        return Err(
            "Expected: cruise run, mcp, reset-connection-key, setup-codex, check-mcp, capture, or demo (or --help)"
                .into(),
        );
    }
    let mut workspace = env::var_os("CRUISE_WORKSPACE")
        .or_else(|| env::var_os("COURSE_CAPTAIN_WORKSPACE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("workspace/content"));
    if command == "capture" {
        let mut capture_args = Vec::new();
        while let Some(option) = args.next() {
            if option == "--workspace" {
                workspace = args.next().ok_or("Missing --workspace value")?.into();
            } else {
                capture_args.push(option);
            }
        }
        if workspace.is_relative() {
            workspace = home.join(workspace);
        }
        capture::run(&Store::new(workspace), &capture_args)?;
        return Ok(());
    }
    let mut port = 4321u16;
    let mut windows_host = false;
    let mut request = None;
    let mut origins: Vec<String> = env::var("CRUISE_ALLOWED_ORIGINS")
        .or_else(|_| env::var("COURSE_CAPTAIN_ALLOWED_ORIGINS"))
        .unwrap_or_else(|_| "http://localhost:3000,http://127.0.0.1:3000".into())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    while let Some(option) = args.next() {
        if option == "--windows-host" {
            windows_host = true;
            continue;
        }
        let value = args.next().ok_or("Missing option value")?;
        match option.as_str() {
            "--port" => port = value.parse()?,
            "--workspace" => workspace = value.into(),
            "--origin" => origins.push(value),
            "--request" => request = Some(value),
            _ => return Err(format!("Unknown option: {option}").into()),
        }
    }
    if workspace.is_relative() {
        workspace = home.join(workspace);
    }
    let store = Store::new(workspace.clone());
    match command.as_str() {
        "reset-connection-key" => {
            let token = connection::reset(&connection::key_path())?;
            println!(
                "Connection key: {token}\nPrevious key revoked. Reconnect your browsers with this key."
            );
            return Ok(());
        }
        "mcp" => {
            mcp::serve(&store)?;
            return Ok(());
        }
        "check-mcp" => {
            let count = mcp::check(&store)?;
            println!(
                "cruise MCP ready: {count} tools; workspace readable; no Node runtime required."
            );
            return Ok(());
        }
        "setup-codex" => {
            let path = mcp::configure(&home, windows_host)?;
            println!(
                "cruise MCP configured at {}. Reopen the trusted project to reconnect.",
                path.display()
            );
            return Ok(());
        }
        "command-worker" => {
            commands::worker(&store, &request.ok_or("Missing --request")?)?;
            return Ok(());
        }
        "demo" => {
            demo::seed(&store)?;
            println!("Demo workspace created.");
            return Ok(());
        }
        _ => {}
    }
    if port == 0 {
        return Err("Port must be between 1 and 65535".into());
    }
    if origins.iter().any(|o| !valid_origin(o)) {
        return Err("Origins must be exact HTTPS origins (or HTTP loopback), without a path or trailing slash".into());
    }
    let key_path = connection::key_path();
    let token = connection::load_or_create(&key_path)?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    println!(
        "cruise API: http://127.0.0.1:{port}\nConnection key: {token}\nWorkspace: {}\nAllowed frontends: {}\nPress Ctrl+C to stop.",
        workspace.display(),
        origins.join(", ")
    );
    let app = Arc::new(App {
        workspace,
        key_path,
        port,
        origins,
        assets: assets::Assets::default(),
    });
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async {
            #[cfg(unix)]
            {
                let mut terminate =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("SIGTERM handler");
                tokio::select! {_=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{}}
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;
    #[tokio::test]
    async fn authentication_exact_origins_and_native_crud() {
        let root = std::env::temp_dir().join(format!("cruise-http-{}", cc_daemon::store::id()));
        let key_path = root.join("connection-key");
        let token = connection::load_or_create(&key_path).unwrap();
        let bearer = format!("Bearer {token}");
        let app = Arc::new(App {
            workspace: root.clone(),
            key_path: key_path.clone(),
            port: 4321,
            origins: vec!["https://cruise.ink".into()],
            assets: assets::Assets::default(),
        });
        for (host, origin, auth, method, status) in [
            (
                "127.0.0.1:4321",
                "https://cruise.ink",
                bearer.as_str(),
                "GET",
                200,
            ),
            ("127.0.0.1:4321", "https://cruise.ink", "", "GET", 401),
            (
                "127.0.0.1:4321",
                "https://evil.example",
                bearer.as_str(),
                "GET",
                403,
            ),
            (
                "evil.example:4321",
                "https://cruise.ink",
                bearer.as_str(),
                "GET",
                403,
            ),
            ("127.0.0.1:4321", "https://cruise.ink", "", "OPTIONS", 204),
        ] {
            let r = Request::builder()
                .uri("/api/health")
                .method(method)
                .header("host", host)
                .header("origin", origin)
                .header("authorization", auth)
                .body(Body::empty())
                .unwrap();
            assert_eq!(
                router(app.clone())
                    .oneshot(r)
                    .await
                    .unwrap()
                    .status()
                    .as_u16(),
                status
            );
        }
        let r = Request::builder()
            .method("POST")
            .uri("/api/action")
            .header("host", "127.0.0.1:4321")
            .header("origin", "https://cruise.ink")
            .header("authorization", &bearer)
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"action":"course.create","data":{"code":"NATIVE","name":"Native backend"}}"#,
            ))
            .unwrap();
        let r = router(app.clone()).oneshot(r).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let v: serde_json::Value =
            serde_json::from_slice(&to_bytes(r.into_body(), MAX_BODY).await.unwrap()).unwrap();
        assert_eq!(v["result"]["code"], "NATIVE");
        let replacement = connection::reset(&key_path).unwrap();
        for (key, expected) in [
            (token, StatusCode::UNAUTHORIZED),
            (replacement, StatusCode::OK),
        ] {
            let request = Request::builder()
                .uri("/api/health")
                .header("host", "127.0.0.1:4321")
                .header("authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap();
            assert_eq!(
                router(app.clone()).oneshot(request).await.unwrap().status(),
                expected
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn origins_reject_paths_and_nonloopback_http() {
        assert!(valid_origin("https://cruise.ink"));
        for o in [
            "http://evil.example",
            "https://cruise.ink/path",
            "https://cruise.ink/",
            "https://user@cruise.ink",
            "*",
        ] {
            assert!(!valid_origin(o));
        }
    }
}
