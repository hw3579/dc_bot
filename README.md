# Discord Channel Exporter

这个项目使用 `uv` 管理 Python 环境，并通过 Discord 官方 API 拉取某个频道的历史消息。

## 前提

- Bot 已加入目标服务器
- Bot 具备 `View Channel` 和 `Read Message History` 权限
- 如果要读取消息正文、附件、embed，请在 Developer Portal 开启 `Message Content Intent`

## 初始化

```bash
uv sync
cp .env.example .env
```

然后编辑 `.env`，填写：

- `DISCORD_BOT_TOKEN`
- `DISCORD_CHANNEL_ID`
- 可选：`DISCORD_OUTPUT_FILE`
- 可选：`DISCORD_REQUEST_DELAY_SECONDS`
- 可选：`DISCORD_PROXY_URL`，例如 `http://127.0.0.1:10808`
- 可选：`DISCORD_POLL_INTERVAL_SECONDS`，实时监控轮询间隔，默认 `2.0`
- 可选：`DISCORD_MONITOR_STATE_FILE`，实时监控游标状态文件
- 可选：`DISCORD_DEFAULT_ENTRY_CONTRACT_TYPE`，当消息没写 calls/puts 时的显式兜底值
- 可选：`DISCORD_DEFAULT_ENTRY_EXPIRY_LABEL`，当消息没写到期标签时的显式兜底值
- 可选：`DISCORD_AUDIT_CHANNEL_ID`，把准备发给 Tauri 的原始 JSON 发到这个校验频道

## 运行

```bash
uv run discord-export
```

也可以显式指定配置文件：

```bash
uv run discord-export --env-file .env
```

如果只导出最近 3 天，并在直连失败时回退到本地代理：

```bash
uv run discord-export --channel-id 1496916217523470468 --since-days 3 --proxy-url http://127.0.0.1:10808
```

## 筛选下单信息

导出后，可以用规则筛选器从 JSONL 里挑出更像下单/加仓/减仓/持仓更新的消息：

```bash
uv run discord-filter-orders data/discord_channel_1496916217523470468_last3d.jsonl
```

默认会输出到同目录下的 `.orders.jsonl` 文件，并按以下类别分类：

- `entry`：更像新开仓或给出具体进场价格
- `add`：更像加仓或继续加码
- `exit`：更像减仓、止盈、卖出、落袋
- `update`：更像持仓更新或仓位计划

也可以只保留某些类别：

```bash
uv run discord-filter-orders data/discord_channel_1496916217523470468_last3d.jsonl --categories entry,exit
```

## 通过 NATS Topic 分发

筛选完成后，可以把结果通过 NATS subject 发出去。当前项目默认读取：

- `NATS_SERVER_ADDRESS`，默认 `127.0.0.1:4222`
- `NATS_SUBJECT`，默认 `signals.options.entry`

发布 `entry` 类消息：

```bash
uv run discord-publish-orders data/discord_channel_1496916217523470468_last3d.orders.jsonl
```

如果你希望按类别拆 topic：

```bash
uv run discord-publish-orders \
	data/discord_channel_1496916217523470468_last3d.orders.jsonl \
	--categories entry,add,exit,update \
	--subject-template 'signals.options.{category}'
```

这样会分别发到：

- `signals.options.entry`
- `signals.options.add`
- `signals.options.exit`
- `signals.options.update`

Leaf 节点订阅示例：

```bash
uv run discord-subscribe-topic --subject 'signals.options.>'
```

如果你希望每个 leaf 节点都收到同一条指令，不要设置 queue group。只有在要做消费负载均衡时，才给订阅端传 `--queue-group`。

如果要导出指定频道最近 3 天的消息：

```bash
uv run discord-export --channel-id 1496916217523470468 --since-days 3 --output-file discord_channel_messages_last_3_days.jsonl
```

## 实时监控并自动发布

如果你希望服务端常驻监控 Discord，只要来了新的 order-like 消息就立刻发到本机 NATS：

```bash
uv run discord-watch-orders
```

这个命令会：

- 读取上面的 Discord / NATS 配置
- 在 `DISCORD_MONITOR_STATE_FILE` 里保存最后处理过的 message id，避免重启后重复发历史消息
- 把所有新消息追加到 `DISCORD_OUTPUT_FILE`
- 把命中的 order-like 记录追加到对应的 `.orders.jsonl`
- 只对“新消息”做过滤和发布，不会在首次启动时把整个历史频道重新扫一遍

如果消息里缺少 calls/puts 或缺少 expiry label，而你又接受显式默认值，可以在 `.env` 里设置：

- `DISCORD_DEFAULT_ENTRY_CONTRACT_TYPE=call`
- `DISCORD_DEFAULT_ENTRY_EXPIRY_LABEL=weekly`

如果 entry 文案里缺少 expiry label，但结构已经足够明确，监控器还会按消息时间换算到美东时区，并自动补成该自然周的周五到期日；audit 里会额外标记 `expiry_inferred=current_week_friday_eastern`。

仍然缺少必要字段时，监控器会跳过这些信息不完整、无法安全自动下单的消息，并在日志里说明原因。

如果你设置了 `DISCORD_AUDIT_CHANNEL_ID`，每条命中的 order-like 消息还会额外把“准备发给 Tauri 的原始 JSON”发到该频道，内容里会带上：

- `relayReady`：这条消息是否已经满足下发条件
- `relayError`：如果还不能正式发给 relay，缺的是哪几个字段
- `signal`：当前解析出来的原始字段

## Linux systemd 安装

仓库现在自带 user-level systemd 安装和卸载脚本，以及两个 service 模板：

- `scripts/install-systemd-user.sh`
- `scripts/uninstall-systemd-user.sh`
- `systemd/user/dc-bot-discord-watch.service.tpl`
- `systemd/user/dc-bot-ib-relay.service.tpl`

默认会安装两个服务：

- `dc-bot-discord-watch.service`：常驻轮询 Discord 并把新消息发到 NATS
- `dc-bot-ib-relay.service`：以 `--headless` 模式启动本地 relay 并自动订阅 NATS；broker/NATS 配置由客户端自己的 JSON 管理

如果两端在同一台机器：

```bash
./scripts/install-systemd-user.sh
```

如果这台机器只跑 Discord watcher：

```bash
./scripts/install-systemd-user.sh --watcher-only
```

如果这台机器只跑 headless relay：

```bash
./scripts/install-systemd-user.sh --relay-only
```

安装脚本会把 release relay 二进制复制到 `~/.local/share/dc-bot/bin/ib-options-relay`，并把 user service 写到 `~/.config/systemd/user/`。

卸载示例：

```bash
./scripts/uninstall-systemd-user.sh
./scripts/uninstall-systemd-user.sh --watcher-only
./scripts/uninstall-systemd-user.sh --relay-only
```

常用 systemd 命令：

```bash
systemctl --user status dc-bot-discord-watch.service
systemctl --user status dc-bot-ib-relay.service
journalctl --user -u dc-bot-discord-watch.service -f
journalctl --user -u dc-bot-ib-relay.service -f
```

如果你希望 user service 在退出桌面会话后也继续跑，需要额外启用 linger：

```bash
sudo loginctl enable-linger "$USER"
```

## Tauri relay 多券商

桌面端 / headless relay 现在已经把券商执行层拆成独立模块，当前支持：

- `broker=ibkr`：继续走原来的 `ibapi` 异步下单
- `broker=moomoo`：通过 PyO3 调 Python SDK，再调用 `moomoo-api` / `futu-api`

默认桌面便携版现在只打包 IBKR / NATS 原生链路，不包含 Moomoo Python bridge，这样 Windows 可执行文件不会再依赖 `python312.dll`。
如果你需要 Moomoo，请在构建时显式开启 `moomoo-python` feature，例如在 `app/` 下执行 `pnpm build:desktop:moomoo`。
GitHub Actions 现在还会额外产出一个 `options-relay-windows-moomoo.zip`，其中包含精简过的 Windows embeddable Python runtime 和 `moomoo-api` 依赖，给没有本地 Python 的机器直接使用。

客户端配置现在独立保存在 Tauri/headless 自己的 JSON 文件里，不再依赖 exporter 的 `.env`：

- Broker 配置和 NATS 订阅配置：保存在 `options-relay-config.json`
- 消息队列和投递结果：保存在 `options-relay-runtime.json`
- 如果设置了 `OPTIONS_RELAY_HOME`，这两个 JSON 会优先写到该目录
- 如果目录里只有旧的 `options-relay-state.json`，当前版本会继续读取，并在下一次保存时迁移成新格式

如果这是给客户直接使用的跟单客户端，Tauri 页面现在有一个“客户跟单快速配置”区，可以按这个顺序操作：

1. 先点 `IBKR 模拟模板` 或 `Moomoo 模拟模板`
2. 填好 NATS Server 和 Subject
3. 设置 `Default Quantity` 作为默认仓位大小
4. 打开 `Auto Subscribe` 和 `Auto Relay`
5. 先保持 `Dry Run`，点 `一键保存跟单配置`
6. 等客户端实际接到几条消息并确认仓位无误后，再切到 Live

Moomoo 模式需要：

- 使用带 `moomoo-python` feature 的桌面构建，或者直接使用 CI 产出的 `options-relay-windows-moomoo.zip`
- 本机已启动 OpenD
- 如果是自己本地构建/运行，还需要可用的 Python 3.12 运行时，以及已安装 `moomoo-api`
- 在客户端 UI 或客户端 config JSON 里配置 `Moomoo Host`、`OpenD Port`、`Trade Env` 等字段
- 如果你用的是 `REAL` 环境，还需要额外提供 `MOOMOO_TRADE_PASSWORD` 或 `MOOMOO_TRADE_PASSWORD_MD5`

Moomoo 期权下单时，relay 会先根据 `symbol + expiry + call/put + strike` 查询 option chain，再把匹配到的期权代码传给交易接口，所以上游仍然只需要传统一的下单指令 JSON。
