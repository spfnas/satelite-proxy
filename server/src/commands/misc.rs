//! Misc commands that lived at crate root in the Tauri version.

use crate::compat::{AppCtx, State};
use crate::domain;
use crate::state::AppState;

/// Parse a subscription text (Clash / sing-box URL or YAML) without persisting.
pub fn parse_subscription_text(content: String) -> Result<domain::ParseResult, String> {
    crate::subscription::parse_subscription(&content).map_err(|e| e.to_string())
}

/// Persist UI shell preference (pro | simple) for window size on recreate.
/// Web backend: stored under the app data dir (no window recreation, but kept
/// for parity with the desktop build).
pub fn set_ui_mode_pref(app: &AppCtx, mode: String) -> Result<(), String> {
    let dir = &app.state().app_data_dir;
    let path = dir.join("data").join("ui_mode");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let v = match mode.trim().to_ascii_lowercase().as_str() {
        "simple" => "simple",
        _ => "pro",
    };
    let _ = std::fs::write(path, v);
    Ok(())
}

/// Deep-link import URLs still waiting for the add form (None after user closes it).
pub fn peek_pending_import_urls(state: State<'_, AppState>) -> Option<Vec<String>> {
    state.peek_pending_import_urls()
}

/// User closed / finished the add-subscription dialog — do not re-open on next UI wake.
pub fn clear_pending_import_urls(state: State<'_, AppState>) {
    state.clear_pending_import_urls();
}
