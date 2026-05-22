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

type SignalFormState = {
  source: string;
  strategyTag: string;
  symbol: string;
  optionType: OptionType;
  expiry: string;
  strike: string;
  side: OrderSide;
  quantity: string;
  limitPrice: string;
  rawMessage: string;
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

const emptySnapshot: RuntimeSnapshot = {
  brokerConfig: defaultBrokerConfig,
  messages: [],
  stats: {
    total: 0,
    queued: 0,
    forwarding: 0,
    sent: 0,
    failed: 0,
  },
};

const defaultSignalForm: SignalFormState = {
  source: "Discord options desk",
  strategyTag: "opening-sweep",
  symbol: "AAPL",
  optionType: "call",
  expiry: "2026-06-19",
  strike: "200",
  side: "buy",
  quantity: "1",
  limitPrice: "1.25",
  rawMessage: "AAPL 2026-06-19 200C BUY 1 @ 1.25",
};

function App() {
  const [snapshot, setSnapshot] = useState<RuntimeSnapshot>(emptySnapshot);
  const [signalForm, setSignalForm] = useState<SignalFormState>(defaultSignalForm);
  const [configForm, setConfigForm] = useState<ConfigFormState>(configToForm(defaultBrokerConfig));
  const [configDirty, setConfigDirty] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [savingConfig, setSavingConfig] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const configDirtyRef = useRef(false);

  const deferredMessages = useDeferredValue(snapshot.messages);
  const activeBacklog = snapshot.stats.queued + snapshot.stats.forwarding;

  useEffect(() => {
    configDirtyRef.current = configDirty;
  }, [configDirty]);

  useEffect(() => {
    let dispose: (() => void) | undefined;

    void invoke<RuntimeSnapshot>("bootstrap_state")
      .then((payload) => {
        startTransition(() => {
          setSnapshot(payload);

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

  async function handleSignalSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSubmitting(true);
    setNotice(null);

    try {
      const payload = signalToPayload(signalForm);
      await invoke<RelayMessage>("submit_option_signal", { signal: payload });

      setNotice({
        tone: "success",
        text: snapshot.brokerConfig.autoForward
          ? "信号已入队，后台 relay 会继续推送状态。"
          : "信号已保存，但当前 Auto Relay 处于关闭状态。",
      });

      setSignalForm((current) => ({
        ...current,
        rawMessage: "",
      }));
    } catch (error) {
      setNotice({ tone: "error", text: formatError(error) });
    } finally {
      setSubmitting(false);
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

  function updateSignalField<K extends keyof SignalFormState>(
    field: K,
    value: SignalFormState[K],
  ) {
    setSignalForm((current) => ({
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
            当前骨架已经具备本地队列、异步转发、状态回推和 IB Gateway 配置面板。
            默认以 dry-run 模式启动，避免误下实盘单。
          </p>
          <div className="hero-tags">
            <span className="hero-tag">Tauri 2</span>
            <span className="hero-tag">Vite + React</span>
            <span className="hero-tag">pnpm</span>
            <span className="hero-tag">ibapi 2.12.0</span>
          </div>
        </div>

        <div className="hero-route">
          <div className="route-badge-grid">
            <div className="route-badge">
              <span>Mode</span>
              <strong>{snapshot.brokerConfig.dryRun ? "Dry Run" : "Live"}</strong>
            </div>
            <div className="route-badge">
              <span>Relay</span>
              <strong>{snapshot.brokerConfig.autoForward ? "Auto" : "Manual"}</strong>
            </div>
            <div className="route-badge">
              <span>Queue</span>
              <strong>{activeBacklog}</strong>
            </div>
            <div className="route-badge">
              <span>Target</span>
              <strong>{snapshot.brokerConfig.host}:{snapshot.brokerConfig.port}</strong>
            </div>
          </div>

          <ol className="route-list">
            <li>Discord / 其他源推送原始下单消息</li>
            <li>前端立即展示标准化后的期权信号</li>
            <li>Rust 后台将信号压入 relay 队列</li>
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
        <form className="panel composer-panel" onSubmit={handleSignalSubmit}>
          <div className="section-heading">
            <div>
              <p className="eyebrow">Signal Intake</p>
              <h2>期权信号录入</h2>
            </div>
            <span className="section-chip">本地入队后即刻可见</span>
          </div>

          <div className="form-grid two-columns">
            <label>
              <span>Source</span>
              <input
                value={signalForm.source}
                onChange={(event) => updateSignalField("source", event.currentTarget.value)}
                placeholder="Discord options desk"
              />
            </label>
            <label>
              <span>Strategy Tag</span>
              <input
                value={signalForm.strategyTag}
                onChange={(event) => updateSignalField("strategyTag", event.currentTarget.value)}
                placeholder="opening-sweep"
              />
            </label>
            <label>
              <span>Symbol</span>
              <input
                value={signalForm.symbol}
                onChange={(event) => updateSignalField("symbol", event.currentTarget.value.toUpperCase())}
                placeholder="AAPL"
              />
            </label>
            <label>
              <span>Expiry</span>
              <input
                value={signalForm.expiry}
                onChange={(event) => updateSignalField("expiry", event.currentTarget.value)}
                placeholder="2026-06-19"
              />
            </label>
            <label>
              <span>Option Type</span>
              <select
                value={signalForm.optionType}
                onChange={(event) => updateSignalField("optionType", event.currentTarget.value as OptionType)}
              >
                <option value="call">Call</option>
                <option value="put">Put</option>
              </select>
            </label>
            <label>
              <span>Side</span>
              <select
                value={signalForm.side}
                onChange={(event) => updateSignalField("side", event.currentTarget.value as OrderSide)}
              >
                <option value="buy">Buy</option>
                <option value="sell">Sell</option>
              </select>
            </label>
            <label>
              <span>Strike</span>
              <input
                value={signalForm.strike}
                onChange={(event) => updateSignalField("strike", event.currentTarget.value)}
                placeholder="200"
              />
            </label>
            <label>
              <span>Quantity</span>
              <input
                value={signalForm.quantity}
                onChange={(event) => updateSignalField("quantity", event.currentTarget.value)}
                placeholder="1"
              />
            </label>
            <label className="span-2">
              <span>Limit Price</span>
              <input
                value={signalForm.limitPrice}
                onChange={(event) => updateSignalField("limitPrice", event.currentTarget.value)}
                placeholder="留空则使用市价单"
              />
            </label>
            <label className="span-2">
              <span>Raw Message</span>
              <textarea
                rows={5}
                value={signalForm.rawMessage}
                onChange={(event) => updateSignalField("rawMessage", event.currentTarget.value)}
                placeholder="从 Discord、Telegram 或 webhook 拿到的原始下单消息"
              />
            </label>
          </div>

          <div className="panel-footer">
            <div className="footer-copy">
              <strong>{snapshot.brokerConfig.autoForward ? "Auto Relay 已启用" : "当前仅本地入队"}</strong>
              <p>
                {snapshot.brokerConfig.dryRun
                  ? "Dry-run 打开时只验证合约和订单构造，不会真的发给 IBKR。"
                  : "Dry-run 关闭后，会尝试连接 IB Gateway / TWS 并立刻 place order。"}
              </p>
            </div>
            <button type="submit" disabled={submitting}>
              {submitting ? "Submitting..." : "Queue Signal"}
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
                <input
                  value={configForm.port}
                  onChange={(event) => updateConfigField("port", event.currentTarget.value)}
                  placeholder="4002"
                />
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
                <input
                  value={configForm.defaultExchange}
                  onChange={(event) => updateConfigField("defaultExchange", event.currentTarget.value.toUpperCase())}
                  placeholder="SMART"
                />
              </label>
              <label>
                <span>Currency</span>
                <input
                  value={configForm.currency}
                  onChange={(event) => updateConfigField("currency", event.currentTarget.value.toUpperCase())}
                  placeholder="USD"
                />
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

          <section className="panel pipeline-panel">
            <div className="section-heading">
              <div>
                <p className="eyebrow">Runtime Shape</p>
                <h2>当前骨架边界</h2>
              </div>
            </div>

            <ul className="pipeline-list">
              <li>
                <strong>前端展示先行</strong>
                <span>所有信号先进入本地状态，让操作台立即看到。</span>
              </li>
              <li>
                <strong>Rust relay 独立</strong>
                <span>IB 逻辑已被隔离，后面替换成别的 broker 也容易扩展。</span>
              </li>
              <li>
                <strong>默认安全模式</strong>
                <span>Dry-run 默认开启，防止骨架阶段误发实盘订单。</span>
              </li>
              <li>
                <strong>后续入口清晰</strong>
                <span>下一步可以把 Discord 解析结果直接喂给 submit_option_signal。</span>
              </li>
            </ul>
          </section>
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
            <strong>还没有信号进入控制台。</strong>
            <p>先在上面的表单里提交一条期权指令，或者把 Discord 解析器接到这个入口。</p>
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

function signalToPayload(form: SignalFormState): OptionSignalInput {
  return {
    source: form.source.trim(),
    strategyTag: form.strategyTag.trim(),
    symbol: form.symbol.trim().toUpperCase(),
    optionType: form.optionType,
    expiry: form.expiry.trim(),
    strike: parseDecimal(form.strike, "Strike"),
    side: form.side,
    quantity: parseDecimal(form.quantity, "Quantity"),
    limitPrice: form.limitPrice.trim() ? parseDecimal(form.limitPrice, "Limit Price") : null,
    rawMessage: form.rawMessage.trim(),
  };
}

function parseDecimal(value: string, label: string): number {
  const parsed = Number(value);

  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`${label} 必须是大于 0 的数字`);
  }

  return parsed;
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
