//! Satelite Proxy — pure Web server entry point.
//!
//! Loads the app store, starts background workers, and serves the React
//! frontend + REST/WebSocket API on `http://localhost:8268` (see `web`).

use satelite_web::{AppState, EventBus};
use std::path::PathBuf;
use std::sync::Arc;

fn default_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SATELITE_DATA_DIR") {
        return PathBuf::from(dir);
    }
    // Linux convention: $XDG_DATA_HOME/satelite-proxy, else ~/.local/share/…
    if let Ok(base) = std::env::var("XDG_DATA_HOME") {
        if !base.is_empty() {
            return PathBuf::from(base).join("satelite-proxy");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("satelite-proxy");
        }
    }
    // Fallback: current directory (Windows dev convenience).
    PathBuf::from("satelite-data")
}

fn default_resource_dir() -> Option<PathBuf> {
    std::env::var("SATELITE_RESOURCE_DIR").ok().map(PathBuf::from)
}

fn main() {
    let data_dir = default_data_dir();
    std::fs::create_dir_all(&data_dir).expect("create app data dir");
    satelite_web::app_log::init(data_dir.join("logs"));

    let resource_dir = default_resource_dir();
    let state = match AppState::load(data_dir.clone(), resource_dir) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("[satelite] failed to load app store: {e}");
            std::process::exit(1);
        }
    };

    satelite_web::app_log::info("app", "Satelite web server starting");

    // Build reqwest blocking client on a plain OS thread so its internal
    // Tokio runtime is never created/dropped on an async worker.
    std::thread::spawn(|| {
        satelite_web::api::warmup_blocking_client();
    });

    let bus = EventBus::default();

    // Auto-start proxy if the user had it enabled (desktop parity).
    let auto_start = state
        .with_store(|s| Ok(s.settings.auto_start_proxy))
        .unwrap_or(false);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    rt.block_on(async {
        // Background workers (same as the desktop build, minus tray/deep-link).
        // All tokio::spawn calls must run inside the runtime.
        let ctx = satelite_web::compat::AppCtx::new(state.clone(), bus.clone());
        satelite_web::conn_journal::spawn_connection_journal(state.clone());
        satelite_web::subscription_auto::spawn(state.clone());
        satelite_web::remote_rule_auto::spawn(ctx.clone());
        satelite_web::smart_switch::spawn(state.clone());

        if auto_start {
            satelite_web::app_log::info("app", "auto_start_proxy is enabled — starting core");
            let st = state.clone();
            std::thread::spawn(move || {
                let _ = st.start_proxy(st.resource_dir.as_deref(), false);
            });
        }

        let web = satelite_web::web::WebState {
            state: state.clone(),
            bus: bus.clone(),
            dist_dir: satelite_web::web::default_dist_dir(),
        };

        if let Err(e) = satelite_web::web::serve(web).await {
            satelite_web::app_log::error("web", format!("server error: {e}"));
            eprintln!("[satelite] server error: {e}");
            std::process::exit(1);
        }
    });
}
