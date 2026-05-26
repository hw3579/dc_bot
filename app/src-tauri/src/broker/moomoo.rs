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