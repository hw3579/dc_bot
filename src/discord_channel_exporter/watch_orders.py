from __future__ import annotations

import argparse
import asyncio
import json
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import aiohttp
import nats
from dotenv import load_dotenv

from .cli import (
    RequestContext,
    Settings,
    build_headers,
    create_channel_message,
    get_channel_info,
    get_messages_page,
    load_settings,
    serialize_message,
)
from .filter_orders import default_output_path, filter_message, prepare_record_for_relay
from .nats_topic import (
    NatsConfig,
    build_envelope,
    build_subject,
    load_nats_config,
    now_rfc3339,
    parse_categories,
)


@dataclass(slots=True)
class WatchSettings:
    discord: Settings
    nats: NatsConfig
    categories: set[str]
    min_score: int
    poll_interval_seconds: float
    state_file: Path
    default_contract_type: str | None
    default_expiry_label: str | None
    audit_channel_id: str | None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Monitor Discord for new order-like messages and publish them to NATS."
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
        help="Override DISCORD_OUTPUT_FILE from the env file for the raw JSONL audit log.",
    )
    parser.add_argument(
        "--proxy-url",
        help="Retry failed Discord requests through this proxy URL.",
    )
    parser.add_argument(
        "--server-address",
        help="Override NATS_SERVER_ADDRESS. Default: 127.0.0.1:4222",
    )
    parser.add_argument(
        "--subject-template",
        help="Override NATS_SUBJECT. Supports {category}, for example signals.options.{category}.",
    )
    parser.add_argument(
        "--categories",
        default="entry",
        help="Comma-separated categories to publish. Default: entry",
    )
    parser.add_argument(
        "--min-score",
        type=int,
        default=0,
        help="Only publish messages that meet or exceed this filter score.",
    )
    parser.add_argument(
        "--poll-interval",
        type=float,
        help="Polling interval in seconds. Default: DISCORD_POLL_INTERVAL_SECONDS or 2.0",
    )
    parser.add_argument(
        "--state-file",
        help="Persist the last processed Discord message id to this JSON file.",
    )
    parser.add_argument(
        "--default-contract-type",
        help="Optional fallback contract type for incomplete entry messages: call or put.",
    )
    parser.add_argument(
        "--default-expiry-label",
        help="Optional fallback expiry label for incomplete entry messages, for example weekly or 0dte.",
    )
    parser.add_argument(
        "--audit-channel-id",
        help="Optional Discord channel id that receives the raw payload prepared for the Tauri app.",
    )
    return parser.parse_args()


def load_watch_settings(args: argparse.Namespace) -> WatchSettings:
    load_dotenv(args.env_file, override=False)

    discord_settings = load_settings(
        args.env_file,
        channel_id_override=args.channel_id,
        output_file_override=args.output_file,
        proxy_url_override=args.proxy_url,
    )
    nats_config = load_nats_config(
        env_file=args.env_file,
        server_address_override=args.server_address,
        subject_template_override=args.subject_template,
    )

    poll_interval_raw = args.poll_interval
    if poll_interval_raw is None:
        poll_interval_raw = os.getenv("DISCORD_POLL_INTERVAL_SECONDS", "2.0").strip()

    try:
        poll_interval_seconds = float(poll_interval_raw)
    except ValueError as exc:
        raise RuntimeError("DISCORD_POLL_INTERVAL_SECONDS must be a number") from exc

    if poll_interval_seconds <= 0:
        raise RuntimeError("poll interval must be > 0")

    state_file = Path(
        (args.state_file or os.getenv("DISCORD_MONITOR_STATE_FILE", "")).strip()
        or f"data/discord_channel_{discord_settings.channel_id}_watch_state.json"
    )

    return WatchSettings(
        discord=discord_settings,
        nats=nats_config,
        categories=parse_categories(args.categories),
        min_score=args.min_score,
        poll_interval_seconds=poll_interval_seconds,
        state_file=state_file,
        default_contract_type=(
            (args.default_contract_type or os.getenv("DISCORD_DEFAULT_ENTRY_CONTRACT_TYPE", "")).strip()
            or None
        ),
        default_expiry_label=(
            (args.default_expiry_label or os.getenv("DISCORD_DEFAULT_ENTRY_EXPIRY_LABEL", "")).strip()
            or None
        ),
        audit_channel_id=(
            (args.audit_channel_id or os.getenv("DISCORD_AUDIT_CHANNEL_ID", "")).strip()
            or None
        ),
    )


def load_cursor(state_file: Path, channel_id: str) -> str | None:
    if not state_file.exists():
        return None

    payload = json.loads(state_file.read_text(encoding="utf-8"))
    if payload.get("channel_id") != channel_id:
        return None

    cursor = str(payload.get("last_message_id", "")).strip()
    return cursor or None


def save_cursor(state_file: Path, channel_id: str, message_id: str) -> None:
    state_file.parent.mkdir(parents=True, exist_ok=True)
    state_file.write_text(
        json.dumps(
            {
                "channel_id": channel_id,
                "last_message_id": message_id,
            },
            ensure_ascii=False,
            indent=2,
        ),
        encoding="utf-8",
    )


def append_jsonl(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(payload, ensure_ascii=False) + "\n")


async def fetch_new_messages(
    session: aiohttp.ClientSession,
    settings: WatchSettings,
    headers: dict[str, str],
    request_context: RequestContext,
    cursor: str,
) -> list[dict[str, Any]]:
    collected: list[dict[str, Any]] = []
    after = cursor

    while True:
        batch = await get_messages_page(
            session=session,
            channel_id=settings.discord.channel_id,
            headers=headers,
            request_context=request_context,
            after=after,
            limit=100,
        )

        if not batch:
            return collected

        ordered_batch = sorted(
            batch,
            key=lambda message: int(str(message.get("id", "0"))),
        )
        collected.extend(ordered_batch)
        after = str(ordered_batch[-1]["id"])

        if len(batch) < 100:
            return collected

        if settings.discord.request_delay_seconds > 0:
            await asyncio.sleep(settings.discord.request_delay_seconds)


async def publish_live_record(
    nc: Any,
    settings: WatchSettings,
    record: dict[str, Any],
) -> None:
    subject = build_subject(settings.nats.subject_template, record)
    envelope = build_envelope(
        subject,
        record,
        f"discord-watch:{settings.discord.channel_id}",
    )
    await nc.publish(
        subject,
        json.dumps(envelope, ensure_ascii=False).encode("utf-8"),
    )


def build_audit_payload(
    settings: WatchSettings,
    record: dict[str, Any],
    relay_error: str | None,
) -> dict[str, Any]:
    subject = build_subject(settings.nats.subject_template, record)
    return {
        "eventType": "discord.order_signal.audit",
        "publishedAt": now_rfc3339(),
        "subject": subject,
        "relayReady": relay_error is None,
        "relayError": relay_error,
        "signal": record,
    }


def split_json_for_discord(payload: dict[str, Any]) -> list[str]:
    body = json.dumps(payload, ensure_ascii=False, indent=2)
    max_chunk_length = 1800
    return [body[index : index + max_chunk_length] for index in range(0, len(body), max_chunk_length)] or [body]


async def send_audit_payload(
    session: aiohttp.ClientSession,
    headers: dict[str, str],
    request_context: RequestContext,
    settings: WatchSettings,
    payload: dict[str, Any],
) -> None:
    if settings.audit_channel_id is None:
        return

    chunks = split_json_for_discord(payload)
    total = len(chunks)
    message_id = str(payload.get("signal", {}).get("message_id", "unknown"))

    for index, chunk in enumerate(chunks, start=1):
        prefix = (
            f"Tauri payload audit for source message {message_id} ({index}/{total})"
            if total > 1
            else f"Tauri payload audit for source message {message_id}"
        )
        await create_channel_message(
            session=session,
            channel_id=settings.audit_channel_id,
            headers=headers,
            request_context=request_context,
            content=f"{prefix}\n```json\n{chunk}\n```",
        )


async def monitor_orders(settings: WatchSettings) -> None:
    headers = build_headers(settings.discord.bot_token)
    request_context = RequestContext(proxy_url=settings.discord.proxy_url)
    timeout = aiohttp.ClientTimeout(total=60, connect=10, sock_connect=10, sock_read=60)
    raw_output_path = settings.discord.output_file
    filtered_output_path = default_output_path(raw_output_path)
    cursor = load_cursor(settings.state_file, settings.discord.channel_id)

    async with aiohttp.ClientSession(timeout=timeout) as session:
        channel = await get_channel_info(
            session=session,
            channel_id=settings.discord.channel_id,
            headers=headers,
            request_context=request_context,
        )
        print(
            "Monitoring channel "
            f"{channel.get('name') or '<unnamed>'} "
            f"(id={channel.get('id')}, guild_id={channel.get('guild_id')})"
        )

        if cursor is None:
            latest = await get_messages_page(
                session=session,
                channel_id=settings.discord.channel_id,
                headers=headers,
                request_context=request_context,
                limit=1,
            )
            if latest:
                cursor = str(latest[0]["id"])
                save_cursor(settings.state_file, settings.discord.channel_id, cursor)
                print(
                    f"Initialized monitor cursor at latest message {cursor}; waiting for new messages."
                )
            else:
                print("Channel is currently empty; waiting for the first message.")

        nc = await nats.connect(servers=[settings.nats.server_address])
        try:
            while True:
                if cursor is None:
                    latest = await get_messages_page(
                        session=session,
                        channel_id=settings.discord.channel_id,
                        headers=headers,
                        request_context=request_context,
                        limit=1,
                    )
                    if latest:
                        cursor = str(latest[0]["id"])
                        save_cursor(settings.state_file, settings.discord.channel_id, cursor)
                        print(
                            f"Initialized monitor cursor at latest message {cursor}; waiting for new messages."
                        )

                    await asyncio.sleep(settings.poll_interval_seconds)
                    continue

                new_messages = await fetch_new_messages(
                    session=session,
                    settings=settings,
                    headers=headers,
                    request_context=request_context,
                    cursor=cursor,
                )

                published = 0
                for message in new_messages:
                    next_cursor = str(message["id"])
                    serialized = serialize_message(message)
                    append_jsonl(raw_output_path, serialized)

                    record = filter_message(serialized, min_score=settings.min_score)
                    if record is None or record["category"] not in settings.categories:
                        cursor = next_cursor
                        save_cursor(settings.state_file, settings.discord.channel_id, cursor)
                        continue

                    prepared_record, relay_error = prepare_record_for_relay(
                        record,
                        default_contract_type=settings.default_contract_type,
                        default_expiry_label=settings.default_expiry_label,
                    )
                    await send_audit_payload(
                        session=session,
                        headers=headers,
                        request_context=request_context,
                        settings=settings,
                        payload=build_audit_payload(settings, prepared_record, relay_error),
                    )
                    if relay_error is not None:
                        print(
                            f"Skipping Discord message {next_cursor}: {relay_error}. "
                            "Set DISCORD_DEFAULT_ENTRY_CONTRACT_TYPE or DISCORD_DEFAULT_ENTRY_EXPIRY_LABEL if you want explicit fallbacks."
                        )
                        cursor = next_cursor
                        save_cursor(settings.state_file, settings.discord.channel_id, cursor)
                        continue

                    await publish_live_record(nc, settings, prepared_record)
                    append_jsonl(filtered_output_path, prepared_record)
                    published += 1
                    cursor = next_cursor
                    save_cursor(settings.state_file, settings.discord.channel_id, cursor)

                if published > 0:
                    await nc.flush()
                    print(f"Published {published} new order-like messages")

                await asyncio.sleep(settings.poll_interval_seconds)
        finally:
            await nc.drain()


def main() -> None:
    settings = load_watch_settings(parse_args())
    asyncio.run(monitor_orders(settings))


if __name__ == "__main__":
    main()