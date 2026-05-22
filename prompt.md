可以。不要用网页爬虫，也不要用用户 token/self-bot；正确做法是让你的 **Discord App Bot** 通过官方 API 拉取频道历史消息。Discord 明确禁止用普通用户账号自动化 self-bot；API 调用应使用 Bot token。([Discord Support][1])

## 1. 先确认权限

你的机器人必须满足：

1. 已被邀请进对应 server；
2. 对目标频道有：

   * **View Channel**
   * **Read Message History**
3. 如果要读取正文 `content`、附件、embed 等，通常还要在 Developer Portal → Bot 里开启 **Message Content Intent**。Discord 官方说明，Message Content 控制的是 message object 里的 `content`、`embeds`、`attachments`、`components` 等字段访问；未验证 app 可使用但必须开启，规模化 verified bot 需要申请。([Discord Support][2])

官方的历史消息接口是：

```http
GET /channels/{channel.id}/messages
```

它会按 **newest to oldest** 返回消息；单次 `limit` 最大只能是 **100**，所以要用 `before=<oldest_message_id>` 不断向前翻页。目标频道中如果 bot 没有 `VIEW_CHANNEL` 或 `READ_MESSAGE_HISTORY` 权限，就拿不到消息。([Discord Docs][3])

## 2. Python REST 方式：最直接

安装依赖：

```bash
pip install aiohttp python-dotenv
```

创建 `.env`：

```bash
DISCORD_BOT_TOKEN=你的_bot_token
DISCORD_CHANNEL_ID=目标频道ID
```

脚本 `export_discord_channel.py`：

```python
import os
import json
import asyncio
import aiohttp
from dotenv import load_dotenv

load_dotenv()

TOKEN = os.getenv("DISCORD_BOT_TOKEN")
CHANNEL_ID = os.getenv("DISCORD_CHANNEL_ID")

API_BASE = "https://discord.com/api/v10"
HEADERS = {
    "Authorization": f"Bot {TOKEN}",
    "User-Agent": "channel-archive-bot/1.0"
}


async def get_messages_page(session, before=None):
    params = {"limit": 100}
    if before:
        params["before"] = before

    url = f"{API_BASE}/channels/{CHANNEL_ID}/messages"

    while True:
        async with session.get(url, headers=HEADERS, params=params) as resp:
            if resp.status == 429:
                data = await resp.json()
                retry_after = float(data.get("retry_after", 1))
                await asyncio.sleep(retry_after)
                continue

            if resp.status == 403:
                raise RuntimeError("403 Forbidden：Bot 没有 View Channel / Read Message History 权限，或频道权限覆盖拒绝了 Bot。")

            if resp.status == 401:
                raise RuntimeError("401 Unauthorized：Bot token 错误，或 Authorization 头格式错误。")

            resp.raise_for_status()
            return await resp.json()


async def export_all_messages():
    all_messages = []
    before = None

    async with aiohttp.ClientSession() as session:
        while True:
            batch = await get_messages_page(session, before=before)

            if not batch:
                break

            all_messages.extend(batch)
            before = batch[-1]["id"]

            print(f"已拉取 {len(all_messages)} 条，当前最早 message_id = {before}")

            if len(batch) < 100:
                break

            # 不要硬冲 API，简单留一点间隔
            await asyncio.sleep(0.25)

    # Discord 返回是新 -> 旧，这里反转成旧 -> 新
    all_messages.reverse()

    with open("discord_channel_messages.jsonl", "w", encoding="utf-8") as f:
        for msg in all_messages:
            row = {
                "id": msg.get("id"),
                "channel_id": msg.get("channel_id"),
                "timestamp": msg.get("timestamp"),
                "edited_timestamp": msg.get("edited_timestamp"),
                "author_id": msg.get("author", {}).get("id"),
                "author_username": msg.get("author", {}).get("username"),
                "author_global_name": msg.get("author", {}).get("global_name"),
                "content": msg.get("content", ""),
                "attachments": [
                    {
                        "id": a.get("id"),
                        "filename": a.get("filename"),
                        "url": a.get("url"),
                        "content_type": a.get("content_type"),
                        "size": a.get("size"),
                    }
                    for a in msg.get("attachments", [])
                ],
                "embeds": msg.get("embeds", []),
                "mentions": msg.get("mentions", []),
                "reference": msg.get("message_reference"),
            }
            f.write(json.dumps(row, ensure_ascii=False) + "\n")

    print(f"完成：共导出 {len(all_messages)} 条消息 -> discord_channel_messages.jsonl")


if __name__ == "__main__":
    if not TOKEN or not CHANNEL_ID:
        raise RuntimeError("请先在 .env 里设置 DISCORD_BOT_TOKEN 和 DISCORD_CHANNEL_ID")

    asyncio.run(export_all_messages())
```

运行：

```bash
python export_discord_channel.py
```

## 3. 常见问题

如果导出的 `content` 是空的，优先检查 **Message Content Intent** 是否在 Developer Portal 开启；Discord 官方说该 intent 控制 `content / embeds / attachments / components` 等字段访问。([Discord Support][2])

如果返回 403，通常是频道权限问题：bot 虽然在 server 里，但目标频道可能单独覆盖了权限，导致它没有 **View Channel** 或 **Read Message History**。Discord 的消息接口明确要求这些权限。([Discord Docs][3])

如果频道消息很多，不要自己写死请求频率。Discord 官方说明 rate limit 会按 route 和全局维度变化，应用应读取响应头或处理 429 的 `retry_after`，而不是硬编码固定限额。上面的脚本已经处理了 429。([Discord Docs][4])

如果是 **论坛频道 / thread**，每个帖子本质上是单独的 thread channel，你需要对每个 thread 的 channel_id 分别拉消息；普通文本频道只需要目标 channel_id。

[1]: https://support.discord.com/hc/en-us/articles/115002192352-Automated-User-Accounts-Self-Bots?utm_source=chatgpt.com "Automated User Accounts (Self-Bots)"
[2]: https://support-dev.discord.com/hc/en-us/articles/6207308062871-What-are-Privileged-Intents "What are Privileged Intents? – Developers"
[3]: https://docs.discord.com/developers/resources/message "Message Resource - Documentation - Discord"
[4]: https://docs.discord.com/developers/topics/rate-limits "Rate Limits - Documentation - Discord"
