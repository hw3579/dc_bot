use directories::ProjectDirs;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, Weekday};

use crate::broker::BrokerConfig;
use crate::runtime_env::env_string;

pub const SNAPSHOT_EVENT: &str = "relay:snapshot";

const APP_CONFIG_FILE_NAME: &str = "options-relay-config.json";
const RUNTIME_STATE_FILE_NAME: &str = "options-relay-runtime.json";
const LEGACY_STATE_FILE_NAME: &str = "options-relay-state.json";
const STORAGE_HOME_ENV: &str = "OPTIONS_RELAY_HOME";
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
        Self {
            server_address: String::from("127.0.0.1:4222"),
            subject: String::from("signals.options.entry"),
            queue_group: String::new(),
            auto_subscribe: false,
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
    pub storage_paths: ClientStoragePaths,
    pub messages: Vec<RelayMessage>,
    pub stats: RelayStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedClientConfig {
    #[serde(default)]
    broker_config: BrokerConfig,
    #[serde(default)]
    nats_config: NatsFeedConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PersistedRuntimeState {
    #[serde(default)]
    messages: Vec<RelayMessage>,
}

impl Default for PersistedClientConfig {
    fn default() -> Self {
        Self {
            broker_config: BrokerConfig::default(),
            nats_config: NatsFeedConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyPersistedState {
    #[serde(default)]
    broker_config: BrokerConfig,
    #[serde(default)]
    nats_config: NatsFeedConfig,
    #[serde(default)]
    messages: Vec<RelayMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientStoragePaths {
    pub client_config_file: String,
    pub runtime_state_file: String,
}

#[derive(Debug, Clone)]
struct ResolvedStoragePaths {
    client_config_file: PathBuf,
    runtime_state_file: PathBuf,
}

impl ResolvedStoragePaths {
    fn display_paths(&self) -> ClientStoragePaths {
        ClientStoragePaths {
            client_config_file: self.client_config_file.display().to_string(),
            runtime_state_file: self.runtime_state_file.display().to_string(),
        }
    }
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
        let (client_config, runtime_state) = match load_client_persistence() {
            Ok(Some((client_config, runtime_state))) => (client_config, runtime_state),
            Ok(None) => (
                PersistedClientConfig::default(),
                PersistedRuntimeState::default(),
            ),
            Err(error) => {
                eprintln!("failed to load persisted relay state: {error}");
                (
                    PersistedClientConfig::default(),
                    PersistedRuntimeState::default(),
                )
            }
        };

        let next_id = runtime_state
            .messages
            .iter()
            .map(|message| message.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);

        Self {
            config: Mutex::new(client_config.broker_config),
            nats_config: Mutex::new(client_config.nats_config),
            messages: Mutex::new(runtime_state.messages),
            next_id: AtomicU64::new(next_id),
            nats_generation: AtomicU64::new(0),
        }
    }

    pub fn next_message_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn advance_nats_generation(&self) -> u64 {
        self.nats_generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn nats_generation(&self) -> u64 {
        self.nats_generation.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum IncomingDiscordPayload {
    Envelope(DiscordEnvelope),
    Signal(DiscordSignalRecord),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscordEnvelope {
    #[serde(default)]
    published_at: String,
    signal: DiscordSignalRecord,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DiscordSignalRecord {
    #[serde(default)]
    timestamp: String,
    #[serde(default, alias = "authorUsername")]
    author_username: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    symbols: Vec<String>,
    #[serde(default)]
    content: String,
    #[serde(default, alias = "parsedEntry")]
    parsed_entry: Option<ParsedEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ParsedEntry {
    #[serde(default)]
    symbol: String,
    #[serde(default)]
    strike: String,
    #[serde(default, alias = "contractType")]
    contract_type: String,
    #[serde(default, alias = "expiryLabel")]
    expiry_label: String,
    #[serde(default)]
    expiry: String,
    #[serde(default)]
    price: String,
}

pub fn build_snapshot(state: &AppState) -> Result<RuntimeSnapshot, String> {
    let broker_config = state
        .config
        .lock()
        .map_err(|_| String::from("无法读取应用状态"))?
        .clone();

    let nats_config = state
        .nats_config
        .lock()
        .map_err(|_| String::from("无法读取 NATS Feed 配置"))?
        .clone();

    let messages = state
        .messages
        .lock()
        .map_err(|_| String::from("无法读取消息队列"))?
        .clone();

    let mut stats = RelayStats::default();
    stats.total = messages.len();

    for message in &messages {
        match message.status {
            RelayStatus::Queued => stats.queued += 1,
            RelayStatus::Forwarding => stats.forwarding += 1,
            RelayStatus::Sent => stats.sent += 1,
            RelayStatus::Failed => stats.failed += 1,
        }
    }

    Ok(RuntimeSnapshot {
        broker_config,
        nats_config,
        storage_paths: resolved_storage_paths().display_paths(),
        messages,
        stats,
    })
}

pub fn current_nats_config(state: &AppState) -> Result<NatsFeedConfig, String> {
    state
        .nats_config
        .lock()
        .map_err(|_| String::from("无法读取 NATS Feed 配置"))
        .map(|config| config.clone())
}

pub fn queue_signal(
    state: &AppState,
    signal: OptionSignalInput,
) -> Result<(RelayMessage, BrokerConfig), String> {
    validate_signal(&signal)?;

    let config = state
        .config
        .lock()
        .map_err(|_| String::from("无法读取 broker 配置"))?
        .clone();

    let message = RelayMessage {
        id: state.next_message_id(),
        received_at: timestamp_now()?,
        signal,
        status: RelayStatus::Queued,
        relay_notes: config.queued_note(),
        receipt: None,
    };

    {
        let mut messages = state
            .messages
            .lock()
            .map_err(|_| String::from("无法写入本地消息队列"))?;
        messages.insert(0, message.clone());
    }

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
        .map_err(|_| String::from("无法更新消息状态"))?;

    let entry = messages
        .iter_mut()
        .find(|message| message.id == message_id)
        .ok_or_else(|| format!("找不到消息 {}", message_id))?;

    mutate(entry);
    Ok(())
}

pub fn normalize_nats_server_address(server_address: &str) -> String {
    let trimmed = server_address.trim();

    if trimmed.starts_with("nats://")
        || trimmed.starts_with("tls://")
        || trimmed.starts_with("ws://")
        || trimmed.starts_with("wss://")
    {
        trimmed.to_string()
    } else {
        format!("nats://{trimmed}")
    }
}

pub fn current_broker_config(state: &AppState) -> Result<BrokerConfig, String> {
    state
        .config
        .lock()
        .map_err(|_| String::from("无法读取 broker 配置"))
        .map(|config| config.clone())
}

pub fn signal_from_nats_payload(
    payload: &[u8],
    default_quantity: f64,
) -> Result<OptionSignalInput, String> {
    let incoming = serde_json::from_slice::<IncomingDiscordPayload>(payload)
        .map_err(|error| format!("无法解析 NATS 消息 JSON: {error}"))?;

    match incoming {
        IncomingDiscordPayload::Envelope(mut envelope) => {
            if envelope.signal.timestamp.trim().is_empty() {
                envelope.signal.timestamp = envelope.published_at;
            }
            build_signal_from_record(&envelope.signal, default_quantity)
        }
        IncomingDiscordPayload::Signal(record) => build_signal_from_record(&record, default_quantity),
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

pub(crate) fn parse_expiry(expiry: &str) -> Result<(u16, u8, u8), String> {
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
    pub(crate) fn as_suffix(&self) -> &'static str {
        match self {
            OptionType::Call => "C",
            OptionType::Put => "P",
        }
    }
}

impl OrderSide {
    pub(crate) fn as_label(&self) -> &'static str {
        match self {
            OrderSide::Buy => "BUY",
            OrderSide::Sell => "SELL",
        }
    }
}

fn build_signal_from_record(
    record: &DiscordSignalRecord,
    default_quantity: f64,
) -> Result<OptionSignalInput, String> {
    let parsed_entry = record
        .parsed_entry
        .as_ref()
        .ok_or_else(|| String::from("NATS 消息缺少 parsed_entry，无法转换为期权下单信号"))?;

    let symbol = parse_symbol(parsed_entry, record)?;
    let strike = parse_decimal_field(&parsed_entry.strike, "strike")?;
    let option_type = parse_option_type(&parsed_entry.contract_type, &record.content)?;
    let expiry = if parsed_entry.expiry.trim().is_empty() {
        resolve_expiry(&record.timestamp, &parsed_entry.expiry_label)?
    } else {
        parsed_entry.expiry.trim().to_string()
    };
    let side = category_to_side(&record.category)?;
    let limit_price = parse_optional_decimal_field(&parsed_entry.price, "price")?;
    let source = if record.author_username.trim().is_empty() {
        String::from("discord")
    } else {
        record.author_username.trim().to_string()
    };
    let category = record.category.trim();

    Ok(OptionSignalInput {
        source,
        strategy_tag: format!(
            "discord:{}",
            if category.is_empty() { "entry" } else { category }
        ),
        symbol,
        option_type,
        expiry,
        strike,
        side,
        quantity: default_quantity,
        limit_price,
        raw_message: record.content.trim().to_string(),
    })
}

fn parse_symbol(parsed_entry: &ParsedEntry, record: &DiscordSignalRecord) -> Result<String, String> {
    if !parsed_entry.symbol.trim().is_empty() {
        return Ok(parsed_entry.symbol.trim().to_uppercase());
    }

    record
        .symbols
        .first()
        .map(|symbol| symbol.trim().to_uppercase())
        .filter(|symbol| !symbol.is_empty())
        .ok_or_else(|| String::from("NATS 消息缺少 symbol，无法转换为期权下单信号"))
}

fn parse_decimal_field(value: &str, field_name: &str) -> Result<f64, String> {
    let normalized = normalize_decimal_string(value);

    normalized
        .parse::<f64>()
        .map_err(|_| format!("NATS 消息中的 {field_name} 无法解析为数字"))
}

fn parse_optional_decimal_field(value: &str, field_name: &str) -> Result<Option<f64>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }

    parse_decimal_field(value, field_name).map(Some)
}

fn normalize_decimal_string(value: &str) -> String {
    let trimmed = value.trim();

    if trimmed.starts_with('.') {
        format!("0{trimmed}")
    } else {
        trimmed.to_string()
    }
}

fn parse_option_type(contract_type: &str, content: &str) -> Result<OptionType, String> {
    let normalized_contract = contract_type.trim().to_ascii_lowercase();

    if normalized_contract.contains("call") {
        return Ok(OptionType::Call);
    }

    if normalized_contract.contains("put") {
        return Ok(OptionType::Put);
    }

    for token in content
        .split(|character: char| !character.is_ascii_alphabetic())
        .filter(|token| !token.is_empty())
    {
        match token.to_ascii_lowercase().as_str() {
            "call" | "calls" => return Ok(OptionType::Call),
            "put" | "puts" => return Ok(OptionType::Put),
            _ => continue,
        }
    }

    Err(String::from(
        "NATS 消息缺少 call/put 信息，无法映射到 IB 期权方向",
    ))
}

fn category_to_side(category: &str) -> Result<OrderSide, String> {
    match category.trim().to_ascii_lowercase().as_str() {
        "" | "entry" | "add" => Ok(OrderSide::Buy),
        "exit" => Ok(OrderSide::Sell),
        "update" => Err(String::from("update 类消息不是可执行下单信号")),
        other => Err(format!("暂不支持 category={other} 的自动下单映射")),
    }
}

fn resolve_expiry(timestamp: &str, expiry_label: &str) -> Result<String, String> {
    let normalized_label = expiry_label.trim().to_ascii_lowercase();

    if normalized_label.is_empty() {
        return Err(String::from(
            "NATS 消息缺少 expiry_label，无法映射到具体到期日",
        ));
    }

    let reference_time = if timestamp.trim().is_empty() {
        OffsetDateTime::now_utc()
    } else {
        OffsetDateTime::parse(timestamp.trim(), &Rfc3339)
            .map_err(|error| format!("NATS 消息时间戳无法解析: {error}"))?
    };

    let reference_date = reference_time.date();
    let expiry_date = match normalized_label.as_str() {
        "0dte" | "daily" => reference_date,
        "1dte" | "tomorrow" => reference_date + Duration::days(1),
        "weekly" | "weeklies" => next_weekday_on_or_after(reference_date, Weekday::Friday),
        "next week" => {
            next_weekday_on_or_after(reference_date + Duration::days(7), Weekday::Friday)
        }
        _ => {
            return Err(format!(
                "暂不支持 expiry_label={normalized_label} 的自动日期映射"
            ))
        }
    };

    Ok(format!(
        "{:04}-{:02}-{:02}",
        expiry_date.year(),
        u8::from(expiry_date.month()),
        expiry_date.day()
    ))
}

fn next_weekday_on_or_after(date: time::Date, weekday: Weekday) -> time::Date {
    let current_index = i64::from(date.weekday().number_days_from_monday());
    let target_index = i64::from(weekday.number_days_from_monday());
    let delta = (target_index - current_index).rem_euclid(7);

    date + Duration::days(delta)
}

pub fn persist_runtime_snapshot(snapshot: &RuntimeSnapshot) -> Result<(), String> {
    let client_config = PersistedClientConfig {
        broker_config: snapshot.broker_config.clone(),
        nats_config: snapshot.nats_config.clone(),
    };
    let runtime_state = PersistedRuntimeState {
        messages: snapshot.messages.clone(),
    };

    persist_client_config(&client_config)?;
    persist_runtime_state(&runtime_state)?;
    Ok(())
}

fn load_client_persistence(
) -> Result<Option<(PersistedClientConfig, PersistedRuntimeState)>, String> {
    let storage_paths = resolved_storage_paths();
    let client_config = load_json_file::<PersistedClientConfig>(
        &storage_paths.client_config_file,
        "客户端配置文件",
    )?;
    let runtime_state = load_json_file::<PersistedRuntimeState>(
        &storage_paths.runtime_state_file,
        "客户端运行状态文件",
    )?;

    if client_config.is_some() || runtime_state.is_some() {
        return Ok(Some((
            client_config.unwrap_or_default(),
            runtime_state.unwrap_or_default(),
        )));
    }

    let Some(legacy_state) = load_legacy_persisted_state()? else {
        return Ok(None);
    };

    let client_config = PersistedClientConfig {
        broker_config: legacy_state.broker_config,
        nats_config: legacy_state.nats_config,
    };
    let runtime_state = PersistedRuntimeState {
        messages: legacy_state.messages,
    };

    persist_client_config(&client_config)?;
    persist_runtime_state(&runtime_state)?;

    Ok(Some((client_config, runtime_state)))
}

fn persist_client_config(config: &PersistedClientConfig) -> Result<(), String> {
    let storage_paths = resolved_storage_paths();
    let payload = serde_json::to_string_pretty(config).map_err(|error| error.to_string())?;
    write_json_file(
        &storage_paths.client_config_file,
        &payload,
        "客户端配置文件",
    )
}

fn persist_runtime_state(state: &PersistedRuntimeState) -> Result<(), String> {
    let storage_paths = resolved_storage_paths();
    let payload = serde_json::to_string_pretty(state).map_err(|error| error.to_string())?;
    write_json_file(
        &storage_paths.runtime_state_file,
        &payload,
        "客户端运行状态文件",
    )
}

fn write_json_file(path: &PathBuf, payload: &str, label: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建{label}目录 {}: {error}", parent.display()))?;
    }

    fs::write(path, payload)
        .map_err(|error| format!("无法写入{label} {}: {error}", path.display()))
}

fn load_json_file<T>(path: &PathBuf, label: &str) -> Result<Option<T>, String>
where
    T: for<'de> Deserialize<'de>,
{
    if !path.exists() {
        return Ok(None);
    }

    let payload = fs::read_to_string(path)
        .map_err(|error| format!("无法读取{label} {}: {error}", path.display()))?;

    let parsed = serde_json::from_str::<T>(&payload)
        .map_err(|error| format!("无法解析{label} {}: {error}", path.display()))?;

    Ok(Some(parsed))
}

fn load_legacy_persisted_state() -> Result<Option<LegacyPersistedState>, String> {
    let path = legacy_state_file_path();

    if !path.exists() {
        return Ok(None);
    }

    let payload = fs::read_to_string(&path)
        .map_err(|error| format!("无法读取旧版状态文件 {}: {error}", path.display()))?;

    let state = serde_json::from_str::<LegacyPersistedState>(&payload)
        .map_err(|error| format!("无法解析旧版状态文件 {}: {error}", path.display()))?;

    Ok(Some(state))
}

fn resolved_storage_paths() -> ResolvedStoragePaths {
    if let Some(override_home) = env_string(STORAGE_HOME_ENV) {
        let base_dir = PathBuf::from(override_home);
        return ResolvedStoragePaths {
            client_config_file: base_dir.join(APP_CONFIG_FILE_NAME),
            runtime_state_file: base_dir.join(RUNTIME_STATE_FILE_NAME),
        };
    }

    if let Some(project_dirs) = ProjectDirs::from("com", "dcbot", "options-relay") {
        return ResolvedStoragePaths {
            client_config_file: project_dirs.config_dir().join(APP_CONFIG_FILE_NAME),
            runtime_state_file: project_dirs.data_dir().join(RUNTIME_STATE_FILE_NAME),
        };
    }

    let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    ResolvedStoragePaths {
        client_config_file: base_dir.join(APP_CONFIG_FILE_NAME),
        runtime_state_file: base_dir.join(RUNTIME_STATE_FILE_NAME),
    }
}

fn legacy_state_file_path() -> PathBuf {
    if cfg!(debug_assertions) {
        if let Ok(current_dir) = std::env::current_dir() {
            return current_dir.join(LEGACY_STATE_FILE_NAME);
        }
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            return parent.join(LEGACY_STATE_FILE_NAME);
        }
    }

    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(LEGACY_STATE_FILE_NAME)
}