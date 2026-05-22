import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  startTransition,
  useDeferredValue,
  useEffect,
  useRef,
  useState,
  type FormEvent,
} from "react";
import relayLogo from "./assets/relay-logo.svg";
import "./App.css";

type OptionType = "call" | "put";
type OrderSide = "buy" | "sell";
type RelayStatus = "queued" | "forwarding" | "sent" | "failed";
type NoticeTone = "success" | "error";

type NatsFormState = {
  serverAddress: string;
  subject: string;
  queueGroup: string;
  autoSubscribe: boolean;
};

type ConfigFormState = {
  host: string;
  port: string;
  clientId: string;
  account: string;
  defaultExchange: string;
  currency: string;
  dryRun: boolean;
  autoForward: boolean;
};

type OptionSignalInput = {
  source: string;
  strategyTag: string;
  symbol: string;
  optionType: OptionType;
  expiry: string;
  strike: number;
  side: OrderSide;
  quantity: number;
  limitPrice: number | null;
  rawMessage: string;
};

type NatsFeedConfig = {
  serverAddress: string;
  subject: string;
  queueGroup: string;
  autoSubscribe: boolean;
};

type IbGatewayConfig = {
  host: string;
  port: number;
  clientId: number;
  account: string;
  defaultExchange: string;
  currency: string;
  dryRun: boolean;
  autoForward: boolean;
};

type RelayReceipt = {
  broker: string;
  orderId: string | null;
  message: string;
  simulated: boolean;
};

type RelayMessage = {
  id: number;
  receivedAt: string;
  signal: OptionSignalInput;
  status: RelayStatus;
  relayNotes: string;
  receipt: RelayReceipt | null;
};

type RelayStats = {
  total: number;
  queued: number;
  forwarding: number;
  sent: number;
  failed: number;
};

type RuntimeSnapshot = {
  brokerConfig: IbGatewayConfig;
  natsConfig: NatsFeedConfig;
  messages: RelayMessage[];
  stats: RelayStats;
};

type Notice = {
  tone: NoticeTone;
  text: string;
};

const defaultBrokerConfig: IbGatewayConfig = {
  host: "127.0.0.1",
  port: 4002,
  clientId: 100,
  account: "",
  defaultExchange: "SMART",
  currency: "USD",
  dryRun: true,
  autoForward: true,
};

const defaultNatsConfig: NatsFeedConfig = {
  serverAddress: "127.0.0.1:4222",
  subject: "signals.options.entry",
  queueGroup: "",
  autoSubscribe: false,
};

const ibPortOptions = [
  { value: "4001", label: "4001 / IB Gateway Live" },
  { value: "4002", label: "4002 / IB Gateway Paper" },
  { value: "7496", label: "7496 / TWS Live" },
  { value: "7497", label: "7497 / TWS Paper" },
];

const ibExchangeOptions = ["SMART", "CBOE", "ISE", "BOX", "GEMINI"];
const ibCurrencyOptions = ["USD", "EUR", "HKD", "CAD"];

const natsPayloadExample = `{
  "author_username": "Enrich Trades",
  "timestamp": "2026-05-22T13:51:59.619Z",
  "category": "entry",
  "parsed_entry": {
    "symbol": "ARM",
    "strike": "305",
    "contract_type": "calls",
    "expiry_label": "0dte",
    "price": "2.70"
  },
  "content": "$ARM - $305 0DTE lotto size $2.70"
}`;

const emptySnapshot: RuntimeSnapshot = {
  brokerConfig: defaultBrokerConfig,
  natsConfig: defaultNatsConfig,
  messages: [],
  stats: {
    total: 0,
    queued: 0,
    forwarding: 0,
    sent: 0,
    failed: 0,
  },
};

function App() {
  const [snapshot, setSnapshot] = useState<RuntimeSnapshot>(emptySnapshot);
  const [natsForm, setNatsForm] = useState<NatsFormState>(natsToForm(defaultNatsConfig));
  const [natsDirty, setNatsDirty] = useState(false);
  const [configForm, setConfigForm] = useState<ConfigFormState>(configToForm(defaultBrokerConfig));
  const [configDirty, setConfigDirty] = useState(false);
  const [savingNatsConfig, setSavingNatsConfig] = useState(false);
  const [savingConfig, setSavingConfig] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const natsDirtyRef = useRef(false);
  const configDirtyRef = useRef(false);

  const deferredMessages = useDeferredValue(snapshot.messages);
  const activeBacklog = snapshot.stats.queued + snapshot.stats.forwarding;

  useEffect(() => {
    natsDirtyRef.current = natsDirty;
  }, [natsDirty]);

  useEffect(() => {
    configDirtyRef.current = configDirty;
  }, [configDirty]);

  useEffect(() => {
    let dispose: (() => void) | undefined;

    void invoke<RuntimeSnapshot>("bootstrap_state")
      .then((payload) => {
        startTransition(() => {
          setSnapshot(payload);

          if (!natsDirtyRef.current) {
            setNatsForm(natsToForm(payload.natsConfig));
          }

          if (!configDirtyRef.current) {
            setConfigForm(configToForm(payload.brokerConfig));
          }
        });
      })
      .catch((error) => {
        setNotice({ tone: "error", text: formatError(error) });
      });

    void listen<RuntimeSnapshot>("relay:snapshot", (event) => {
      startTransition(() => {
        setSnapshot(event.payload);

        if (!natsDirtyRef.current) {
          setNatsForm(natsToForm(event.payload.natsConfig));
        }

        if (!configDirtyRef.current) {
          setConfigForm(configToForm(event.payload.brokerConfig));
        }
      });
    })
      .then((unlisten) => {
        dispose = unlisten;
      })
      .catch((error) => {
        setNotice({ tone: "error", text: formatError(error) });
      });

    return () => {
      dispose?.();
    };
  }, []);

  async function handleNatsConfigSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSavingNatsConfig(true);
    setNotice(null);

    try {
      const payload = natsToPayload(natsForm);
      const nextSnapshot = await invoke<RuntimeSnapshot>("save_nats_feed_config", {
        config: payload,
      });

      startTransition(() => {
        setSnapshot(nextSnapshot);
        setNatsForm(natsToForm(nextSnapshot.natsConfig));
      });

      natsDirtyRef.current = false;
      setNatsDirty(false);
      setNotice({
        tone: "success",
        text: payload.autoSubscribe
          ? "NATS Feed 配置已保存，Rust 侧会按自动订阅策略准备接入。"
          : "NATS Feed 配置已保存，当前仍处于手动订阅策略。",
      });
    } catch (error) {
      setNotice({ tone: "error", text: formatError(error) });
    } finally {
      setSavingNatsConfig(false);
    }
  }

  async function handleConfigSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSavingConfig(true);
    setNotice(null);

    try {
      const payload = configToPayload(configForm);
      const nextSnapshot = await invoke<RuntimeSnapshot>("save_ib_gateway_config", {
        config: payload,
      });

      startTransition(() => {
        setSnapshot(nextSnapshot);
        setConfigForm(configToForm(nextSnapshot.brokerConfig));
      });

      configDirtyRef.current = false;
      setConfigDirty(false);
      setNotice({
        tone: "success",
        text: payload.dryRun
          ? "IBKR 配置已保存，当前仍处于 dry-run。"
          : "IBKR 配置已保存，系统已切到真实转发模式。",
      });
    } catch (error) {
      setNotice({ tone: "error", text: formatError(error) });
    } finally {
      setSavingConfig(false);
    }
  }

  function updateNatsField<K extends keyof NatsFormState>(field: K, value: NatsFormState[K]) {
    natsDirtyRef.current = true;
    setNatsDirty(true);
    setNatsForm((current) => ({
      ...current,
      [field]: value,
    }));
  }

  function updateConfigField<K extends keyof ConfigFormState>(
    field: K,
    value: ConfigFormState[K],
  ) {
    configDirtyRef.current = true;
    setConfigDirty(true);
    setConfigForm((current) => ({
      ...current,
      [field]: value,
    }));
  }

  return (
    <main className="shell">
      <section className="hero panel">
        <div className="hero-copy">
          <div className="brand-row">
            <img className="brand-logo" src={relayLogo} alt="Options Relay logo" />
            <span>Options Relay</span>
          </div>
          <p className="eyebrow">Phase 1 / IBKR Relay Console</p>
          <h1>接收期权信号，先展示，再尽快转发。</h1>
          <p className="hero-text">
            页面现在聚焦 Rust 原生 NATS feed 订阅入口和 IBKR 路由配置。
            收到的 entry 信号仍然会先进入本地状态，再交给 relay 继续转发。
          </p>
          <div className="hero-tags">
            <span className="hero-tag">NATS Native</span>
            <span className="hero-tag">Tauri 2</span>
            <span className="hero-tag">Vite + React</span>
            <span className="hero-tag">pnpm</span>
            <span className="hero-tag">ibapi 2.12.0</span>
          </div>
        </div>

        <div className="hero-route">
          <div className="route-badge-grid">
            <div className="route-badge">
              <span>Feed</span>
              <strong>{compactValue(snapshot.natsConfig.serverAddress, "未配置")}</strong>
            </div>
            <div className="route-badge">
              <span>Subject</span>
              <strong>{compactValue(snapshot.natsConfig.subject, "未配置")}</strong>
            </div>
            <div className="route-badge">
              <span>Mode</span>
              <strong>{snapshot.brokerConfig.dryRun ? "Dry Run" : "Live"}</strong>
            </div>
            <div className="route-badge">
              <span>Target</span>
              <strong>{snapshot.brokerConfig.host}:{snapshot.brokerConfig.port}</strong>
            </div>
          </div>

          <ol className="route-list">
            <li>Rust 直接订阅指定 NATS server 和 subject</li>
            <li>页面保存 feed 配置并等待接入消息流</li>
            <li>Rust relay 将标准化信号压入本地队列</li>
            <li>IBKR 适配层异步下发并把结果回写 UI</li>
          </ol>
        </div>
      </section>

      <section className="stats-grid">
        <article className="stat-card accent-ocean">
          <span className="stat-label">Captured</span>
          <strong>{snapshot.stats.total}</strong>
          <p>已进入控制台的总信号数</p>
        </article>
        <article className="stat-card accent-sand">
          <span className="stat-label">In Flight</span>
          <strong>{activeBacklog}</strong>
          <p>排队或转发中的信号</p>
        </article>
        <article className="stat-card accent-mint">
          <span className="stat-label">Sent</span>
          <strong>{snapshot.stats.sent}</strong>
          <p>已通过当前 relay 路径提交</p>
        </article>
        <article className="stat-card accent-coral">
          <span className="stat-label">Failed</span>
          <strong>{snapshot.stats.failed}</strong>
          <p>连接、校验或投递失败的请求</p>
        </article>
      </section>

      {notice ? <section className={`notice notice-${notice.tone}`}>{notice.text}</section> : null}

      <section className="workspace-grid">
        <form className="panel composer-panel" onSubmit={handleNatsConfigSave}>
          <div className="section-heading">
            <div>
              <p className="eyebrow">Feed Intake</p>
              <h2>NATS 原生订阅入口</h2>
            </div>
            <span className="section-chip">保存后会持久化到本地 JSON</span>
          </div>

          <div className="form-grid two-columns">
            <label className="span-2">
              <span>NATS Server / IP:Port</span>
              <input
                value={natsForm.serverAddress}
                onChange={(event) => updateNatsField("serverAddress", event.currentTarget.value)}
                placeholder="127.0.0.1:4222 或 nats://127.0.0.1:4222"
              />
            </label>
            <label>
              <span>Subject</span>
              <input
                value={natsForm.subject}
                onChange={(event) => updateNatsField("subject", event.currentTarget.value)}
                placeholder="signals.options.entry"
              />
            </label>
            <label>
              <span>Queue Group</span>
              <input
                value={natsForm.queueGroup}
                onChange={(event) => updateNatsField("queueGroup", event.currentTarget.value)}
                placeholder="可选，用于多实例消费"
              />
            </label>
            <div className="info-card span-2">
              <strong>推荐入站消息格式</strong>
              <p>
                当前页面已经从手动录单切到 feed 配置入口，推荐直接沿用 Discord 解析后的 entry JSON 结构。
              </p>
              <pre className="raw-message compact">{natsPayloadExample}</pre>
            </div>
          </div>

          <div className="toggle-grid">
            <label className="toggle-card">
              <input
                type="checkbox"
                checked={natsForm.autoSubscribe}
                onChange={(event) => updateNatsField("autoSubscribe", event.currentTarget.checked)}
              />
              <div>
                <strong>Auto Subscribe</strong>
                <p>启动后优先按当前 NATS server 和 subject 准备订阅 feed。</p>
              </div>
            </label>
          </div>

          <div className="panel-footer">
            <div className="footer-copy">
              <strong>
                {natsDirty ? "有未保存的 Feed 修改" : "Feed 配置已同步到本地状态 JSON"}
              </strong>
              <p>
                保存的是 NATS 接入口配置；消息实际到来后，仍然会先写入本地状态，再交给 IB relay。
              </p>
            </div>
            <button type="submit" disabled={savingNatsConfig}>
              {savingNatsConfig ? "Saving..." : "Save Feed Config"}
            </button>
          </div>
        </form>

        <div className="side-stack">
          <form className="panel gateway-panel" onSubmit={handleConfigSave}>
            <div className="section-heading">
              <div>
                <p className="eyebrow">IBKR Target</p>
                <h2>Gateway 配置</h2>
              </div>
              <span className="section-chip">实时影响 relay 行为</span>
            </div>

            <div className="form-grid">
              <label>
                <span>Host</span>
                <input
                  value={configForm.host}
                  onChange={(event) => updateConfigField("host", event.currentTarget.value)}
                  placeholder="127.0.0.1"
                />
              </label>
              <label>
                <span>Port</span>
                <select
                  value={configForm.port}
                  onChange={(event) => updateConfigField("port", event.currentTarget.value)}
                >
                  {ibPortOptions.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                <span>Client ID</span>
                <input
                  value={configForm.clientId}
                  onChange={(event) => updateConfigField("clientId", event.currentTarget.value)}
                  placeholder="100"
                />
              </label>
              <label>
                <span>Account</span>
                <input
                  value={configForm.account}
                  onChange={(event) => updateConfigField("account", event.currentTarget.value)}
                  placeholder="可选，实盘或模拟账户"
                />
              </label>
              <label>
                <span>Exchange</span>
                <select
                  value={configForm.defaultExchange}
                  onChange={(event) => updateConfigField("defaultExchange", event.currentTarget.value.toUpperCase())}
                >
                  {ibExchangeOptions.map((option) => (
                    <option key={option} value={option}>
                      {option}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                <span>Currency</span>
                <select
                  value={configForm.currency}
                  onChange={(event) => updateConfigField("currency", event.currentTarget.value.toUpperCase())}
                >
                  {ibCurrencyOptions.map((option) => (
                    <option key={option} value={option}>
                      {option}
                    </option>
                  ))}
                </select>
              </label>
            </div>

            <div className="toggle-grid">
              <label className="toggle-card">
                <input
                  type="checkbox"
                  checked={configForm.dryRun}
                  onChange={(event) => updateConfigField("dryRun", event.currentTarget.checked)}
                />
                <div>
                  <strong>Dry Run</strong>
                  <p>仅演练 relay 路径，不真正提交 IB 订单。</p>
                </div>
              </label>
              <label className="toggle-card">
                <input
                  type="checkbox"
                  checked={configForm.autoForward}
                  onChange={(event) => updateConfigField("autoForward", event.currentTarget.checked)}
                />
                <div>
                  <strong>Auto Relay</strong>
                  <p>消息一入队就立刻触发后台 relay 任务。</p>
                </div>
              </label>
            </div>

            <div className="panel-footer compact">
              <p>{configDirty ? "有未保存修改" : "配置已同步"}</p>
              <button type="submit" disabled={savingConfig}>
                {savingConfig ? "Saving..." : "Save IB Config"}
              </button>
            </div>
          </form>
        </div>
      </section>

      <section className="panel tape-panel">
        <div className="section-heading">
          <div>
            <p className="eyebrow">Relay Tape</p>
            <h2>消息流与投递结果</h2>
          </div>
          <span className="section-chip">按最新消息倒序展示</span>
        </div>

        {deferredMessages.length === 0 ? (
          <div className="empty-state">
            <strong>还没有消息通过 NATS feed 进入控制台。</strong>
            <p>先保存上面的 NATS server 和 subject，后续把 entry 消息推到对应 subject 即可。</p>
          </div>
        ) : (
          <div className="ticket-list">
            {deferredMessages.map((message) => (
              <article className="ticket" key={message.id}>
                <div className="ticket-header">
                  <div>
                    <p className="ticket-kicker">
                      #{message.id} / {message.signal.source}
                    </p>
                    <h3>
                      {message.signal.symbol} {message.signal.expiry} {message.signal.strike}
                      {message.signal.optionType === "call" ? "C" : "P"}
                    </h3>
                  </div>
                  <span className={`status-pill status-${message.status}`}>
                    {statusLabel(message.status)}
                  </span>
                </div>

                <div className="ticket-meta-row">
                  <span>{message.signal.side.toUpperCase()} x {message.signal.quantity}</span>
                  <span>
                    {message.signal.limitPrice === null
                      ? "Market"
                      : `$${message.signal.limitPrice.toFixed(2)}`}
                  </span>
                  <span>{formatTimestamp(message.receivedAt)}</span>
                  <span>{message.signal.strategyTag}</span>
                </div>

                {message.signal.rawMessage ? (
                  <pre className="raw-message">{message.signal.rawMessage}</pre>
                ) : null}

                <p className="ticket-note">{message.relayNotes}</p>

                {message.receipt ? (
                  <div className="receipt-row">
                    <span>{message.receipt.broker.toUpperCase()}</span>
                    <span>{message.receipt.orderId ? `order #${message.receipt.orderId}` : "pending order id"}</span>
                    <span>{message.receipt.simulated ? "dry-run" : "live"}</span>
                  </div>
                ) : null}
              </article>
            ))}
          </div>
        )}
      </section>
    </main>
  );
}

function natsToForm(config: NatsFeedConfig): NatsFormState {
  return {
    serverAddress: config.serverAddress,
    subject: config.subject,
    queueGroup: config.queueGroup,
    autoSubscribe: config.autoSubscribe,
  };
}

function natsToPayload(form: NatsFormState): NatsFeedConfig {
  return {
    serverAddress: form.serverAddress.trim(),
    subject: form.subject.trim(),
    queueGroup: form.queueGroup.trim(),
    autoSubscribe: form.autoSubscribe,
  };
}

function configToForm(config: IbGatewayConfig): ConfigFormState {
  return {
    host: config.host,
    port: String(config.port),
    clientId: String(config.clientId),
    account: config.account,
    defaultExchange: config.defaultExchange,
    currency: config.currency,
    dryRun: config.dryRun,
    autoForward: config.autoForward,
  };
}

function configToPayload(form: ConfigFormState): IbGatewayConfig {
  return {
    host: form.host.trim(),
    port: parseInteger(form.port, "Port"),
    clientId: parseInteger(form.clientId, "Client ID"),
    account: form.account.trim(),
    defaultExchange: form.defaultExchange.trim().toUpperCase(),
    currency: form.currency.trim().toUpperCase(),
    dryRun: form.dryRun,
    autoForward: form.autoForward,
  };
}

function parseInteger(value: string, label: string): number {
  const parsed = Number.parseInt(value, 10);

  if (!Number.isInteger(parsed) || parsed < 0) {
    throw new Error(`${label} 必须是有效整数`);
  }

  return parsed;
}

function formatError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }

  if (typeof error === "string") {
    return error;
  }

  return "发生未知错误";
}

function formatTimestamp(value: string): string {
  const parsed = new Date(value);

  if (Number.isNaN(parsed.getTime())) {
    return value;
  }

  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(parsed);
}

function compactValue(value: string, fallback: string): string {
  const trimmed = value.trim();

  if (!trimmed) {
    return fallback;
  }

  return trimmed.length > 28 ? `${trimmed.slice(0, 25)}...` : trimmed;
}

function statusLabel(status: RelayStatus): string {
  switch (status) {
    case "queued":
      return "Queued";
    case "forwarding":
      return "Forwarding";
    case "sent":
      return "Sent";
    case "failed":
      return "Failed";
    default:
      return status;
  }
}

export default App;
