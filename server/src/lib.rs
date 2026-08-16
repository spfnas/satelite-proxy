//! Satelite Proxy — Pure Web backend.
//!
//! This crate provides the same subscription/node/DNS/rule/connection-management
//! logic as the original Tauri desktop app, but without any Tauri dependency.
//! It serves the React frontend as static files and exposes all commands through
//! a REST + WebSocket API on `http://localhost:8268`.
//!
//! ## Architecture
//!
//! - **`main.rs`** — axum server entry point, starts the HTTP listener and
//!   background runtime workers.
//! - **`state.rs`** — `AppState` (shared via `Arc`), holds the persistent store,
//!   runtime handle, and background-job coordination.
//! - **`web/mod.rs`** — axum router, HTTP dispatch, WebSocket event bus, static
//!   file serving.
//! - **`events.rs`** — event bus (`tokio::sync::broadcast`);
//!   replaces `AppHandle::emit` used in the Tauri version.
//! - **`rule_apply.rs`** — debounced, globally serialised apply-and-restart for
//!   rule-set changes (same logic, no Tauri).
//! - **`domain/`** — pure data models (subscription, node, rule, DNS, settings).
//! - **`storage/`** — persistent store (JSON-based, `AppStore`).
//! - **`subscription/`** — parse Clash/Sing-box subscription formats.
//! - **`config/`** — generate sing-box JSON config from store state.
//! - **`services/`** — import helpers, latency test.
//! - **`api/`** — Clash API client (reqwest/ureq).
//! - **`core/`** — sing-box process lifecycle, download, path resolution.
//! - **`runtime/`** — proxy runtime state.
//! - **`proxy/`** — platform-specific proxy helpers (stub on Linux).
//! - **`app_log.rs`** — structured file logging.
//! - **`log_retention.rs`** — hourly log rotation & cleanup.
//! - **`error.rs`** — error types.

// `arg::<T>` with never-type fallback was accepted in older rustc; keep parity
// with the Tauri version instead of failing the build.
#![allow(dependency_on_unit_never_type_fallback)]

pub mod api;
pub mod app_log;
pub mod commands;
pub mod compat;
pub mod config;
pub mod conn_journal;
pub mod core;
pub mod domain;
pub mod error;
pub mod events;
pub mod log_retention;
pub mod proxy;
pub mod remote_rule_auto;
pub mod rule_apply;
pub mod runtime;
pub mod services;
pub mod smart_switch;
pub mod state;
pub mod storage;
pub mod subscription;
pub mod subscription_auto;
pub mod web;

pub use error::AppResult;
pub use state::AppState;
pub use events::EventBus;