//! HTTP dispatch — routes `POST /api/{command}` to the exact same functions
//! that Tauri registers via `invoke_handler`. Arguments arrive as a JSON object
//! with snake_case keys; the frontend transport mirrors Tauri's camelCase →
//! snake_case remapping so the two surfaces stay in sync.
//!
//! Blocking commands run on the blocking pool (`run_sync`), async commands are
//! awaited directly. Results are serialized with the same serde impls Tauri
//! uses, so response shapes are identical to `invoke`.

use crate::commands;
use crate::state::AppState;
use serde::de::DeserializeOwned;
use serde_json::Value;
use crate::compat::{AppCtx, State};

fn st(app: &AppCtx) -> Result<State<'_, AppState>, String> {
    Ok(State::new(app.app_state()))
}

/// Deserialize one argument by key. Missing / null → `None` for `Option<T>`,
/// an explicit error for required `T`.
fn arg<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T, String> {
    let v = args.get(name).cloned().unwrap_or(Value::Null);
    serde_json::from_value(v).map_err(|e| format!("bad argument '{name}': {e}"))
}

/// `serde_json::Value` wrapped from a structured argument (nested objects).
fn arg_value(args: &Value, name: &str) -> Result<Value, String> {
    match args.get(name) {
        Some(v) if !v.is_null() => Ok(v.clone()),
        _ => Err(format!("missing argument '{name}'")),
    }
}

/// Run a blocking command on the blocking pool, then serialize its result.
async fn run_sync<T: serde::Serialize + Send + 'static>(
    app: AppCtx,
    args: Value,
    f: impl FnOnce(&AppCtx, &Value) -> Result<T, String> + Send + 'static,
) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || f(&app, &args))
        .await
        .map_err(|e| format!("command task failed: {e}"))?
        .map(|v| serde_json::to_value(v).map_err(|e| e.to_string()))?
}

/// Serialize a command result. Commands return `Result<T, String>`; unwrap so
/// the HTTP payload is the plain value (`T`), matching Tauri's invoke.
fn ok<T: serde::Serialize>(value: Result<T, String>) -> Result<Value, String> {
    match value {
        Ok(v) => serde_json::to_value(v).map_err(|e| e.to_string()),
        Err(e) => Err(e),
    }
}

fn err_string<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

pub async fn dispatch(app: &AppCtx, command: &str, args: Value) -> Result<Value, String> {
    match command {
        // ---------------- subscriptions ----------------
        "list_subscriptions" => {
            run_sync(app.clone(), args, |a, _| ok(commands::list_subscriptions(st(a)?))).await
        }
        "get_subscription" => run_sync(app.clone(), args, |a, args| {
            ok(commands::get_subscription(st(a)?, arg(args, "id")?))
        })
        .await,
        "add_subscription_url" => {
            let name: Option<String> = arg(&args, "name")?;
            let url: String = arg(&args, "url")?;
            let via_proxy: Option<bool> = arg(&args, "via_proxy")?;
            let auto_update: Option<bool> = arg(&args, "auto_update")?;
            let interval: Option<u32> = arg(&args, "auto_update_interval_min")?;
            ok(commands::add_subscription_url(
                st(app)?,
                name,
                url,
                via_proxy,
                auto_update,
                interval,
            )
            .await
            .map_err(err_string))
        }
        "add_subscription_file" => run_sync(app.clone(), args, |a, args| {
            ok(commands::add_subscription_file(
                st(a)?,
                arg(args, "name")?,
                arg(args, "path")?,
                arg(args, "auto_update")?,
                arg(args, "auto_update_interval_min")?,
            ))
        })
        .await,
        // Web-only: import subscription content uploaded from the browser.
        // (add_subscription_yaml was never implemented in commands::subscription;
        //  Web clients upload a file via add_subscription_file instead.)
        "add_subscription_yaml" => Err(
            "add_subscription_yaml is not implemented; use add_subscription_file".into(),
        ),
        "add_subscription_content" => run_sync(app.clone(), args, |a, args| {
            ok(commands::add_subscription_content(
                st(a)?,
                arg(args, "name")?,
                arg(args, "content")?,
                arg(args, "auto_update")?,
                arg(args, "auto_update_interval_min")?,
            ))
        })
        .await,
        "update_subscription" => {
            let s = st(app)?;
            ok(commands::update_subscription(
                s,
                arg(&args, "id")?,
                arg(&args, "name")?,
                arg(&args, "kind")?,
                arg(&args, "url")?,
                arg(&args, "path")?,
                arg(&args, "content")?,
                arg(&args, "via_proxy")?,
                arg(&args, "auto_update")?,
                arg(&args, "auto_update_interval_min")?,
            )
            .await
            .map_err(err_string))
        }
        "refresh_subscription" => {
            let s = st(app)?;
            ok(commands::refresh_subscription(s, arg(&args, "id")?, arg(&args, "via_proxy")?)
                .await
                .map_err(err_string))
        }
        "remove_subscription" => run_sync(app.clone(), args, |a, args| {
            ok(commands::remove_subscription(st(a)?, arg(args, "id")?))
        })
        .await,
        "list_subscription_nodes" => run_sync(app.clone(), args, |a, args| {
            ok(commands::list_subscription_nodes(st(a)?, arg(args, "id")?))
        })
        .await,
        "activate_subscription" => run_sync(app.clone(), args, |a, args| {
            ok(commands::activate_subscription(st(a)?, arg(args, "id")?))
        })
        .await,
        "set_mix_mode" => run_sync(app.clone(), args, |a, args| {
            ok(commands::set_mix_mode(st(a)?, arg(args, "mix")?))
        })
        .await,

        // ---------------- settings / config ----------------
        "get_settings" => run_sync(app.clone(), args, |a, _| ok(commands::get_settings(st(a)?))).await,
        "update_settings" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::update_settings(
                a,
                s,
                arg(args, "mixed_port")?,
                arg(args, "api_port")?,
                arg(args, "probe_url")?,
                arg(args, "tun_enabled")?,
                arg(args, "tun_stack")?,
                arg(args, "transparent_enabled")?,
                arg(args, "transparent_tcp_port")?,
                arg(args, "transparent_udp_port")?,
                arg(args, "close_to_tray")?,
                arg(args, "launch_at_login")?,
                arg(args, "silent_start")?,
                arg(args, "auto_start_proxy")?,
                arg(args, "close_connections_on_switch")?,
                arg(args, "locale")?,
                arg(args, "theme")?,
                arg(args, "accent")?,
                arg(args, "tray_icon")?,
                arg(args, "unload_ui_on_tray")?,
                arg(args, "smart_switch")?,
                arg(args, "auto_select")?,
                arg(args, "route_final")?,
                arg(args, "find_process")?,
            ))
        })
        .await,
        "set_current_node" => run_sync(app.clone(), args, |a, args| {
            ok(commands::set_current_node(st(a)?, arg(args, "node_id")?))
        })
        .await,
        "list_all_nodes" => run_sync(app.clone(), args, |a, _| ok(commands::list_all_nodes(st(a)?))).await,
        "generate_singbox_config" => {
            run_sync(app.clone(), args, |a, _| ok(commands::generate_singbox_config(st(a)?))).await
        }
        "preview_singbox_config" => {
            run_sync(app.clone(), args, |a, _| ok(commands::preview_singbox_config(st(a)?))).await
        }
        "get_active_config_path" => {
            run_sync(app.clone(), args, |a, _| ok(commands::get_active_config_path(st(a)?))).await
        }

        // ---------------- core ----------------
        "get_core_info" => run_sync(app.clone(), args, |a, _| {
            let s = st(a)?;
            ok(commands::get_core_info(a, s))
        })
        .await,
        "check_core_update" => {
            let local_version: Option<String> = arg(&args, "local_version")?;
            ok(commands::check_core_update(local_version)
                .await
                .map_err(err_string))
        }
        "download_core" => {
            let tag: Option<String> = arg(&args, "tag")?;
            ok(commands::download_core(st(app)?, tag)
                .await
                .map_err(err_string)
                .map(Result::<_, String>::Ok)?)
        }
        "fetch_core_latest" => {
            ok(commands::fetch_core_latest()
                .await
                .map_err(err_string)
                .map(Result::<_, String>::Ok)?)
        }

        // ---------------- proxy runtime ----------------
        "get_proxy_status" => run_sync(app.clone(), args, |a, _| {
            let s = st(a)?;
            ok(commands::get_proxy_status(a, s))
        })
        .await,
        "start_proxy" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::start_proxy(
                a,
                s,
                arg(args, "enable_system_proxy")?,
            ))
        })
        .await,
        "stop_proxy" => run_sync(app.clone(), args, |a, _| {
            let s = st(a)?;
            ok(commands::stop_proxy(s))
        })
        .await,
        "restart_proxy" => run_sync(app.clone(), args, |a, _| {
            let s = st(a)?;
            ok(commands::restart_proxy(a, s))
        })
        .await,
        "set_system_proxy" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::set_system_proxy(a, s, arg(args, "enabled")?))
        })
        .await,
        "set_tun_enabled" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::set_tun_enabled(a, s, arg(args, "enabled")?))
        })
        .await,
        "set_transparent_enabled" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::set_transparent_enabled(a, s, arg(args, "enabled")?))
        })
        .await,
        "set_capture_mode" => {
            let mode: String = arg(&args, "mode")?;
            ok(commands::set_capture_mode(app, mode)
                .await
                .map_err(err_string))
        }
        "set_outbound_mode" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::set_outbound_mode(a, s, arg(args, "mode")?))
        })
        .await,
        "set_current_node_live" => run_sync(app.clone(), args, |a, args| {
            ok(commands::set_current_node_live(st(a)?, arg(args, "node_id")?))
        })
        .await,
        "smart_switch_now" => {
            let s = st(app)?;
            ok(commands::smart_switch_now(s).await.map_err(err_string))
        }

        // ---------------- DNS ----------------
        "get_dns_settings" => run_sync(app.clone(), args, |a, _| ok(commands::get_dns_settings(st(a)?))).await,
        "update_dns_settings" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            let settings: crate::domain::DnsSettings =
                serde_json::from_value(arg_value(args, "settings")?)
                    .map_err(|e| format!("bad settings: {e}"))?;
            let apply: Option<bool> = arg(args, "apply")?;
            ok(commands::update_dns_settings(a, s, settings, apply))
        })
        .await,
        "reset_dns_defaults" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::reset_dns_defaults(
                a,
                s,
                arg(args, "section")?,
                arg(args, "apply")?,
            ))
        })
        .await,
        "test_dns_lookup" => run_sync(app.clone(), args, |a, args| {
            ok(commands::test_dns_lookup(st(a)?, arg(args, "domain")?))
        })
        .await,
        "read_system_hosts" => run_sync(app.clone(), args, |_, _| {
            ok(Ok(commands::read_system_hosts()))
        })
        .await,

        // ---------------- logs ----------------
        "list_app_logs" => run_sync(app.clone(), args, |_, args| {
            ok(commands::list_app_logs(
                arg(args, "min_level")?,
                arg(args, "limit")?,
                arg(args, "query")?,
            ))
        })
        .await,
        "clear_app_logs" => run_sync(app.clone(), args, |_, _| ok(commands::clear_app_logs())).await,

        // ---------------- latency ----------------
        "test_nodes_latency" => {
            let s = st(app)?;
            let ids: Option<Vec<String>> = arg(&args, "ids")?;
            let timeout_ms: Option<u64> = arg(&args, "timeout_ms")?;
            ok(commands::test_nodes_latency(s, ids, timeout_ms)
                .await
                .map_err(err_string))
        }

        // ---------------- connections ----------------
        "list_connections" => run_sync(app.clone(), args, |a, _| ok(commands::list_connections(st(a)?))).await,
        "list_requests" => run_sync(app.clone(), args, |a, args| {
            ok(commands::list_requests(
                st(a)?,
                arg(args, "query")?,
                arg(args, "limit")?,
            ))
        })
        .await,
        "list_request_failures" => run_sync(app.clone(), args, |a, args| {
            ok(commands::list_request_failures(
                st(a)?,
                arg(args, "query")?,
                arg(args, "limit")?,
            ))
        })
        .await,
        "clear_request_history" => {
            run_sync(app.clone(), args, |a, _| ok(commands::clear_request_history(st(a)?))).await
        }

        // ---------------- rule sets ----------------
        "list_rule_sets" => run_sync(app.clone(), args, |a, _| ok(commands::list_rule_sets(st(a)?))).await,
        "get_rule_set" => run_sync(app.clone(), args, |a, args| {
            ok(commands::get_rule_set(st(a)?, arg(args, "id")?))
        })
        .await,
        "list_remote_rule_items" => {
            let s = st(app)?;
            ok(commands::list_remote_rule_items(
                app,
                s,
                arg(&args, "id")?,
                arg(&args, "offset")?,
                arg(&args, "limit")?,
                arg(&args, "query")?,
            )
            .await
            .map_err(err_string))
        }
        "set_active_rule_set" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::set_active_rule_set(a, s, arg(args, "id")?))
        })
        .await,
        "set_rule_set_enabled" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::set_rule_set_enabled(
                a,
                s,
                arg(args, "id")?,
                arg(args, "enabled")?,
            ))
        })
        .await,
        "set_rule_set_strategy" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::set_rule_set_strategy(
                a,
                s,
                arg(args, "id")?,
                arg(args, "strategy")?,
            ))
        })
        .await,
        "set_rule_set_dns_strategy" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::set_rule_set_dns_strategy(
                a,
                s,
                arg(args, "id")?,
                arg(args, "strategy")?,
            ))
        })
        .await,
        "reorder_rule_sets" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::reorder_rule_sets(a, s, arg(args, "ids")?))
        })
        .await,
        "create_rule_set" => run_sync(app.clone(), args, |a, args| {
            ok(commands::create_rule_set(
                st(a)?,
                arg(args, "name")?,
                arg(args, "remote_url")?,
                arg(args, "target")?,
                arg(args, "update_interval")?,
            ))
        })
        .await,
        "update_rule_set" => run_sync(app.clone(), args, |a, args| {
            ok(commands::update_rule_set(
                st(a)?,
                arg(args, "id")?,
                arg(args, "name")?,
                arg(args, "remote_url")?,
                arg(args, "update_interval")?,
            ))
        })
        .await,
        "refresh_remote_rule_set" => {
            let id: String = arg(&args, "id")?;
            ok(commands::refresh_remote_rule_set(app, id)
                .await
                .map_err(err_string))
        }
        "delete_rule_set" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::delete_rule_set(a, s, arg(args, "id")?))
        })
        .await,
        "reset_rule_set" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::reset_rule_set(a, s, arg(args, "id")?))
        })
        .await,
        "reset_builtin_rule_set" => run_sync(app.clone(), args, |a, _| {
            let s = st(a)?;
            ok(commands::reset_builtin_rule_set(a, s))
        })
        .await,

        // ---------------- rules ----------------
        "list_rules" => run_sync(app.clone(), args, |a, args| {
            ok(commands::list_rules(st(a)?, arg(args, "set_id")?))
        })
        .await,
        "save_rule" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            let input: commands::SaveRuleInput =
                serde_json::from_value(arg_value(args, "input")?)
                    .map_err(|e| format!("bad input: {e}"))?;
            ok(commands::save_rule(a, s, input))
        })
        .await,
        "remove_rule" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::remove_rule(
                a,
                s,
                arg(args, "id")?,
                arg(args, "set_id")?,
            ))
        })
        .await,
        "set_rule_enabled" => run_sync(app.clone(), args, |a, args| {
            let s = st(a)?;
            ok(commands::set_rule_enabled(
                a,
                s,
                arg(args, "id")?,
                arg(args, "enabled")?,
                arg(args, "set_id")?,
            ))
        })
        .await,

        // ---------------- lib-level commands ----------------
        "parse_subscription_text" => run_sync(app.clone(), args, |_, args| {
            ok(commands::parse_subscription_text(arg(args, "content")?))
        })
        .await,
        "set_ui_mode_pref" => run_sync(app.clone(), args, |a, args| {
            ok(commands::set_ui_mode_pref(a, arg(args, "mode")?))
        })
        .await,
        "peek_pending_import_urls" => {
            run_sync(app.clone(), args, |a, _| {
                ok(Ok(commands::peek_pending_import_urls(st(a)?)))
            })
            .await
        }
        "clear_pending_import_urls" => run_sync(app.clone(), args, |a, _| {
            commands::clear_pending_import_urls(st(a)?);
            ok(Ok(()))
        })
        .await,

        _ => Err(format!("unknown command: {command}")),
    }
}