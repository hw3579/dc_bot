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

type BrokerKind = "ibkr" | "moomoo";
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
  broker: BrokerKind;
  defaultQuantity: string;
  host: string;
  port: string;
  clientId: string;
  account: string;
  defaultExchange: string;
  currency: string;
  moomooHost: string;
  moomooPort: string;
  moomooMarket: string;
  moomooTrdEnv: string;
  moomooAccId: string;
  moomooSecurityFirm: string;
  moomooTimeInForce: string;
  moomooSession: string;
  moomooFillOutsideRth: boolean;
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

type BrokerConfig = {
  broker: BrokerKind;
  defaultQuantity: number;
  host: string;
  port: number;
  clientId: number;
  account: string;
  defaultExchange: string;
  currency: string;
  moomooHost: string;
  moomooPort: number;
  moomooMarket: string;
  moomooTrdEnv: string;
  moomooAccId: number;
  moomooSecurityFirm: string;
  moomooTimeInForce: string;
  moomooSession: string;
  moomooFillOutsideRth: boolean;
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

type ClientStoragePaths = {
  clientConfigFile: string;
  runtimeStateFile: string;
};

type RuntimeSnapshot = {
  brokerConfig: BrokerConfig;
  natsConfig: NatsFeedConfig;
  storagePaths: ClientStoragePaths;
  messages: RelayMessage[];
  stats: RelayStats;
};

type Notice = {
  tone: NoticeTone;
  text: string;
};

const defaultBrokerConfig: BrokerConfig = {
  broker: "ibkr",
  defaultQuantity: 1,
  host: "127.0.0.1",
  port: 4002,
  clientId: 100,
  account: "",
  defaultExchange: "SMART",
  currency: "USD",
  moomooHost: "127.0.0.1",
  moomooPort: 11111,
  moomooMarket: "US",
  moomooTrdEnv: "SIMULATE",
  moomooAccId: 0,
  moomooSecurityFirm: "FUTUSECURITIES",
  moomooTimeInForce: "DAY",
  moomooSession: "NONE",
  moomooFillOutsideRth: false,
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

const brokerOptions = [
  { value: "ibkr", label: "IBKR / ibapi" },
  { value: "moomoo", label: "Moomoo / PyO3" },
];

const ibExchangeOptions = ["SMART", "CBOE", "ISE", "BOX", "GEMINI"];
const ibCurrencyOptions = ["USD", "EUR", "HKD", "CAD"];
const moomooMarketOptions = ["US", "HK", "CN", "SG", "JP", "AU", "CA"];
const moomooTrdEnvOptions = ["SIMULATE", "REAL"];
const moomooTimeInForceOptions = ["DAY", "GTC"];
const moomooSessionOptions = ["NONE", "RTH", "ETH", "OVERNIGHT"];

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
  storagePaths: {
    clientConfigFile: "",
    runtimeStateFile: "",
  },
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
  const [savingQuickConfig, setSavingQuickConfig] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const natsDirtyRef = useRef(false);
  const configDirtyRef = useRef(false);

  const deferredMessages = useDeferredValue(snapshot.messages);
  const activeBacklog = snapshot.stats.queued + snapshot.stats.forwarding;
  const feedReady = isFilled(natsForm.serverAddress) && isFilled(natsForm.subject);
  const brokerReady = isBrokerFormReady(configForm);
  const quantityReady = isPositiveNumberValue(configForm.defaultQuantity);
  const automationReady = natsForm.autoSubscribe && configForm.autoForward;
  const readySteps = [feedReady, brokerReady, quantityReady, automationReady].filter(Boolean).length;
  const quickSetupHint = followSetupHint(configForm, natsForm);

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
      const nextSnapshot = await invoke<RuntimeSnapshot>("save_broker_config", {
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
          ? `${brokerDisplayName(payload.broker)} 配置已保存，当前仍处于 dry-run。`
          : `${brokerDisplayName(payload.broker)} 配置已保存，系统已切到真实转发模式。`,
      });
    } catch (error) {
      setNotice({ tone: "error", text: formatError(error) });
    } finally {
      setSavingConfig(false);
    }
  }

  async function handleQuickSave() {
    setSavingQuickConfig(true);
    setNotice(null);

    try {
      const natsPayload = natsToPayload(natsForm);
      const brokerPayload = configToPayload(configForm);

      await invoke<RuntimeSnapshot>("save_nats_feed_config", {
        config: natsPayload,
      });

      const nextSnapshot = await invoke<RuntimeSnapshot>("save_broker_config", {
        config: brokerPayload,
      });

      startTransition(() => {
        setSnapshot(nextSnapshot);
        setNatsForm(natsToForm(nextSnapshot.natsConfig));
        setConfigForm(configToForm(nextSnapshot.brokerConfig));
      });

      natsDirtyRef.current = false;
      configDirtyRef.current = false;
      setNatsDirty(false);
      setConfigDirty(false);
      setNotice({
        tone: "success",
        text: brokerPayload.dryRun
          ? "客户跟单配置已保存，当前是 dry-run，可先验证消息接入和默认仓位。"
          : "客户跟单配置已保存，当前会自动真实转发，请确认券商账户和仓位无误。",
      });
    } catch (error) {
      setNotice({ tone: "error", text: formatError(error) });
    } finally {
      setSavingQuickConfig(false);
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

  function applyFollowPreset(broker: BrokerKind) {
    natsDirtyRef.current = true;
    configDirtyRef.current = true;
    setNatsDirty(true);
    setConfigDirty(true);

    setNatsForm((current) => ({
      ...current,
      serverAddress: isFilled(current.serverAddress)
        ? current.serverAddress
        : defaultNatsConfig.serverAddress,
      subject: isFilled(current.subject) ? current.subject : defaultNatsConfig.subject,
      autoSubscribe: true,
    }));

    setConfigForm((current) => {
      if (broker === "ibkr") {
        return {
          ...current,
          broker: "ibkr",
          defaultQuantity: isPositiveNumberValue(current.defaultQuantity)
            ? current.defaultQuantity
            : String(defaultBrokerConfig.defaultQuantity),
          host: isFilled(current.host) ? current.host : defaultBrokerConfig.host,
          port: isPositiveIntegerValue(current.port) ? current.port : String(defaultBrokerConfig.port),
          clientId: isNonNegativeIntegerValue(current.clientId)
            ? current.clientId
            : String(defaultBrokerConfig.clientId),
          defaultExchange: defaultBrokerConfig.defaultExchange,
          currency: defaultBrokerConfig.currency,
          dryRun: true,
          autoForward: true,
        };
      }

      return {
        ...current,
        broker: "moomoo",
        defaultQuantity: isPositiveNumberValue(current.defaultQuantity)
          ? current.defaultQuantity
          : String(defaultBrokerConfig.defaultQuantity),
        moomooHost: isFilled(current.moomooHost)
          ? current.moomooHost
          : defaultBrokerConfig.moomooHost,
        moomooPort: isPositiveIntegerValue(current.moomooPort)
          ? current.moomooPort
          : String(defaultBrokerConfig.moomooPort),
        moomooMarket: defaultBrokerConfig.moomooMarket,
        moomooTrdEnv: defaultBrokerConfig.moomooTrdEnv,
        moomooSecurityFirm: defaultBrokerConfig.moomooSecurityFirm,
        moomooTimeInForce: defaultBrokerConfig.moomooTimeInForce,
        moomooSession: defaultBrokerConfig.moomooSession,
        dryRun: true,
        autoForward: true,
      };
    });

    setNotice({
      tone: "success",
      text:
        broker === "ibkr"
          ? "已填入 IBKR 模拟跟单模板，确认后点一键保存即可。"
          : "已填入 Moomoo 模拟跟单模板，确认后点一键保存即可。",
    });
  }

  return (
    <main className="shell">
      <section className="hero panel">
        <div className="hero-copy">
          <div className="brand-row">
            <img className="brand-logo" src={relayLogo} alt="Options Relay logo" />
            <span>Options Relay</span>
          </div>
          <p className="eyebrow">Phase 2 / Multi-Broker Relay Console</p>
          <h1>接收期权信号，先展示，再尽快转发。</h1>
          <p className="hero-text">
            页面现在聚焦 Rust 原生 NATS feed 订阅入口和券商路由配置。
            收到的 entry 信号仍然会先进入本地状态，再交给 relay 继续转发。
          </p>
          <div className="hero-tags">
            <span className="hero-tag">NATS Native</span>
            <span className="hero-tag">Tauri 2</span>
            <span className="hero-tag">Vite + React</span>
            <span className="hero-tag">PyO3</span>
            <span className="hero-tag">ibapi 2.12.0</span>
            <span className="hero-tag">moomoo-api</span>
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
              <span>Broker</span>
              <strong>{brokerDisplayName(snapshot.brokerConfig.broker)}</strong>
            </div>
            <div className="route-badge">
              <span>Qty</span>
              <strong>{snapshot.brokerConfig.defaultQuantity}</strong>
            </div>
            <div className="route-badge">
              <span>Mode</span>
              <strong>{snapshot.brokerConfig.dryRun ? "Dry Run" : "Live"}</strong>
            </div>
            <div className="route-badge">
              <span>Target</span>
              <strong>{brokerTarget(snapshot.brokerConfig)}</strong>
            </div>
          </div>

          <ol className="route-list">
            <li>Rust 直接订阅指定 NATS server 和 subject</li>
            <li>页面保存 feed 配置并等待接入消息流</li>
            <li>Rust relay 将标准化信号压入本地队列</li>
            <li>当前 broker 适配层异步下发并把结果回写 UI</li>
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
                保存的是 NATS 接入口配置；消息实际到来后，仍然会先写入本地状态，再交给当前 broker relay。
              </p>
            </div>
            <button type="submit" disabled={savingNatsConfig}>
              {savingNatsConfig ? "Saving..." : "Save Feed Config"}
            </button>
          </div>
        </form>

        <div className="side-stack">
          <div className="panel quickstart-panel">
            <div className="section-heading">
              <div>
                <p className="eyebrow">Client Quick Setup</p>
                <h2>客户跟单快速配置</h2>
              </div>
              <span className="section-chip">模板 + 一键保存</span>
            </div>

            <div className={`setup-summary ${readySteps === 4 ? "setup-summary-ready" : ""}`}>
              <div className="setup-counter">
                <strong>{readySteps}/4</strong>
                <span>核心项</span>
              </div>
              <div className="setup-copy">
                <strong>{readySteps === 4 ? "可以开始跟单" : "离可用配置还差几步"}</strong>
                <p>{quickSetupHint}</p>
              </div>
            </div>

            <div className="setup-card-grid">
              <article className={`setup-card ${feedReady ? "setup-card-ready" : ""}`}>
                <span>01</span>
                <strong>Feed</strong>
                <p>
                  {feedReady
                    ? `${compactValue(natsForm.serverAddress, "")}${natsForm.subject ? ` / ${compactValue(natsForm.subject, "")}` : ""}`
                    : "填写 NATS 地址和 subject"}
                </p>
              </article>
              <article className={`setup-card ${brokerReady ? "setup-card-ready" : ""}`}>
                <span>02</span>
                <strong>Broker</strong>
                <p>{brokerReady ? formBrokerTarget(configForm) : brokerSetupHint(configForm)}</p>
              </article>
              <article className={`setup-card ${quantityReady ? "setup-card-ready" : ""}`}>
                <span>03</span>
                <strong>Quantity</strong>
                <p>{quantityReady ? `默认仓位 ${configForm.defaultQuantity}` : "设置 Default Quantity，大于 0"}</p>
              </article>
              <article className={`setup-card ${automationReady ? "setup-card-ready" : ""}`}>
                <span>04</span>
                <strong>Automation</strong>
                <p>
                  {automationReady
                    ? `${natsForm.autoSubscribe ? "Auto Subscribe" : ""} / ${configForm.autoForward ? "Auto Relay" : ""}`
                    : "开启 Auto Subscribe 和 Auto Relay"}
                </p>
              </article>
            </div>

            <div className="quick-actions">
              <button type="button" className="ghost-button" onClick={() => applyFollowPreset("ibkr")}>
                套用 IBKR 模拟模板
              </button>
              <button type="button" className="ghost-button" onClick={() => applyFollowPreset("moomoo")}>
                套用 Moomoo 模拟模板
              </button>
            </div>

            <div className="info-card quickstart-note">
              <strong>{configForm.dryRun ? "当前为 Dry Run 演练模式" : "当前为 Live 自动下单模式"}</strong>
              <p>
                {configForm.dryRun
                  ? "建议先保持 dry-run，等客户端实际接到几条消息并确认仓位无误后，再切换到 live。"
                  : "当前允许真实转发，请再确认默认仓位、券商目标和账户环境。"}
              </p>
            </div>

            <div className="panel-footer compact quickstart-footer">
              <p>{readySteps === 4 ? "核心配置已就绪，可以一键保存。" : `当前完成 ${readySteps}/4，请先补齐剩余项。`}</p>
              <button type="button" disabled={savingQuickConfig} onClick={handleQuickSave}>
                {savingQuickConfig ? "Saving..." : "一键保存跟单配置"}
              </button>
            </div>
          </div>

          <form className="panel gateway-panel" onSubmit={handleConfigSave}>
            <div className="section-heading">
              <div>
                <p className="eyebrow">Broker Target</p>
                <h2>Relay 配置</h2>
              </div>
              <span className="section-chip">实时影响 relay 行为</span>
            </div>

            <div className="form-grid">
              <label className="span-2">
                <span>Broker</span>
                <select
                  value={configForm.broker}
                  onChange={(event) => updateConfigField("broker", event.currentTarget.value as BrokerKind)}
                >
                  {brokerOptions.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                <span>Default Quantity</span>
                <input
                  value={configForm.defaultQuantity}
                  onChange={(event) => updateConfigField("defaultQuantity", event.currentTarget.value)}
                  placeholder="1"
                />
              </label>
              {configForm.broker === "ibkr" ? (
                <>
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
                </>
              ) : (
                <>
                  <label>
                    <span>OpenD Host</span>
                    <input
                      value={configForm.moomooHost}
                      onChange={(event) => updateConfigField("moomooHost", event.currentTarget.value)}
                      placeholder="127.0.0.1"
                    />
                  </label>
                  <label>
                    <span>OpenD Port</span>
                    <input
                      value={configForm.moomooPort}
                      onChange={(event) => updateConfigField("moomooPort", event.currentTarget.value)}
                      placeholder="11111"
                    />
                  </label>
                  <label>
                    <span>Market</span>
                    <select
                      value={configForm.moomooMarket}
                      onChange={(event) => updateConfigField("moomooMarket", event.currentTarget.value.toUpperCase())}
                    >
                      {moomooMarketOptions.map((option) => (
                        <option key={option} value={option}>
                          {option}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    <span>Trade Env</span>
                    <select
                      value={configForm.moomooTrdEnv}
                      onChange={(event) => updateConfigField("moomooTrdEnv", event.currentTarget.value.toUpperCase())}
                    >
                      {moomooTrdEnvOptions.map((option) => (
                        <option key={option} value={option}>
                          {option}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    <span>Account ID</span>
                    <input
                      value={configForm.moomooAccId}
                      onChange={(event) => updateConfigField("moomooAccId", event.currentTarget.value)}
                      placeholder="0"
                    />
                  </label>
                  <label>
                    <span>Security Firm</span>
                    <input
                      value={configForm.moomooSecurityFirm}
                      onChange={(event) => updateConfigField("moomooSecurityFirm", event.currentTarget.value.toUpperCase())}
                      placeholder="FUTUSECURITIES"
                    />
                  </label>
                  <label>
                    <span>Time In Force</span>
                    <select
                      value={configForm.moomooTimeInForce}
                      onChange={(event) => updateConfigField("moomooTimeInForce", event.currentTarget.value.toUpperCase())}
                    >
                      {moomooTimeInForceOptions.map((option) => (
                        <option key={option} value={option}>
                          {option}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    <span>Session</span>
                    <select
                      value={configForm.moomooSession}
                      onChange={(event) => updateConfigField("moomooSession", event.currentTarget.value.toUpperCase())}
                    >
                      {moomooSessionOptions.map((option) => (
                        <option key={option} value={option}>
                          {option}
                        </option>
                      ))}
                    </select>
                  </label>
                </>
              )}
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
                  <p>仅演练 relay 路径，不真正提交券商订单。</p>
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
              {configForm.broker === "moomoo" ? (
                <label className="toggle-card">
                  <input
                    type="checkbox"
                    checked={configForm.moomooFillOutsideRth}
                    onChange={(event) =>
                      updateConfigField("moomooFillOutsideRth", event.currentTarget.checked)
                    }
                  />
                  <div>
                    <strong>Outside RTH</strong>
                    <p>传给 Moomoo 下单接口的 fill_outside_rth 标志。</p>
                  </div>
                </label>
              ) : null}
            </div>

            <div className="panel-footer compact">
              <p>{configDirty ? "有未保存修改" : "配置已同步"}</p>
              <button type="submit" disabled={savingConfig}>
                {savingConfig ? "Saving..." : "Save Broker Config"}
              </button>
            </div>
          </form>

          <div className="panel">
            <div className="section-heading">
              <div>
                <p className="eyebrow">Client Files</p>
                <h2>客户端配置落盘位置</h2>
              </div>
              <span className="section-chip">独立于 exporter .env</span>
            </div>

            <div className="info-card">
              <strong>Config JSON</strong>
              <p>{compactValue(snapshot.storagePaths.clientConfigFile, "启动后生成")}</p>
            </div>
            <div className="info-card">
              <strong>Runtime JSON</strong>
              <p>{compactValue(snapshot.storagePaths.runtimeStateFile, "启动后生成")}</p>
            </div>
          </div>
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

function configToForm(config: BrokerConfig): ConfigFormState {
  return {
    broker: config.broker,
    defaultQuantity: String(config.defaultQuantity),
    host: config.host,
    port: String(config.port),
    clientId: String(config.clientId),
    account: config.account,
    defaultExchange: config.defaultExchange,
    currency: config.currency,
    moomooHost: config.moomooHost,
    moomooPort: String(config.moomooPort),
    moomooMarket: config.moomooMarket,
    moomooTrdEnv: config.moomooTrdEnv,
    moomooAccId: String(config.moomooAccId),
    moomooSecurityFirm: config.moomooSecurityFirm,
    moomooTimeInForce: config.moomooTimeInForce,
    moomooSession: config.moomooSession,
    moomooFillOutsideRth: config.moomooFillOutsideRth,
    dryRun: config.dryRun,
    autoForward: config.autoForward,
  };
}

function configToPayload(form: ConfigFormState): BrokerConfig {
  return {
    broker: form.broker,
    defaultQuantity: parsePositiveNumber(form.defaultQuantity, "Default Quantity"),
    host: form.host.trim(),
    port: parseInteger(form.port, "Port"),
    clientId: parseInteger(form.clientId, "Client ID"),
    account: form.account.trim(),
    defaultExchange: form.defaultExchange.trim().toUpperCase(),
    currency: form.currency.trim().toUpperCase(),
    moomooHost: form.moomooHost.trim(),
    moomooPort: parseInteger(form.moomooPort, "Moomoo Port"),
    moomooMarket: form.moomooMarket.trim().toUpperCase(),
    moomooTrdEnv: form.moomooTrdEnv.trim().toUpperCase(),
    moomooAccId: parseInteger(form.moomooAccId, "Moomoo Account ID"),
    moomooSecurityFirm: form.moomooSecurityFirm.trim().toUpperCase(),
    moomooTimeInForce: form.moomooTimeInForce.trim().toUpperCase(),
    moomooSession: form.moomooSession.trim().toUpperCase(),
    moomooFillOutsideRth: form.moomooFillOutsideRth,
    dryRun: form.dryRun,
    autoForward: form.autoForward,
  };
}

function brokerDisplayName(broker: BrokerKind): string {
  return broker === "moomoo" ? "Moomoo" : "IBKR";
}

function brokerTarget(config: BrokerConfig): string {
  return config.broker === "moomoo"
    ? `${config.moomooHost}:${config.moomooPort}`
    : `${config.host}:${config.port}`;
}

function formBrokerTarget(config: ConfigFormState): string {
  return config.broker === "moomoo"
    ? `OpenD ${config.moomooHost || "?"}:${config.moomooPort || "?"}`
    : `IB Gateway ${config.host || "?"}:${config.port || "?"}`;
}

function brokerSetupHint(config: ConfigFormState): string {
  return config.broker === "moomoo"
    ? "补齐 OpenD host / port，并确认 market 与环境"
    : "补齐 IB Gateway host / port";
}

function isBrokerFormReady(config: ConfigFormState): boolean {
  if (config.broker === "ibkr") {
    return isFilled(config.host) && isPositiveIntegerValue(config.port);
  }

  return (
    isFilled(config.moomooHost) &&
    isPositiveIntegerValue(config.moomooPort) &&
    isFilled(config.moomooMarket) &&
    matchesAllowedValue(config.moomooTrdEnv, moomooTrdEnvOptions) &&
    isFilled(config.moomooTimeInForce) &&
    isFilled(config.moomooSession)
  );
}

function followSetupHint(config: ConfigFormState, natsForm: NatsFormState): string {
  if (!isFilled(natsForm.serverAddress)) {
    return "先填 NATS Server，例如 127.0.0.1:4222。";
  }

  if (!isFilled(natsForm.subject)) {
    return "先填 subject，例如 signals.options.entry。";
  }

  if (!isPositiveNumberValue(config.defaultQuantity)) {
    return "先设置默认仓位大小 Default Quantity，必须大于 0。";
  }

  if (!isBrokerFormReady(config)) {
    return config.broker === "moomoo"
      ? "补齐 Moomoo OpenD 连接信息，并确认交易环境。"
      : "补齐 IB Gateway 的 host 和 port。";
  }

  if (!natsForm.autoSubscribe) {
    return "打开 Auto Subscribe，这样客户端重启后也会自动接收信号。";
  }

  if (!config.autoForward) {
    return "打开 Auto Relay，这样消息入队后才会自动转发到券商。";
  }

  if (config.dryRun) {
    return "核心配置已齐，建议先保持 dry-run 收几条消息验证，再切换 live。";
  }

  return "核心配置已齐，当前已经满足客户自动跟单。";
}

function isFilled(value: string): boolean {
  return value.trim().length > 0;
}

function isPositiveIntegerValue(value: string): boolean {
  return /^\d+$/.test(value.trim()) && Number.parseInt(value, 10) > 0;
}

function isNonNegativeIntegerValue(value: string): boolean {
  return /^\d+$/.test(value.trim());
}

function isPositiveNumberValue(value: string): boolean {
  return /^(?:\d+|\d+\.\d+|\.\d+)$/.test(value.trim()) && Number.parseFloat(value) > 0;
}

function matchesAllowedValue(value: string, options: readonly string[]): boolean {
  return options.includes(value.trim().toUpperCase());
}

function parseInteger(value: string, label: string): number {
  const parsed = Number.parseInt(value, 10);

  if (!Number.isInteger(parsed) || parsed < 0) {
    throw new Error(`${label} 必须是有效整数`);
  }

  return parsed;
}

function parsePositiveNumber(value: string, label: string): number {
  const parsed = Number.parseFloat(value);

  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`${label} 必须是大于 0 的数字`);
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
