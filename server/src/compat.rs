//! Tauri-compatibility shims for the Web backend.
//!
//! The commands layer was written against `tauri::State<'_, AppState>` and
//! `tauri::AppHandle`. In the Web backend:
//! - `State<'_, T>` is just `&T` (an `Arc<AppState>` derefs to `&AppState`).
//! - `AppHandle` is replaced by an `Arc<AppState>` plus an `EventBus` where
//!   event emission is required. To keep command signatures unchanged, we use a
//!   thin `AppCtx` carrying both.

use crate::events::EventBus;
use crate::state::AppState;
use std::ops::Deref;
use std::sync::Arc;

/// State shim — mirrors the subset of `tauri::State` that commands use.
/// The vast majority of command bodies call `state.with_store(...)` etc.,
/// which `&AppState` supports directly via `Deref`.
#[derive(Clone, Copy)]
pub struct State<'a, T> {
    inner: &'a T,
}

impl<'a, T> State<'a, T> {
    pub fn new(inner: &'a T) -> Self {
        Self { inner }
    }
}

impl<T> Deref for State<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.inner
    }
}

/// Application context passed to commands: shared state + event bus.
/// Replaces `tauri::AppHandle` in command signatures.
#[derive(Clone)]
pub struct AppCtx {
    state: Arc<AppState>,
    bus: EventBus,
}

impl AppCtx {
    pub fn new(state: Arc<AppState>, bus: EventBus) -> Self {
        Self { state, bus }
    }

    pub fn state(&self) -> &Arc<AppState> {
        &self.state
    }

    pub fn app_state(&self) -> &AppState {
        &self.state
    }

    pub fn bus(&self) -> &EventBus {
        &self.bus
    }

    /// Mirrors `app.path().resource_dir()` used in the Tauri version.
    pub fn resource_dir(&self) -> Option<&std::path::Path> {
        self.state.resource_dir.as_deref()
    }
}

impl Deref for AppCtx {
    type Target = AppState;
    fn deref(&self) -> &AppState {
        &self.state
    }
}
