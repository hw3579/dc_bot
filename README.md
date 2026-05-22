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

如果要导出指定频道最近 3 天的消息：

```bash
uv run discord-export --channel-id 1496916217523470468 --since-days 3 --output-file discord_channel_messages_last_3_days.jsonl
```