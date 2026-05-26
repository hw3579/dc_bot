mod ib;
mod moomoo;

use serde::{Deserialize, Serialize};

use crate::relay::{OptionSignalInput, RelayReceipt};

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
        Self {
            broker: BrokerKind::Ibkr,
            default_quantity: 1.0,
            host: String::from("127.0.0.1"),
            port: 4002,
            client_id: 100,
            account: String::new(),
            default_exchange: String::from("SMART"),
            currency: String::from("USD"),
            moomoo_host: String::from("127.0.0.1"),
            moomoo_port: 11111,
            moomoo_market: String::from("US"),
            moomoo_trd_env: String::from("SIMULATE"),
            moomoo_acc_id: 0,
            moomoo_security_firm: String::from("FUTUSECURITIES"),
            moomoo_time_in_force: String::from("DAY"),
            moomoo_session: String::from("NONE"),
            moomoo_fill_outside_rth: false,
            dry_run: true,
            auto_forward: true,
        }
    }
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
        BrokerKind::Moomoo => moomoo::relay(signal, config).await,
    }
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