use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use ibapi::orders::{order_builder, Action};
use ibapi::prelude::{Client, Contract, Currency, Exchange};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const SNAPSHOT_EVENT: &str = "relay:snapshot";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OptionType {
    Call,
    Put,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionSignalInput {
    pub source: String,
    pub strategy_tag: String,
    pub symbol: String,
    pub option_type: OptionType,
    pub expiry: String,
    pub strike: f64,
    pub side: OrderSide,
    pub quantity: f64,
    pub limit_price: Option<f64>,
    pub raw_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IbGatewayConfig {
    pub host: String,
    pub port: u16,
    pub client_id: i32,
    pub account: String,
    pub default_exchange: String,
    pub currency: String,
    pub dry_run: bool,
    pub auto_forward: bool,
}

impl Default for IbGatewayConfig {
    fn default() -> Self {
        Self {
            host: String::from("127.0.0.1"),
            port: 4002,
            client_id: 100,
            account: String::new(),
            default_exchange: String::from("SMART"),
            currency: String::from("USD"),
            dry_run: true,
            auto_forward: true,
        }
    }
}

impl IbGatewayConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.host.trim().is_empty() {
            return Err(String::from("IB Gateway host 不能为空"));
        }

        if self.port == 0 {
            return Err(String::from("IB Gateway port 必须大于 0"));
        }

        Ok(())
    }

    pub fn connection_url(&self) -> String {
        format!("{}:{}", self.host.trim(), self.port)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RelayStatus {
    Queued,
    Forwarding,
    Sent,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayReceipt {
    pub broker: String,
    pub order_id: Option<String>,
    pub message: String,
    pub simulated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayMessage {
    pub id: u64,
    pub received_at: String,
    pub signal: OptionSignalInput,
    pub status: RelayStatus,
    pub relay_notes: String,
    pub receipt: Option<RelayReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RelayStats {
    pub total: usize,
    pub queued: usize,
    pub forwarding: usize,
    pub sent: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub broker_config: IbGatewayConfig,
    pub messages: Vec<RelayMessage>,
    pub stats: RelayStats,
}

pub struct AppState {
    pub config: Mutex<IbGatewayConfig>,
    pub messages: Mutex<Vec<RelayMessage>>,
    next_id: AtomicU64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            config: Mutex::new(IbGatewayConfig::default()),
            messages: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }
}

impl AppState {
    pub fn next_message_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

pub fn validate_signal(signal: &OptionSignalInput) -> Result<(), String> {
    if signal.source.trim().is_empty() {
        return Err(String::from("Source 不能为空"));
    }

    if signal.strategy_tag.trim().is_empty() {
        return Err(String::from("Strategy Tag 不能为空"));
    }

    if signal.symbol.trim().is_empty() {
        return Err(String::from("Symbol 不能为空"));
    }

    if signal.strike <= 0.0 {
        return Err(String::from("Strike 必须大于 0"));
    }

    if signal.quantity <= 0.0 {
        return Err(String::from("Quantity 必须大于 0"));
    }

    if let Some(limit_price) = signal.limit_price {
        if limit_price <= 0.0 {
            return Err(String::from("Limit Price 必须大于 0"));
        }
    }

    parse_expiry(&signal.expiry)?;
    Ok(())
}

pub fn timestamp_now() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| error.to_string())
}

pub async fn relay_to_ib(
    signal: &OptionSignalInput,
    config: &IbGatewayConfig,
) -> Result<RelayReceipt, String> {
    config.validate()?;
    validate_signal(signal)?;

    let contract = build_contract(signal, config)?;
    let mut order = build_order(signal);

    if !config.account.trim().is_empty() {
        order.account = config.account.trim().to_string();
    }

    if config.dry_run {
        return Ok(RelayReceipt {
            broker: String::from("ibkr"),
            order_id: None,
            message: format!(
                "dry-run: 已生成 {} {} {} {}{} 合约与订单",
                signal.side.as_label(),
                signal.quantity,
                signal.symbol.trim().to_uppercase(),
                signal.strike,
                signal.option_type.as_suffix(),
            ),
            simulated: true,
        });
    }

    let connection_url = config.connection_url();
    let client = Client::connect(connection_url.as_str(), config.client_id)
        .await
        .map_err(|error| format!("连接 IB Gateway 失败: {error}"))?;

    let order_id = client.next_order_id();
    client
        .place_order(order_id, &contract, &order)
        .await
        .map_err(|error| format!("提交订单失败: {error}"))?;

    Ok(RelayReceipt {
        broker: String::from("ibkr"),
        order_id: Some(order_id.to_string()),
        message: format!(
            "订单已提交到 {}，order id = {}",
            connection_url, order_id
        ),
        simulated: false,
    })
}

fn build_contract(signal: &OptionSignalInput, config: &IbGatewayConfig) -> Result<Contract, String> {
    let (year, month, day) = parse_expiry(&signal.expiry)?;

    let mut contract = match signal.option_type {
        OptionType::Call => Contract::call(signal.symbol.trim().to_uppercase())
            .strike(signal.strike)
            .expires_on(year, month, day)
            .build(),
        OptionType::Put => Contract::put(signal.symbol.trim().to_uppercase())
            .strike(signal.strike)
            .expires_on(year, month, day)
            .build(),
    };

    contract.exchange = Exchange::from(config.default_exchange.trim());
    contract.currency = Currency::from(config.currency.trim());

    if contract.multiplier.is_empty() {
        contract.multiplier = String::from("100");
    }

    Ok(contract)
}

fn build_order(signal: &OptionSignalInput) -> ibapi::orders::Order {
    let action = match signal.side {
        OrderSide::Buy => Action::Buy,
        OrderSide::Sell => Action::Sell,
    };

    match signal.limit_price {
        Some(limit_price) => order_builder::limit_order(action, signal.quantity, limit_price),
        None => order_builder::market_order(action, signal.quantity),
    }
}

fn parse_expiry(expiry: &str) -> Result<(u16, u8, u8), String> {
    let mut parts = expiry.trim().split('-');

    let year = parts
        .next()
        .ok_or_else(|| String::from("Expiry 需要使用 YYYY-MM-DD 格式"))?
        .parse::<u16>()
        .map_err(|_| String::from("Expiry 年份无效"))?;

    let month = parts
        .next()
        .ok_or_else(|| String::from("Expiry 需要使用 YYYY-MM-DD 格式"))?
        .parse::<u8>()
        .map_err(|_| String::from("Expiry 月份无效"))?;

    let day = parts
        .next()
        .ok_or_else(|| String::from("Expiry 需要使用 YYYY-MM-DD 格式"))?
        .parse::<u8>()
        .map_err(|_| String::from("Expiry 日期无效"))?;

    if parts.next().is_some() {
        return Err(String::from("Expiry 需要使用 YYYY-MM-DD 格式"));
    }

    Ok((year, month, day))
}

impl OptionType {
    fn as_suffix(&self) -> &'static str {
        match self {
            OptionType::Call => "C",
            OptionType::Put => "P",
        }
    }
}

impl OrderSide {
    fn as_label(&self) -> &'static str {
        match self {
            OrderSide::Buy => "BUY",
            OrderSide::Sell => "SELL",
        }
    }
}