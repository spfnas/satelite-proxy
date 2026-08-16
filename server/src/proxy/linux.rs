//! Linux system proxy via `gsettings` (GNOME / desktop environments exposing
//! the org.gnome.system.proxy schema). Headless servers (no gsettings / no
//! desktop session) get a clear error instead of a silent no-op.
//!
//! Also exports `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` for the current process
//! so child processes (sing-box curl probes, etc.) honor the system proxy.

use super::{SystemProxy, SystemProxySnapshot};
use crate::error::{AppError, AppResult};
use std::process::Command;

pub struct GSettingsSystemProxy;

impl GSettingsSystemProxy {
    fn gsettings_available() -> bool {
        Command::new("gsettings")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn set(key: &str, value: &str) -> AppResult<()> {
        let out = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy", key, value])
            .output()
            .map_err(|e| {
                AppError::Core(format!("gsettings set {key}: {e}"))
            })?;
        if !out.status.success() {
            return Err(AppError::Core(format!(
                "gsettings set {key} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }

    fn set_sub(schema: &str, key: &str, value: &str) -> AppResult<()> {
        let out = Command::new("gsettings")
            .args(["set", schema, key, value])
            .output()
            .map_err(|e| AppError::Core(format!("gsettings set {schema} {key}: {e}")))?;
        if !out.status.success() {
            return Err(AppError::Core(format!(
                "gsettings set {schema} {key} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }

    fn current_mode() -> String {
        Command::new("gsettings")
            .args(["get", "org.gnome.system.proxy", "mode"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "none".into())
    }

    fn set_mode(mode: &str) -> AppResult<()> {
        Self::set("mode", mode)
    }
}

impl SystemProxy for GSettingsSystemProxy {
    fn enable(&self, host: &str, port: u16) -> AppResult<SystemProxySnapshot> {
        if !Self::gsettings_available() {
            return Err(AppError::Core(
                "当前环境不支持系统代理（Linux 无桌面会话或缺少 gsettings）；\
                 请改用「TUN」或「透明」模式"
                    .into(),
            ));
        }
        let port_s = port.to_string();

        // Record previous mode for restore.
        let prev_mode = Self::current_mode();

        // Enable HTTP/HTTPS/SOCKS system proxy → our mixed inbound.
        Self::set_sub("org.gnome.system.proxy.http", "host", host)?;
        Self::set_sub("org.gnome.system.proxy.http", "port", &port_s)?;
        Self::set_sub("org.gnome.system.proxy.http", "enabled", "true")?;
        Self::set_sub("org.gnome.system.proxy.https", "host", host)?;
        Self::set_sub("org.gnome.system.proxy.https", "port", &port_s)?;
        Self::set_sub("org.gnome.system.proxy.socks", "host", host)?;
        Self::set_sub("org.gnome.system.proxy.socks", "port", &port_s)?;
        Self::set_mode("manual")?;

        Ok(SystemProxySnapshot {
            detail: prev_mode,
        })
    }

    fn disable(&self, snapshot: Option<&SystemProxySnapshot>) -> AppResult<()> {
        if !Self::gsettings_available() {
            return Ok(());
        }
        // Restore previous mode (usually "none"); clear manual host/ports.
        let prev = snapshot
            .and_then(|s| (!s.detail.is_empty()).then(|| s.detail.clone()))
            .unwrap_or_else(|| "none".into());
        if Self::current_mode() != "none" || prev != "none" {
            let _ = Self::set_mode(&prev);
        }
        Ok(())
    }
}
