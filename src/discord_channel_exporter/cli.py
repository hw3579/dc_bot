from __future__ import annotations

import argparse
import asyncio
import json
import os
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

import aiohttp
from dotenv import load_dotenv

API_BASE = "https://discord.com/api/v10"
DEFAULT_OUTPUT_FILE = "discord_channel_messages.jsonl"


@dataclass(slots=True)
class Settings:
    bot_token: str
    channel_id: str
    output_file: Path
    request_delay_seconds: float
    since_days: float | None
    proxy_url: str | None


@dataclass(slots=True)
class RequestContext:
    proxy_url: str | None
    use_proxy: bool = False


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Export Discord channel history with a bot token."
    )
    parser.add_argument(
        "--env-file",
        default=".env",
        help="Path to the environment file. Default: .env",
    )
    parser.add_argument(
        "--channel-id",
        help="Override DISCORD_CHANNEL_ID from the env file.",
    )
    parser.add_argument(
        "--output-file",
        help="Override DISCORD_OUTPUT_FILE from the env file.",
    )
    parser.add_argument(
        "--since-days",
        type=float,
        help="Only export messages from the last N days.",
    )
    parser.add_argument(
        "--proxy-url",
        help="Retry failed requests through this proxy URL, for example http://127.0.0.1:10808.",
    )
    return parser.parse_args()


def load_settings(
    env_file: str,
    channel_id_override: str | None = None,
    output_file_override: str | None = None,
    since_days_override: float | None = None,
    proxy_url_override: str | None = None,
) -> Settings:
    load_dotenv(env_file, override=False)

    bot_token = os.getenv("DISCORD_BOT_TOKEN", "").strip()
    channel_id = (channel_id_override or os.getenv("DISCORD_CHANNEL_ID", "")).strip()
    output_file = Path(
        (output_file_override or os.getenv("DISCORD_OUTPUT_FILE", DEFAULT_OUTPUT_FILE)).strip()
        or DEFAULT_OUTPUT_FILE
    )
    request_delay_raw = os.getenv("DISCORD_REQUEST_DELAY_SECONDS", "0.25").strip()
    proxy_url = (proxy_url_override or os.getenv("DISCORD_PROXY_URL", "")).strip() or None

    if not bot_token:
        raise RuntimeError(
            f"Missing DISCORD_BOT_TOKEN. Please set it in {env_file}."
        )
    if not channel_id:
        raise RuntimeError(
            f"Missing DISCORD_CHANNEL_ID. Please set it in {env_file}."
        )

    try:
        request_delay_seconds = float(request_delay_raw)
    except ValueError as exc:
        raise RuntimeError(
            "DISCORD_REQUEST_DELAY_SECONDS must be a number."
        ) from exc

    if request_delay_seconds < 0:
        raise RuntimeError("DISCORD_REQUEST_DELAY_SECONDS must be >= 0.")

    if since_days_override is not None and since_days_override < 0:
        raise RuntimeError("--since-days must be >= 0.")

    return Settings(
        bot_token=bot_token,
        channel_id=channel_id,
        output_file=output_file,
        request_delay_seconds=request_delay_seconds,
        since_days=since_days_override,
        proxy_url=proxy_url,
    )


def build_headers(bot_token: str) -> dict[str, str]:
    return {
        "Authorization": f"Bot {bot_token}",
        "User-Agent": "discord-channel-exporter/0.1.0",
    }


def parse_discord_timestamp(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


async def get_messages_page(
    session: aiohttp.ClientSession,
    channel_id: str,
    headers: dict[str, str],
    request_context: RequestContext,
    before: str | None = None,
    after: str | None = None,
    limit: int = 100,
) -> list[dict[str, Any]]:
    if before and after:
        raise ValueError("before and after cannot be used at the same time")

    if limit < 1 or limit > 100:
        raise ValueError("limit must be between 1 and 100")

    params: dict[str, str | int] = {"limit": limit}
    if before:
        params["before"] = before
    if after:
        params["after"] = after

    url = f"{API_BASE}/channels/{channel_id}/messages"

    payload = await request_json(
        session=session,
        url=url,
        headers=headers,
        request_context=request_context,
        params=params,
        forbidden_message=(
            "403 Forbidden: bot lacks View Channel or Read Message History permissions."
        ),
        unexpected_message="Unexpected Discord API response when listing messages.",
    )
    if not isinstance(payload, list):
        raise RuntimeError("Unexpected Discord API response when listing messages.")
    return payload


async def request_json_once(
    session: aiohttp.ClientSession,
    url: str,
    headers: dict[str, str],
    params: dict[str, str | int] | None,
    proxy: str | None,
    forbidden_message: str,
    method: str = "GET",
    json_body: dict[str, Any] | None = None,
) -> Any:
    while True:
        async with session.request(
            method,
            url,
            headers=headers,
            params=params,
            json=json_body,
            proxy=proxy,
        ) as response:
            if response.status == 429:
                payload = await response.json()
                retry_after = float(payload.get("retry_after", 1))
                await asyncio.sleep(retry_after)
                continue

            if response.status == 401:
                raise RuntimeError(
                    "401 Unauthorized: invalid bot token or malformed Authorization header."
                )

            if response.status == 403:
                raise RuntimeError(forbidden_message)

            response.raise_for_status()
            return await response.json()


async def request_json(
    session: aiohttp.ClientSession,
    url: str,
    headers: dict[str, str],
    request_context: RequestContext,
    params: dict[str, str | int] | None,
    forbidden_message: str,
    unexpected_message: str,
    method: str = "GET",
    json_body: dict[str, Any] | None = None,
) -> Any:
    if request_context.use_proxy and request_context.proxy_url:
        payload = await request_json_once(
            session=session,
            url=url,
            headers=headers,
            params=params,
            proxy=request_context.proxy_url,
            forbidden_message=forbidden_message,
            method=method,
            json_body=json_body,
        )
        if payload is None:
            raise RuntimeError(unexpected_message)
        return payload

    try:
        payload = await request_json_once(
            session=session,
            url=url,
            headers=headers,
            params=params,
            proxy=None,
            forbidden_message=forbidden_message,
            method=method,
            json_body=json_body,
        )
    except (aiohttp.ClientConnectionError, asyncio.TimeoutError) as exc:
        if not request_context.proxy_url:
            raise RuntimeError(f"Request failed without proxy: {exc}") from exc
        request_context.use_proxy = True
        print(f"Direct request failed; retrying with proxy {request_context.proxy_url}")
        payload = await request_json_once(
            session=session,
            url=url,
            headers=headers,
            params=params,
            proxy=request_context.proxy_url,
            forbidden_message=forbidden_message,
            method=method,
            json_body=json_body,
        )

    if payload is None:
        raise RuntimeError(unexpected_message)
    return payload


async def get_channel_info(
    session: aiohttp.ClientSession,
    channel_id: str,
    headers: dict[str, str],
    request_context: RequestContext,
) -> dict[str, Any]:
    url = f"{API_BASE}/channels/{channel_id}"
    payload = await request_json(
        session=session,
        url=url,
        headers=headers,
        request_context=request_context,
        params=None,
        forbidden_message="403 Forbidden: bot lacks permission to view this channel.",
        unexpected_message="Unexpected Discord API response when reading channel info.",
    )
    if not isinstance(payload, dict):
        raise RuntimeError("Unexpected Discord API response when reading channel info.")
    return payload


async def create_channel_message(
    session: aiohttp.ClientSession,
    channel_id: str,
    headers: dict[str, str],
    request_context: RequestContext,
    content: str,
) -> dict[str, Any]:
    url = f"{API_BASE}/channels/{channel_id}/messages"
    payload = await request_json(
        session=session,
        url=url,
        headers=headers,
        request_context=request_context,
        params=None,
        forbidden_message="403 Forbidden: bot lacks permission to send messages to this channel.",
        unexpected_message="Unexpected Discord API response when creating a message.",
        method="POST",
        json_body={
            "content": content,
            "allowed_mentions": {"parse": []},
        },
    )
    if not isinstance(payload, dict):
        raise RuntimeError("Unexpected Discord API response when creating a message.")
    return payload


def serialize_message(message: dict[str, Any]) -> dict[str, Any]:
    return {
        "id": message.get("id"),
        "channel_id": message.get("channel_id"),
        "timestamp": message.get("timestamp"),
        "edited_timestamp": message.get("edited_timestamp"),
        "author_id": message.get("author", {}).get("id"),
        "author_username": message.get("author", {}).get("username"),
        "author_global_name": message.get("author", {}).get("global_name"),
        "content": message.get("content", ""),
        "attachments": [
            {
                "id": attachment.get("id"),
                "filename": attachment.get("filename"),
                "url": attachment.get("url"),
                "content_type": attachment.get("content_type"),
                "size": attachment.get("size"),
            }
            for attachment in message.get("attachments", [])
        ],
        "embeds": message.get("embeds", []),
        "mentions": message.get("mentions", []),
        "reference": message.get("message_reference"),
    }


async def export_all_messages(settings: Settings) -> int:
    all_messages: list[dict[str, Any]] = []
    before: str | None = None
    headers = build_headers(settings.bot_token)
    cutoff = None
    request_context = RequestContext(proxy_url=settings.proxy_url)

    if settings.since_days is not None:
        cutoff = datetime.now(timezone.utc) - timedelta(days=settings.since_days)

    timeout = aiohttp.ClientTimeout(total=60, connect=10, sock_connect=10, sock_read=60)

    async with aiohttp.ClientSession(timeout=timeout) as session:
        channel = await get_channel_info(
            session=session,
            channel_id=settings.channel_id,
            headers=headers,
            request_context=request_context,
        )
        print(
            "Channel "
            f"{channel.get('name') or '<unnamed>'} "
            f"(id={channel.get('id')}, guild_id={channel.get('guild_id')})"
        )

        while True:
            batch = await get_messages_page(
                session=session,
                channel_id=settings.channel_id,
                headers=headers,
                request_context=request_context,
                before=before,
            )

            if not batch:
                break

            reached_cutoff = False
            for message in batch:
                if cutoff is None:
                    all_messages.append(message)
                    continue

                timestamp = message.get("timestamp")
                if not timestamp:
                    continue

                if parse_discord_timestamp(timestamp) >= cutoff:
                    all_messages.append(message)
                    continue

                reached_cutoff = True
                break

            before = batch[-1]["id"]
            print(f"Fetched {len(all_messages)} messages so far; oldest id={before}")

            if reached_cutoff or len(batch) < 100:
                break

            if settings.request_delay_seconds > 0:
                await asyncio.sleep(settings.request_delay_seconds)

    all_messages.reverse()
    settings.output_file.parent.mkdir(parents=True, exist_ok=True)

    with settings.output_file.open("w", encoding="utf-8") as output_handle:
        for message in all_messages:
            output_handle.write(
                json.dumps(serialize_message(message), ensure_ascii=False) + "\n"
            )

    return len(all_messages)


def main() -> None:
    args = parse_args()
    settings = load_settings(
        args.env_file,
        channel_id_override=args.channel_id,
        output_file_override=args.output_file,
        since_days_override=args.since_days,
        proxy_url_override=args.proxy_url,
    )
    total = asyncio.run(export_all_messages(settings))
    print(f"Exported {total} messages to {settings.output_file}")


if __name__ == "__main__":
    main()