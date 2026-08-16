use crate::runtime::ConnectionView;
use crate::state::AppState;
use crate::compat::State;
pub fn list_connections(state: State<'_, AppState>) -> Result<Vec<ConnectionView>, String> {
    let mut runtime = state.lock_runtime();
    let store = state.lock_store();
    Ok(runtime.live_connections(&store))
}
pub fn list_requests(
    state: State<'_, AppState>,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<ConnectionView>, String> {
    let mut runtime = state.lock_runtime();
    let store = state.lock_store();
    Ok(runtime.request_history(&store, query.as_deref(), limit))
}
pub fn list_request_failures(
    state: State<'_, AppState>,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<ConnectionView>, String> {
    let mut runtime = state.lock_runtime();
    let store = state.lock_store();
    Ok(runtime.request_failures(&store, query.as_deref(), limit))
}
pub fn clear_request_history(state: State<'_, AppState>) -> Result<(), String> {
    let mut runtime = state.lock_runtime();
    runtime.clear_request_history();
    Ok(())
}
