use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, Once};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::broker::BrokerConfig;

pub const SNAPSHOT_EVENT: &str = "relay:snapshot";

const DOTENV_SEARCH_PATHS: [&str; 3] = [".env", "../.env", "../../.env"];
const LEGACY_STATE_FILE_NAME: &str = "options-relay-state.json";
const CONFIG_FILE_NAME: &str = "options-relay-config.json";
const RUNTIME_FILE_NAME: &str = "options-relay-runtime.json";

static ENV_FILE_LOADED: Once = Once::new();

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
pub struct NatsFeedConfig {
    #[serde(alias = "wsEndpoint", alias = "ws_endpoint")]
    pub server_address: String,
    pub subject: String,
    pub queue_group: String,
    #[serde(alias = "autoConnect", alias = "auto_connect")]
    pub auto_subscribe: bool,
}

impl Default for NatsFeedConfig {
    fn default() -> Self {
        load_dotenv();

        Self {
            server_address: env_string("NATS_SERVER_ADDRESS")
                .or_else(|| env_string("NATS_WS_ENDPOINT"))
                .unwrap_or_else(|| String::from("127.0.0.1:4222")),
            subject: env_string("NATS_SUBJECT")
                .unwrap_or_else(|| String::from("signals.options.entry")),
            queue_group: env_string("NATS_QUEUE_GROUP").unwrap_or_default(),
            auto_subscribe: env_bool("NATS_AUTO_SUBSCRIBE")
                .or_else(|| env_bool("NATS_AUTO_CONNECT"))
                .unwrap_or(false),
        }
    }
}

impl NatsFeedConfig {
    pub fn validate(&self) -> Result<(), String> {
        let server_address = self.server_address.trim();

        if server_address.is_empty() {
            return Err(String::from("NATS Server 地址不能为空"));
        }

        if self.subject.trim().is_empty() {
            return Err(String::from("NATS Subject 不能为空"));
        }

        Ok(())
    }
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
        load_dotenv();

        Self {
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
            dry_run: env_bool("IB_GATEWAY_DRY_RUN").unwrap_or(true),
            auto_forward: env_bool("IB_GATEWAY_AUTO_FORWARD").unwrap_or(true),
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
    pub broker_config: BrokerConfig,
    pub nats_config: NatsFeedConfig,
    pub messages: Vec<RelayMessage>,
    pub stats: RelayStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedState {
    #[serde(default)]
    broker_config: BrokerConfig,
    #[serde(default)]
    nats_config: NatsFeedConfig,
    #[serde(default)]
    messages: Vec<RelayMessage>,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            broker_config: BrokerConfig::default(),
            nats_config: NatsFeedConfig::default(),
            messages: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedConfig {
    #[serde(default)]
    broker_config: BrokerConfig,
    #[serde(default)]
    nats_config: NatsFeedConfig,
}

impl Default for PersistedConfig {
    fn default() -> Self {
        Self {
            broker_config: BrokerConfig::default(),
            nats_config: NatsFeedConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PersistedRuntime {
    #[serde(default)]
    messages: Vec<RelayMessage>,
}

#[derive(Debug, Clone)]
struct PersistencePaths {
    base_dir: PathBuf,
    config_path: PathBuf,
    runtime_path: PathBuf,
    legacy_state_path: PathBuf,
}

pub struct AppState {
    pub config: Mutex<BrokerConfig>,
    pub nats_config: Mutex<NatsFeedConfig>,
    pub messages: Mutex<Vec<RelayMessage>>,
    next_id: AtomicU64,
    nats_generation: AtomicU64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            config: Mutex::new(BrokerConfig::default()),
            nats_config: Mutex::new(NatsFeedConfig::default()),
            messages: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            nats_generation: AtomicU64::new(0),
        }
    }
}

impl AppState {
    pub fn load() -> Self {
        let persisted = match load_persisted_state() {
            Ok(Some(state)) => state,
            Ok(None) => PersistedState::default(),
            Err(error) => {
                eprintln!("failed to load persisted relay state: {error}");
                PersistedState::default()
            }
        };

        let next_id = persisted
            .messages
            .iter()
            .map(|message| message.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);

        Self {
            config: Mutex::new(persisted.broker_config),
            nats_config: Mutex::new(persisted.nats_config),
            messages: Mutex::new(persisted.messages),
            next_id: AtomicU64::new(next_id),
            nats_generation: AtomicU64::new(0),
        }
    }

    pub fn next_message_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn advance_nats_generation(&self) -> u64 {
        self.nats_generation.fetch_add(1, Ordering::Relaxed).saturating_add(1)
    }

    pub fn nats_generation(&self) -> u64 {
        self.nats_generation.load(Ordering::Relaxed)
    }
}

pub fn current_broker_config(state: &AppState) -> Result<BrokerConfig, String> {
    state
        .config
        .lock()
        .map_err(|_| String::from("无法读取 broker 配置"))
        .map(|config| config.clone())
}

pub fn current_nats_config(state: &AppState) -> Result<NatsFeedConfig, String> {
    state
        .nats_config
        .lock()
        .map_err(|_| String::from("无法读取 NATS Feed 配置"))
        .map(|config| config.clone())
}

pub fn build_snapshot(state: &AppState) -> Result<RuntimeSnapshot, String> {
    let broker_config = current_broker_config(state)?;
    let nats_config = current_nats_config(state)?;
    let messages = state
        .messages
        .lock()
        .map_err(|_| String::from("无法读取消息队列"))?
        .clone();

    Ok(RuntimeSnapshot {
        broker_config,
        nats_config,
        stats: build_stats(&messages),
        messages,
    })
}

pub fn queue_signal(
    state: &AppState,
    signal: OptionSignalInput,
) -> Result<(RelayMessage, BrokerConfig), String> {
    let config = current_broker_config(state)?;
    let message = RelayMessage {
        id: state.next_message_id(),
        received_at: timestamp_now()?,
        signal,
        status: RelayStatus::Queued,
        relay_notes: config.queued_note(),
        receipt: None,
    };

    state
        .messages
        .lock()
        .map_err(|_| String::from("无法更新消息队列"))?
        .push(message.clone());

    Ok((message, config))
}

pub fn update_message(
    state: &AppState,
    message_id: u64,
    mutate: impl FnOnce(&mut RelayMessage),
) -> Result<(), String> {
    let mut messages = state
        .messages
        .lock()
        .map_err(|_| String::from("无法更新消息队列"))?;

    let entry = messages
        .iter_mut()
        .find(|message| message.id == message_id)
        .ok_or_else(|| format!("找不到消息 #{message_id}"))?;

    mutate(entry);
    Ok(())
}

pub fn normalize_nats_server_address(server_address: &str) -> String {
    let trimmed = server_address.trim();

    if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("nats://{trimmed}")
    }
}

pub fn signal_from_nats_payload(
    payload: &[u8],
    default_quantity: f64,
) -> Result<OptionSignalInput, String> {
    let root = serde_json::from_slice::<Value>(payload)
        .map_err(|error| format!("无法解析 NATS payload JSON: {error}"))?;

    if matches!(root.get("relayReady").and_then(Value::as_bool), Some(false)) {
        let relay_error = json_string(root.get("relayError"))
            .unwrap_or_else(|| String::from("上游标记为 relay 不可用"));
        return Err(relay_error);
    }

    let record = root.get("signal").unwrap_or(&root);
    let parsed_entry = record
        .get("parsed_entry")
        .ok_or_else(|| String::from("NATS payload 缺少 parsed_entry"))?;

    let symbol = json_string(parsed_entry.get("symbol"))
        .ok_or_else(|| String::from("NATS payload 缺少 parsed_entry.symbol"))?;
    let expiry = json_string(parsed_entry.get("expiry"))
        .ok_or_else(|| String::from("NATS payload 缺少 parsed_entry.expiry"))?;
    let strike = json_f64(parsed_entry.get("strike"))
        .ok_or_else(|| String::from("NATS payload 缺少 parsed_entry.strike"))?;
    let option_type = parse_option_type(parsed_entry.get("contract_type"))
        .ok_or_else(|| String::from("NATS payload 缺少有效的 parsed_entry.contract_type"))?;
    let category = json_string(record.get("category"))
        .unwrap_or_else(|| String::from("entry"));

    let quantity = if default_quantity > 0.0 {
        default_quantity
    } else {
        1.0
    };

    let source = json_string(record.get("author_username"))
        .or_else(|| json_string(root.get("sourceFile")))
        .or_else(|| json_string(root.get("subject")))
        .unwrap_or_else(|| String::from("discord"));
    let strategy_tag = format!("discord:{}", category.to_ascii_lowercase());
    let raw_message = json_string(record.get("content")).unwrap_or_else(|| {
        serde_json::to_string(record).unwrap_or_else(|_| String::from("{}"))
    });

    Ok(OptionSignalInput {
        source,
        strategy_tag,
        symbol,
        option_type,
        expiry,
        strike,
        side: category_to_side(&category),
        quantity,
        limit_price: json_f64(parsed_entry.get("price")),
        raw_message,
    })
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

pub fn parse_expiry(expiry: &str) -> Result<(u16, u8, u8), String> {
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
    pub fn as_suffix(&self) -> &'static str {
        match self {
            OptionType::Call => "C",
            OptionType::Put => "P",
        }
    }
}

impl OrderSide {
    pub fn as_label(&self) -> &'static str {
        match self {
            OrderSide::Buy => "BUY",
            OrderSide::Sell => "SELL",
        }
    }
}

fn build_stats(messages: &[RelayMessage]) -> RelayStats {
    let mut stats = RelayStats {
        total: messages.len(),
        ..RelayStats::default()
    };

    for message in messages {
        match message.status {
            RelayStatus::Queued => stats.queued += 1,
            RelayStatus::Forwarding => stats.forwarding += 1,
            RelayStatus::Sent => stats.sent += 1,
            RelayStatus::Failed => stats.failed += 1,
        }
    }

    stats
}

fn category_to_side(category: &str) -> OrderSide {
    match category.trim().to_ascii_lowercase().as_str() {
        "exit" => OrderSide::Sell,
        _ => OrderSide::Buy,
    }
}

fn parse_option_type(value: Option<&Value>) -> Option<OptionType> {
    match json_string(value)?.to_ascii_lowercase().as_str() {
        "call" | "calls" | "c" => Some(OptionType::Call),
        "put" | "puts" | "p" => Some(OptionType::Put),
        _ => None,
    }
}

fn json_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        _ => None,
    }
}

fn json_f64(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn load_dotenv() {
    ENV_FILE_LOADED.call_once(|| {
        for path in dotenv_candidates() {
            if dotenvy::from_path(&path).is_ok() {
                break;
            }
        }
    });
}

fn dotenv_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(current_dir) = std::env::current_dir() {
        append_dotenv_candidates(&mut candidates, &current_dir);
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(exe_dir) = current_exe.parent() {
            append_dotenv_candidates(&mut candidates, exe_dir);
        }
    }

    candidates
}

fn append_dotenv_candidates(candidates: &mut Vec<PathBuf>, base_dir: &Path) {
    for relative_path in DOTENV_SEARCH_PATHS {
        let candidate = base_dir.join(relative_path);

        if !candidates.iter().any(|existing| existing == &candidate) {
            candidates.push(candidate);
        }
    }
}

fn env_string(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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

pub fn persist_runtime_snapshot(snapshot: &RuntimeSnapshot) -> Result<(), String> {
    let paths = persistence_paths();
    let config = PersistedConfig {
        broker_config: snapshot.broker_config.clone(),
        nats_config: snapshot.nats_config.clone(),
    };
    let runtime = PersistedRuntime {
        messages: snapshot.messages.clone(),
    };

    fs::create_dir_all(&paths.base_dir).map_err(|error| {
        format!("无法创建本地状态目录 {}: {error}", paths.base_dir.display())
    })?;

    write_json_file(&paths.config_path, &config, "客户端配置文件")?;
    write_json_file(&paths.runtime_path, &runtime, "客户端运行时文件")
}

fn load_persisted_state() -> Result<Option<PersistedState>, String> {
    let paths = persistence_paths();

    if paths.config_path.exists() || paths.runtime_path.exists() {
        let config = read_json_file::<PersistedConfig>(&paths.config_path, "客户端配置文件")?
            .unwrap_or_default();
        let runtime = read_json_file::<PersistedRuntime>(&paths.runtime_path, "客户端运行时文件")?
            .unwrap_or_default();

        return Ok(Some(PersistedState {
            broker_config: config.broker_config,
            nats_config: config.nats_config,
            messages: runtime.messages,
        }));
    }

    if !paths.legacy_state_path.exists() {
        return Ok(None);
    }

    read_json_file::<PersistedState>(&paths.legacy_state_path, "旧版本地状态文件")
}

fn write_json_file<T>(path: &Path, value: &T, label: &str) -> Result<(), String>
where
    T: Serialize,
{
    let payload = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;

    fs::write(path, payload).map_err(|error| format!("无法写入{} {}: {error}", label, path.display()))
}

fn read_json_file<T>(path: &Path, label: &str) -> Result<Option<T>, String>
where
    T: DeserializeOwned,
{
    if !path.exists() {
        return Ok(None);
    }

    let payload = fs::read_to_string(path)
        .map_err(|error| format!("无法读取{} {}: {error}", label, path.display()))?;

    let value = serde_json::from_str::<T>(&payload)
        .map_err(|error| format!("无法解析{} {}: {error}", label, path.display()))?;

    Ok(Some(value))
}

fn persistence_paths() -> PersistencePaths {
    let base_dir = state_storage_dir();

    PersistencePaths {
        config_path: base_dir.join(CONFIG_FILE_NAME),
        runtime_path: base_dir.join(RUNTIME_FILE_NAME),
        legacy_state_path: base_dir.join(LEGACY_STATE_FILE_NAME),
        base_dir,
    }
}

fn state_storage_dir() -> PathBuf {
    load_dotenv();

    if let Some(home_dir) = env_string("OPTIONS_RELAY_HOME") {
        return PathBuf::from(home_dir);
    }

    if cfg!(debug_assertions) {
        if let Ok(current_dir) = std::env::current_dir() {
            return current_dir;
        }
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            return parent.to_path_buf();
        }
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}