//! Web API layer — the pure-Web replacement for the Tauri command surface.
//!
//! Layout (same as the desktop web bridge, but without any Tauri types):
//! - `POST /api/{command}`  → `web::dispatch` (same command names/shapes)
//! - `GET  /ws`             → pushes every app event as `{"event","payload"}`
//! - `GET  /health`         → liveness probe
//! - everything else        → static frontend from dist dir (SPA fallback)
//!
//! Overrides:
//! - `SATELITE_WEB_PORT` (default 8268)
//! - `SATELITE_WEB_DIST` (default `server/../dist`)

use crate::app_log;
use crate::compat::AppCtx;
use crate::events::EventBus;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

pub mod dispatch;

/// Shared HTTP state: app state + event bus + frontend dist directory.
#[derive(Clone)]
pub struct WebState {
    pub state: Arc<AppState>,
    pub bus: EventBus,
    pub dist_dir: PathBuf,
}

/// Build the axum router with the given web state.
pub fn build_router(web: WebState) -> Router {
    Router::new()
        .route("/api/{command}", post(api_handler).options(preflight))
        .route("/ws", get(ws_handler))
        .route("/health", get(health))
        .fallback(serve_static)
        .layer(axum::middleware::from_fn(cors_mw))
        .with_state(web)
}

/// Resolve the static frontend directory (env override, else `../dist`).
pub fn default_dist_dir() -> PathBuf {
    std::env::var("SATELITE_WEB_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // server/../dist  (project root dist)
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist")
        })
}

/// Bind and serve on loopback (or `SATELITE_WEB_ADDR` for server deploys).
pub async fn serve(web: WebState) -> std::io::Result<()> {
    let addr_env = std::env::var("SATELITE_WEB_ADDR").unwrap_or_default();
    let addr: std::net::SocketAddr = if addr_env.is_empty() {
        let port: u16 = std::env::var("SATELITE_WEB_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8268);
        ([127, 0, 0, 1], port).into()
    } else {
        addr_env
            .parse()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?
    };

    app_log::info("web", format!("web server listening on http://{addr}"));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, build_router(web))
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}

/// AppCtx convenience used by dispatch.
impl WebState {
    pub fn ctx(&self) -> AppCtx {
        AppCtx::new(self.state.clone(), self.bus.clone())
    }
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
    State(web): State<WebState>,
    Json(args): Json<Value>,
) -> Response {
    let ctx = web.ctx();
    match dispatch::dispatch(&ctx, &command, args).await {
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
    State(web): State<WebState>,
) -> Response {
    let rx = web.bus.subscribe();
    ws.on_upgrade(move |socket| ws_loop(socket, rx))
}

async fn ws_loop(
    socket: axum::extract::ws::WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<crate::events::AppEvent>,
) {
    use axum::extract::ws::Message as WsMessage;
    use futures_util::{SinkExt, StreamExt};
    let (mut tx, mut stream) = socket.split();
    loop {
        tokio::select! {
            framed = rx.recv() => match framed {
                Ok(event) => {
                    let frame = json!({ "event": event.name, "payload": event.payload });
                    if tx.send(WsMessage::Text(frame.to_string().into())).await.is_err() {
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
async fn serve_static(uri: Uri, State(web): State<WebState>) -> Response {
    let mut rel = uri.path().trim_start_matches('/').to_string();
    if rel.is_empty() {
        rel = "index.html".into();
    }
    if rel.contains("..") || rel.contains('\\') {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let candidate = web.dist_dir.join(&rel);
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
        let idx = web.dist_dir.join("index.html");
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
