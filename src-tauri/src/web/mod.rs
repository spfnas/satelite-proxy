//! Web bridge — exposes the Tauri command surface over HTTP + WebSocket so the
//! built React frontend can run in a plain browser (Linux web mode, or a local
//! dashboard on any platform).
//!
//! Layout:
//! - `POST /api/{command}`  → `web::dispatch` (same commands Tauri registers)
//! - `GET  /ws`             → pushes every Tauri event as `{"event","payload"}`
//! - `GET  /health`         → liveness probe
//! - everything else        → static frontend from `../dist` (SPA fallback)
//!
//! Overrides:
//! - `SATELITE_WEB_PORT` (default 8268)
//! - `SATELITE_WEB_DIST` (default `src-tauri/../dist`)

use crate::app_log;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub mod dispatch;

/// Broadcast bus bridged from `AppHandle::listen_any` → WebSocket subscribers.
pub struct EventBus {
    pub tx: tokio::sync::broadcast::Sender<String>,
}

/// Static frontend location for the SPA fallback / asset serving.
pub struct WebConfig {
    pub dist_dir: PathBuf,
}

/// Start the web bridge. Safe to call on every platform; binds loopback only.
pub fn start(app: &AppHandle) {
    use tauri::Listener;

    let (tx, _rx) = tokio::sync::broadcast::channel(512);
    let dist_dir = std::env::var("SATELITE_WEB_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist"));

    app.manage(EventBus { tx: tx.clone() });
    app.manage(WebConfig { dist_dir });

    // Forward every app event (config-apply-status, rule-set-apply-status,
    // deep-link-urls, connection snapshots, …) into the broadcast bus.
    let forward_app = app.clone();
    let forward_tx = tx.clone();
    forward_app.listen("web://relay", move |event| {
        let name = serde_json::to_string(&event.id().to_string())
            .unwrap_or_else(|_| "\"\"".to_string());
        let raw = event.payload();
        let payload = if raw.is_empty() {
            "null".to_string()
        } else {
            serde_json::to_string(raw).unwrap_or_else(|_| "null".to_string())
        };
        let _ = forward_tx.send(format!("{{\"event\":{name},\"payload\":{payload}}}"));
    });

    let port: u16 = std::env::var("SATELITE_WEB_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8268);
    let serve_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let router = build_router().with_state(serve_app);
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                app_log::info("web", format!("web bridge listening on http://{addr}"));
                app_log::info(
                    "web",
                    "open the UI in a browser, or set SATELITE_WEB_DIST to serve the frontend",
                );
                if let Err(e) = axum::serve(listener, router).await {
                    app_log::error("web", format!("serve error: {e}"));
                }
            }
            Err(e) => app_log::error("web", format!("bind {addr} failed: {e}")),
        }
    });
}

fn build_router() -> Router<AppHandle> {
    Router::new()
        .route("/api/{command}", post(api_handler).options(preflight))
        .route("/ws", get(ws_handler))
        .route("/health", get(health))
        .fallback(serve_static)
        .layer(axum::middleware::from_fn(cors_mw))
}

async fn preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn cors_mw(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type"),
    );
    resp
}

/// `POST /api/{command}` — body is the JSON args map (snake_case keys).
async fn api_handler(
    Path(command): Path<String>,
    State(app): State<AppHandle>,
    Json(args): Json<Value>,
) -> Response {
    match dispatch::dispatch(app, &command, args).await {
        Ok(data) => (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response(),
        Err(msg) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": msg })),
        )
            .into_response(),
    }
}

async fn health() -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "app": "satelite",
        "version": env!("CARGO_PKG_VERSION"),
        "platform": std::env::consts::OS,
    }))
}

/// `GET /ws` — push `{"event","payload"}` frames for every app event.
async fn ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(app): State<AppHandle>,
) -> Response {
    let rx = app.state::<EventBus>().tx.subscribe();
    ws.on_upgrade(move |socket| ws_loop(socket, rx))
}

async fn ws_loop(
    socket: axum::extract::ws::WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<String>,
) {
    use axum::extract::ws::Message as WsMessage;
    use futures_util::{SinkExt, StreamExt};
    let (mut tx, mut stream) = socket.split();
    loop {
        tokio::select! {
            framed = rx.recv() => match framed {
                Ok(text) => {
                    if tx.send(WsMessage::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            },
            msg = stream.next() => match msg {
                Some(Ok(WsMessage::Close(_))) | None => break,
                Some(Ok(WsMessage::Ping(p))) => {
                    if tx.send(WsMessage::Pong(p)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
        }
    }
}

/// SPA asset serving with a small mime map. Extension-less unknown paths fall
/// back to `index.html`; unknown asset-like paths return 404.
async fn serve_static(uri: Uri, State(app): State<AppHandle>) -> Response {
    let cfg = app.state::<WebConfig>();
    let mut rel = uri.path().trim_start_matches('/').to_string();
    if rel.is_empty() {
        rel = "index.html".into();
    }
    if rel.contains("..") || rel.contains('\\') {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let candidate = cfg.dist_dir.join(&rel);
    if candidate.is_file() {
        match std::fs::read(&candidate) {
            Ok(bytes) => {
                return Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime_for(&rel))
                    .body(axum::body::Body::from(bytes))
                    .unwrap();
            }
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "read failed").into_response(),
        }
    }
    let last = rel.rsplit('/').next().unwrap_or(&rel);
    if !last.contains('.') {
        let idx = cfg.dist_dir.join("index.html");
        if let Ok(bytes) = std::fs::read(&idx) {
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(axum::body::Body::from(bytes))
                .unwrap();
        }
    }
    (StatusCode::NOT_FOUND, "not found").into_response()
}

fn mime_for(path: &str) -> &'static str {
    let ext = path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}