#[cfg(target_os = "windows")]
use std::fs;
#[cfg(target_os = "windows")]
use std::path::{Path, PathBuf};

use pyo3::prelude::*;
use pyo3::types::PyModule;
use serde::{Deserialize, Serialize};

use crate::broker::BrokerConfig;
use crate::relay::{validate_signal, OptionSignalInput, RelayReceipt};
use crate::runtime_env::{env_string, load_dotenv};

const MOOMOO_BRIDGE_CODE: &str = include_str!("moomoo_bridge.py");

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MoomooBridgeConfig<'a> {
    host: &'a str,
    port: u16,
    market: &'a str,
    trd_env: &'a str,
    acc_id: i64,
    security_firm: &'a str,
    time_in_force: &'a str,
    session: &'a str,
    fill_outside_rth: bool,
    trade_password: Option<String>,
    trade_password_md5: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoomooBridgeResult {
    order_id: Option<String>,
    message: String,
    simulated: bool,
}

pub async fn relay(
    signal: &OptionSignalInput,
    config: &BrokerConfig,
) -> Result<RelayReceipt, String> {
    validate_signal(signal)?;

    if config.dry_run {
        return Ok(RelayReceipt {
            broker: String::from("moomoo"),
            order_id: None,
            message: format!(
                "dry-run: 已为 {} {} {} {}{} 生成 Moomoo 期权下单请求",
                signal.side.as_label(),
                signal.quantity,
                signal.symbol.trim().to_uppercase(),
                signal.strike,
                signal.option_type.as_suffix(),
            ),
            simulated: true,
        });
    }

    let bridge_config = build_bridge_config(config);
    let config_json = serde_json::to_string(&bridge_config).map_err(|error| error.to_string())?;
    let signal_json = serde_json::to_string(signal).map_err(|error| error.to_string())?;

    let result_json = tokio::task::spawn_blocking(move || call_python_bridge(config_json, signal_json))
        .await
        .map_err(|error| format!("等待 Moomoo Python 任务失败: {error}"))??;

    let result = serde_json::from_str::<MoomooBridgeResult>(&result_json)
        .map_err(|error| format!("无法解析 Moomoo Python 返回值: {error}"))?;

    Ok(RelayReceipt {
        broker: String::from("moomoo"),
        order_id: result.order_id,
        message: result.message,
        simulated: result.simulated,
    })
}

fn build_bridge_config(config: &BrokerConfig) -> MoomooBridgeConfig<'_> {
    load_dotenv();

    MoomooBridgeConfig {
        host: config.moomoo_host.trim(),
        port: config.moomoo_port,
        market: config.moomoo_market.trim(),
        trd_env: config.moomoo_trd_env.trim(),
        acc_id: config.moomoo_acc_id,
        security_firm: config.moomoo_security_firm.trim(),
        time_in_force: config.moomoo_time_in_force.trim(),
        session: config.moomoo_session.trim(),
        fill_outside_rth: config.moomoo_fill_outside_rth,
        trade_password: env_string("MOOMOO_TRADE_PASSWORD"),
        trade_password_md5: env_string("MOOMOO_TRADE_PASSWORD_MD5"),
    }
}

fn call_python_bridge(config_json: String, signal_json: String) -> Result<String, String> {
    configure_python_runtime();

    Python::with_gil(|python| {
        let module = PyModule::from_code_bound(
            python,
            MOOMOO_BRIDGE_CODE,
            "moomoo_bridge.py",
            "moomoo_bridge",
        )
        .map_err(|error| format!("加载 Moomoo bridge Python 模块失败: {error}"))?;

        module
            .getattr("place_option_order")
            .and_then(|callable| callable.call1((config_json, signal_json)))
            .and_then(|result| result.extract::<String>())
            .map_err(|error| format!("执行 Moomoo bridge Python 逻辑失败: {error}"))
    })
}

fn configure_python_runtime() {
    #[cfg(target_os = "windows")]
    {
        let Some(runtime_dir) = resolve_windows_python_runtime_dir() else {
            return;
        };

        let runtime_lib_dir = runtime_dir.join("Lib");
        let site_packages_dir = runtime_lib_dir.join("site-packages");
        let dll_dir = runtime_dir.join("DLLs");

        std::env::set_var("PYTHONHOME", &runtime_dir);

        let python_path_entries = [
            runtime_dir.join("python312.zip"),
            runtime_lib_dir.clone(),
            site_packages_dir,
            dll_dir.clone(),
        ];

        if let Ok(joined_paths) = std::env::join_paths(
            python_path_entries
                .iter()
                .filter(|entry| entry.exists())
                .map(PathBuf::as_path),
        ) {
            std::env::set_var("PYTHONPATH", joined_paths);
        }

        prepend_windows_path([runtime_dir, dll_dir]);
    }
}

#[cfg(target_os = "windows")]
fn resolve_windows_python_runtime_dir() -> Option<PathBuf> {
    let env_override = env_string("MOOMOO_PYTHON_RUNTIME")
        .or_else(|| env_string("OPTIONS_RELAY_PYTHON_RUNTIME"))
        .map(PathBuf::from);

    for candidate in env_override.into_iter().chain(default_runtime_candidates()) {
        if is_windows_python_runtime_dir(&candidate) {
            return Some(candidate);
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn default_runtime_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(exe_dir) = current_exe.parent() {
            candidates.push(exe_dir.join("python-runtime"));
            candidates.push(exe_dir.to_path_buf());
        }
    }

    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("python-runtime"));
        candidates.push(current_dir);
    }

    candidates
}

#[cfg(target_os = "windows")]
fn is_windows_python_runtime_dir(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }

    let has_python_dll = fs::read_dir(path)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .any(|entry| {
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy().to_ascii_lowercase();
            file_name.starts_with("python3") && file_name.ends_with(".dll")
        });

    has_python_dll && path.join("Lib").is_dir()
}

#[cfg(target_os = "windows")]
fn prepend_windows_path(entries: [PathBuf; 2]) {
    let mut merged_entries = Vec::new();

    for entry in entries {
        if entry.exists() && !merged_entries.iter().any(|existing| existing == &entry) {
            merged_entries.push(entry);
        }
    }

    if let Some(existing_path) = std::env::var_os("PATH") {
        for entry in std::env::split_paths(&existing_path) {
            if !merged_entries.iter().any(|existing| existing == &entry) {
                merged_entries.push(entry);
            }
        }
    }

    if let Ok(path_value) = std::env::join_paths(merged_entries.iter().map(PathBuf::as_path)) {
        std::env::set_var("PATH", path_value);
    }
}