mod ib;
#[cfg(feature = "moomoo-python")]
mod moomoo;

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::relay::{OptionSignalInput, RelayReceipt};
use crate::runtime_env::{env_string, load_dotenv};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BrokerKind {
    #[serde(alias = "ib")]
    Ibkr,
    #[serde(alias = "futu")]
    Moomoo,
}

impl BrokerKind {
    pub fn as_id(&self) -> &'static str {
        match self {
            Self::Ibkr => "ibkr",
            Self::Moomoo => "moomoo",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Ibkr => "IBKR",
            Self::Moomoo => "Moomoo",
        }
    }
}

impl Default for BrokerKind {
    fn default() -> Self {
        Self::Ibkr
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BrokerConfig {
    pub broker: BrokerKind,
    pub default_quantity: f64,
    pub host: String,
    pub port: u16,
    pub client_id: i32,
    pub account: String,
    pub default_exchange: String,
    pub currency: String,
    pub moomoo_host: String,
    pub moomoo_port: u16,
    pub moomoo_market: String,
    pub moomoo_trd_env: String,
    pub moomoo_acc_id: i64,
    pub moomoo_security_firm: String,
    pub moomoo_time_in_force: String,
    pub moomoo_session: String,
    pub moomoo_fill_outside_rth: bool,
    pub dry_run: bool,
    pub auto_forward: bool,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        load_dotenv();

        Self {
            broker: env_broker_kind().unwrap_or(BrokerKind::Ibkr),
            default_quantity: env_parse("OPTIONS_RELAY_DEFAULT_QUANTITY").unwrap_or(1.0),
            host: env_string("IB_GATEWAY_HOST").unwrap_or_else(|| String::from("127.0.0.1")),
            port: env_parse("IB_GATEWAY_PORT").unwrap_or(4002),
            client_id: env_parse("IB_GATEWAY_CLIENT_ID").unwrap_or(100),
            account: env_string("IB_GATEWAY_ACCOUNT").unwrap_or_default(),
            default_exchange: env_string("IB_GATEWAY_DEFAULT_EXCHANGE")
                .map(|value| value.to_uppercase())
                .unwrap_or_else(|| String::from("SMART")),
            currency: env_string("IB_GATEWAY_CURRENCY")
                .map(|value| value.to_uppercase())
                .unwrap_or_else(|| String::from("USD")),
            moomoo_host: env_string("MOOMOO_HOST").unwrap_or_else(|| String::from("127.0.0.1")),
            moomoo_port: env_parse("MOOMOO_PORT").unwrap_or(11111),
            moomoo_market: env_string("MOOMOO_MARKET")
                .map(|value| value.to_uppercase())
                .unwrap_or_else(|| String::from("US")),
            moomoo_trd_env: env_string("MOOMOO_TRD_ENV")
                .map(|value| value.to_uppercase())
                .unwrap_or_else(|| String::from("SIMULATE")),
            moomoo_acc_id: env_parse("MOOMOO_ACC_ID").unwrap_or(0),
            moomoo_security_firm: env_string("MOOMOO_SECURITY_FIRM")
                .map(|value| value.to_uppercase())
                .unwrap_or_else(|| String::from("FUTUSECURITIES")),
            moomoo_time_in_force: env_string("MOOMOO_TIME_IN_FORCE")
                .map(|value| value.to_uppercase())
                .unwrap_or_else(|| String::from("DAY")),
            moomoo_session: env_string("MOOMOO_SESSION")
                .map(|value| value.to_uppercase())
                .unwrap_or_else(|| String::from("NONE")),
            moomoo_fill_outside_rth: env_bool("MOOMOO_FILL_OUTSIDE_RTH").unwrap_or(false),
            dry_run: env_bool("IB_GATEWAY_DRY_RUN").unwrap_or(true),
            auto_forward: env_bool("IB_GATEWAY_AUTO_FORWARD").unwrap_or(true),
        }
    }
}

fn env_parse<T>(key: &str) -> Option<T>
where
    T: FromStr,
{
    env_string(key).and_then(|value| value.parse::<T>().ok())
}

fn env_bool(key: &str) -> Option<bool> {
    env_string(key).and_then(|value| match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    })
}

fn env_broker_kind() -> Option<BrokerKind> {
    env_string("OPTIONS_RELAY_BROKER").and_then(|value| match value.to_ascii_lowercase().as_str() {
        "ib" | "ibkr" => Some(BrokerKind::Ibkr),
        "futu" | "moomoo" => Some(BrokerKind::Moomoo),
        _ => None,
    })
}

impl BrokerConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.default_quantity <= 0.0 {
            return Err(String::from("Default Quantity 必须大于 0"));
        }

        match self.broker {
            BrokerKind::Ibkr => validate_ib_config(self),
            BrokerKind::Moomoo => validate_moomoo_config(self),
        }
    }

    pub fn active_broker_id(&self) -> &'static str {
        self.broker.as_id()
    }

    pub fn active_broker_label(&self) -> &'static str {
        self.broker.display_name()
    }

    pub fn active_target_summary(&self) -> String {
        match self.broker {
            BrokerKind::Ibkr => format!("IB Gateway {}:{}", self.host.trim(), self.port),
            BrokerKind::Moomoo => {
                format!("Moomoo OpenD {}:{}", self.moomoo_host.trim(), self.moomoo_port)
            }
        }
    }

    pub fn queued_note(&self) -> String {
        if self.auto_forward {
            format!(
                "信号已入队，准备立即转发到 {}",
                self.active_broker_label()
            )
        } else {
            format!(
                "信号已入队，等待手动开启 Auto Relay ({})",
                self.active_broker_label()
            )
        }
    }

    pub fn forwarding_note(&self) -> String {
        if self.dry_run {
            format!(
                "Dry-run 已接管，正在模拟 {} 下单路径",
                self.active_broker_label()
            )
        } else {
            format!("正在连接 {} 并提交订单", self.active_target_summary())
        }
    }
}

pub async fn relay_signal(
    signal: &OptionSignalInput,
    config: &BrokerConfig,
) -> Result<RelayReceipt, String> {
    config.validate()?;

    match config.broker {
        BrokerKind::Ibkr => ib::relay(signal, config).await,
        BrokerKind::Moomoo => relay_moomoo_signal(signal, config).await,
    }
}

#[cfg(feature = "moomoo-python")]
async fn relay_moomoo_signal(
    signal: &OptionSignalInput,
    config: &BrokerConfig,
) -> Result<RelayReceipt, String> {
    moomoo::relay(signal, config).await
}

#[cfg(not(feature = "moomoo-python"))]
async fn relay_moomoo_signal(
    _signal: &OptionSignalInput,
    _config: &BrokerConfig,
) -> Result<RelayReceipt, String> {
    Err(String::from(
        "当前构建未启用 Moomoo Python bridge，请使用带 moomoo-python feature 的构建版本",
    ))
}

fn validate_ib_config(config: &BrokerConfig) -> Result<(), String> {
    if config.host.trim().is_empty() {
        return Err(String::from("IB Gateway host 不能为空"));
    }

    if config.port == 0 {
        return Err(String::from("IB Gateway port 必须大于 0"));
    }

    Ok(())
}

fn validate_moomoo_config(config: &BrokerConfig) -> Result<(), String> {
    #[cfg(not(feature = "moomoo-python"))]
    {
        let _ = config;

        return Err(String::from(
            "当前构建未启用 Moomoo Python bridge，请使用带 moomoo-python feature 的构建版本",
        ));
    }

    #[cfg(feature = "moomoo-python")]
    {
    if config.moomoo_host.trim().is_empty() {
        return Err(String::from("Moomoo OpenD host 不能为空"));
    }

    if config.moomoo_port == 0 {
        return Err(String::from("Moomoo OpenD port 必须大于 0"));
    }

    if config.moomoo_market.trim().is_empty() {
        return Err(String::from("Moomoo market 不能为空"));
    }

    match config.moomoo_trd_env.trim().to_ascii_uppercase().as_str() {
        "SIMULATE" | "REAL" => {}
        _ => {
            return Err(String::from(
                "Moomoo trd env 只支持 SIMULATE 或 REAL",
            ))
        }
    }

    if config.moomoo_time_in_force.trim().is_empty() {
        return Err(String::from("Moomoo time in force 不能为空"));
    }

    if config.moomoo_session.trim().is_empty() {
        return Err(String::from("Moomoo session 不能为空"));
    }

    Ok(())
    }
}