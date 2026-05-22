from __future__ import annotations

import argparse
import asyncio
import json
import os
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import nats
from dotenv import load_dotenv

DEFAULT_SERVER_ADDRESS = "127.0.0.1:4222"
DEFAULT_SUBJECT = "signals.options.entry"


@dataclass(slots=True)
class NatsConfig:
    server_address: str
    subject_template: str


def now_rfc3339() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def parse_categories(value: str) -> set[str]:
    return {item.strip() for item in value.split(",") if item.strip()}


def normalize_server_address(value: str) -> str:
    if value.startswith(("nats://", "tls://", "ws://", "wss://")):
        return value
    return f"nats://{value}"


def load_nats_config(
    env_file: str,
    server_address_override: str | None = None,
    subject_template_override: str | None = None,
) -> NatsConfig:
    load_dotenv(env_file, override=False)

    server_address = (
        server_address_override
        or os.getenv("NATS_SERVER_ADDRESS", "")
        or os.getenv("NATS_WS_ENDPOINT", "")
    ).strip()
    subject_template = (
        subject_template_override or os.getenv("NATS_SUBJECT", DEFAULT_SUBJECT)
    ).strip()

    if not server_address:
        server_address = DEFAULT_SERVER_ADDRESS
    if not subject_template:
        subject_template = DEFAULT_SUBJECT

    return NatsConfig(
        server_address=normalize_server_address(server_address),
        subject_template=subject_template,
    )


def load_filtered_messages(path: Path) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        payload = json.loads(line)
        if not isinstance(payload, dict) or "category" not in payload:
            raise RuntimeError(
                "Input file must be the output of discord-filter-orders and include a category field."
            )
        records.append(payload)
    return records


def build_subject(subject_template: str, record: dict[str, Any]) -> str:
    return subject_template.format(category=str(record.get("category", "unknown")))


def build_envelope(subject: str, record: dict[str, Any], source_file: Path) -> dict[str, Any]:
    return {
        "eventType": "discord.order_signal",
        "publishedAt": now_rfc3339(),
        "subject": subject,
        "sourceFile": source_file.name,
        "signal": record,
    }


async def publish_records(
    input_path: Path,
    config: NatsConfig,
    categories: set[str],
    limit: int | None,
) -> int:
    records = load_filtered_messages(input_path)
    filtered = [record for record in records if str(record.get("category")) in categories]
    if limit is not None:
        filtered = filtered[:limit]

    if not filtered:
        print("No matching records to publish")
        return 0

    nc = await nats.connect(servers=[config.server_address])
    try:
        for record in filtered:
            subject = build_subject(config.subject_template, record)
            envelope = build_envelope(subject, record, input_path)
            await nc.publish(
                subject,
                json.dumps(envelope, ensure_ascii=False).encode("utf-8"),
            )
        await nc.flush()
    finally:
        await nc.drain()

    subjects = sorted({build_subject(config.subject_template, record) for record in filtered})
    print(f"Published {len(filtered)} messages to {config.server_address}")
    for subject in subjects:
        print(f"- {subject}")
    return len(filtered)


async def subscribe_subject(
    config: NatsConfig,
    queue_group: str,
    max_messages: int,
    pretty: bool,
) -> None:
    nc = await nats.connect(servers=[config.server_address])
    received = 0
    done = asyncio.Event()

    async def handler(message: Any) -> None:
        nonlocal received
        received += 1
        payload = message.data.decode("utf-8")
        if pretty:
            try:
                parsed = json.loads(payload)
            except json.JSONDecodeError:
                print(payload)
            else:
                print(json.dumps(parsed, ensure_ascii=False, indent=2))
        else:
            print(payload)

        if max_messages > 0 and received >= max_messages:
            done.set()

    subscribe_args: dict[str, Any] = {"subject": config.subject_template, "cb": handler}
    if queue_group:
        subscribe_args["queue"] = queue_group

    subscription = await nc.subscribe(**subscribe_args)

    if max_messages > 0:
        await done.wait()
        await subscription.unsubscribe()
        await nc.drain()
        return

    print(
        f"Subscribed to {config.subject_template} on {config.server_address}. "
        "Press Ctrl+C to stop."
    )
    try:
        while True:
            await asyncio.sleep(3600)
    finally:
        await subscription.unsubscribe()
        await nc.drain()


def publish_main() -> None:
    parser = argparse.ArgumentParser(
        description="Publish filtered Discord order signals to a NATS subject."
    )
    parser.add_argument("input_file", help="Path to the filtered .orders.jsonl file.")
    parser.add_argument(
        "--env-file",
        default=".env",
        help="Path to the environment file. Default: .env",
    )
    parser.add_argument(
        "--server-address",
        help="Override NATS_SERVER_ADDRESS. Default: 127.0.0.1:4222",
    )
    parser.add_argument(
        "--subject-template",
        help=(
            "Override NATS_SUBJECT. Supports {category}, for example signals.options.{category}."
        ),
    )
    parser.add_argument(
        "--categories",
        default="entry",
        help="Comma-separated categories to publish. Default: entry",
    )
    parser.add_argument(
        "--limit",
        type=int,
        help="Publish at most N records after filtering.",
    )
    args = parser.parse_args()

    config = load_nats_config(
        env_file=args.env_file,
        server_address_override=args.server_address,
        subject_template_override=args.subject_template,
    )
    categories = parse_categories(args.categories)
    total = asyncio.run(
        publish_records(
            input_path=Path(args.input_file),
            config=config,
            categories=categories,
            limit=args.limit,
        )
    )
    print(f"Publish complete: {total} records")


def subscribe_main() -> None:
    parser = argparse.ArgumentParser(
        description="Subscribe to a NATS subject and print incoming Discord order signals."
    )
    parser.add_argument(
        "--env-file",
        default=".env",
        help="Path to the environment file. Default: .env",
    )
    parser.add_argument(
        "--server-address",
        help="Override NATS_SERVER_ADDRESS. Default: 127.0.0.1:4222",
    )
    parser.add_argument(
        "--subject",
        help="Override NATS_SUBJECT for subscription. Supports wildcards such as signals.options.>",
    )
    parser.add_argument(
        "--queue-group",
        default="",
        help="Optional queue group. Leave empty if every leaf node should receive every message.",
    )
    parser.add_argument(
        "--max-messages",
        type=int,
        default=0,
        help="Stop after receiving N messages. Default: 0 means run until interrupted.",
    )
    parser.add_argument(
        "--compact",
        action="store_true",
        help="Print compact JSON instead of pretty JSON.",
    )
    args = parser.parse_args()

    config = load_nats_config(
        env_file=args.env_file,
        server_address_override=args.server_address,
        subject_template_override=args.subject,
    )
    asyncio.run(
        subscribe_subject(
            config=config,
            queue_group=args.queue_group.strip(),
            max_messages=args.max_messages,
            pretty=not args.compact,
        )
    )


if __name__ == "__main__":
    publish_main()