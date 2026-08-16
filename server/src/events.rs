//! Event bus — replaces the Tauri `AppHandle::emit` / `listen` mechanism.
//!
//! The Tauri version pushed events (config-apply-status, rule-set-apply-status,
//! connection snapshots, deep-link-urls, …) through `app.emit(event, payload)`.
//! This module provides the equivalent using `tokio::sync::broadcast`, so the
//! WebSocket bridge and any internal worker can publish/subscribe without Tauri.

use serde_json::Value;
use std::sync::Arc;
use tokio::sync::broadcast;

/// A single application event (mirrors the shape the Tauri frontend receives).
#[derive(Debug, Clone)]
pub struct AppEvent {
    /// Event name, e.g. `"config-apply-status"`.
    pub name: String,
    /// JSON payload (already serialized).
    pub payload: Value,
}

/// Cloneable handle to the shared event bus.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<AppEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(512)
    }
}

impl EventBus {
    /// Create a bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Publish an event to all subscribers.
    pub fn emit(&self, name: impl Into<String>, payload: Value) {
        let _ = self.tx.send(AppEvent {
            name: name.into(),
            payload,
        });
    }

    /// Convenience: emit with a serializable payload.
    pub fn emit_json<S: serde::Serialize>(&self, name: impl Into<String>, payload: &S) {
        let value = serde_json::to_value(payload).unwrap_or(Value::Null);
        self.emit(name, value);
    }

    /// Subscribe to all events.
    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }
}

/// Shared, thread-safe event bus (used as axum state alongside `AppState`).
pub type SharedEventBus = Arc<EventBus>;
