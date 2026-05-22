from __future__ import annotations

import argparse
import json
import re
from collections import Counter
from pathlib import Path
from typing import Any

SYMBOL_RE = re.compile(r"\$([A-Z]{1,6})\b")
PRICE_RE = re.compile(r"\$(\d+(?:\.\d+)?|\.\d+)")
OPTION_ENTRY_PATTERNS = [
    re.compile(
        r"\$(?P<symbol>[A-Z]{1,6})\s+\$(?P<strike>\d+(?:\.\d+)?)\s+"
        r"(?P<contract_type>calls?|puts?)"
        r"(?:\s+(?P<expiry_label>weekly|weeklies|0dte|1dte|daily|tomorrow|next week))?"
        r"(?:\s+for)?\s+\$(?P<price>\d+(?:\.\d+)?|\.\d+)",
        re.IGNORECASE,
    ),
    re.compile(
        r"\$(?P<symbol>[A-Z]{1,6})\s*-\s*\$(?P<strike>\d+(?:\.\d+)?)\s+"
        r"(?:(?P<expiry_label>0dte|1dte|weekly|weeklies|daily)\s+)?"
        r"(?P<label>lotto|starter|swing|scalp|size)?"
        r"(?:\s+size)?\s+\$(?P<price>\d+(?:\.\d+)?|\.\d+)",
        re.IGNORECASE,
    ),
]
ENTRY_KEYWORDS = (
    "try to fill",
    "better fill",
    "fill now",
    "lotto",
    "starter",
    "0dte",
    "1dte",
)
ADD_KEYWORDS = (
    " more",
    "more.",
    "more\n",
    "round two",
    "added",
    "re-add",
    "second opportunity",
)
EXIT_KEYWORDS = (
    "start trimming",
    "trimming",
    "trim into strength",
    "scale out",
    "scaling out",
    "sell into strength",
    "risk-free trade",
    "down to runners",
    "down to 2/3",
    "cutting",
)
UPDATE_KEYWORDS = (
    "holding",
    "runners position",
    "sizes into tomorrow",
    "here’s my plan",
    "here's my plan",
    "quick update",
    "weekly recap",
)
THRESHOLDS = {
    "entry": 5,
    "add": 3,
    "exit": 4,
    "update": 4,
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Filter exported Discord messages for likely order-related alerts."
    )
    parser.add_argument("input_file", help="Path to the exported Discord JSONL file.")
    parser.add_argument(
        "--output-file",
        help="Where to write filtered JSONL results. Default: <input>.orders.jsonl",
    )
    parser.add_argument(
        "--categories",
        default="entry,add,exit,update",
        help="Comma-separated categories to keep. Default: entry,add,exit,update",
    )
    parser.add_argument(
        "--min-score",
        type=int,
        default=0,
        help="Optional minimum score override after rule matching.",
    )
    return parser.parse_args()


def load_messages(path: Path) -> list[dict[str, Any]]:
    messages: list[dict[str, Any]] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        messages.append(json.loads(line))
    return messages


def collect_message_text(message: dict[str, Any]) -> str:
    pieces = [message.get("content", "")]
    for embed in message.get("embeds", []):
        if title := embed.get("title"):
            pieces.append(title)
        if description := embed.get("description"):
            pieces.append(description)
        for field in embed.get("fields", []):
            if name := field.get("name"):
                pieces.append(name)
            if value := field.get("value"):
                pieces.append(value)
    return "\n".join(piece for piece in pieces if piece)


def unique_symbols(text: str) -> list[str]:
    seen: set[str] = set()
    symbols: list[str] = []
    for symbol in SYMBOL_RE.findall(text):
        if symbol in seen:
            continue
        seen.add(symbol)
        symbols.append(symbol)
    return symbols


def normalize_price(value: str | None) -> str | None:
    if value is None:
        return None
    return value if value.startswith("0") or not value.startswith(".") else f"0{value}"


def extract_entry(text: str) -> dict[str, Any] | None:
    for pattern in OPTION_ENTRY_PATTERNS:
        match = pattern.search(text)
        if not match:
            continue
        parsed = {key: value for key, value in match.groupdict().items() if value}
        if "price" in parsed:
            parsed["price"] = normalize_price(parsed["price"])
        if contract_type := parsed.get("contract_type"):
            parsed["contract_type"] = contract_type.lower()
        if expiry_label := parsed.get("expiry_label"):
            parsed["expiry_label"] = expiry_label.lower()
        return parsed
    return None


def score_category(text: str, symbols: list[str]) -> tuple[str | None, int, list[str], dict[str, Any] | None]:
    lowered = f" {text.lower()} "
    scores = {"entry": 0, "add": 0, "exit": 0, "update": 0}
    reasons: list[str] = []
    parsed_entry = extract_entry(text)

    if parsed_entry:
        scores["entry"] += 6
        reasons.append("matched structured entry pattern")

    if symbols and any(keyword in lowered for keyword in ENTRY_KEYWORDS):
        scores["entry"] += 2
        reasons.append("entry wording present")

    if symbols and re.search(r"\b(calls?|puts?)\b", lowered):
        scores["entry"] += 2
        reasons.append("options contract wording present")

    if symbols and PRICE_RE.search(text) and (" fill" in lowered or " for $" in lowered):
        scores["entry"] += 1
        reasons.append("symbol with actionable price found")

    if symbols and any(keyword in lowered for keyword in ADD_KEYWORDS):
        scores["add"] += 4
        reasons.append("add-to-position wording present")

    if symbols and any(keyword in lowered for keyword in EXIT_KEYWORDS):
        scores["exit"] += 5
        reasons.append("exit/trim wording present")

    if any(keyword in lowered for keyword in UPDATE_KEYWORDS):
        scores["update"] += 4
        reasons.append("position update wording present")

    if symbols and re.search(r"\b\d+%\b", text) and ("holding" in lowered or "cutting" in lowered):
        scores["update"] += 2
        reasons.append("position sizing details present")

    best_category = max(scores, key=scores.get)
    best_score = scores[best_category]
    if best_score < THRESHOLDS[best_category]:
        return None, 0, reasons, parsed_entry
    return best_category, best_score, reasons, parsed_entry


def confidence_from_score(score: int) -> str:
    if score >= 7:
        return "high"
    if score >= 5:
        return "medium"
    return "low"


def filter_message(message: dict[str, Any], min_score: int) -> dict[str, Any] | None:
    text = collect_message_text(message)
    symbols = unique_symbols(text)
    category, score, reasons, parsed_entry = score_category(text, symbols)
    if category is None or score < min_score:
        return None

    return {
        "message_id": message.get("id"),
        "channel_id": message.get("channel_id"),
        "timestamp": message.get("timestamp"),
        "author_username": message.get("author_username"),
        "category": category,
        "confidence": confidence_from_score(score),
        "score": score,
        "symbols": symbols,
        "parsed_entry": parsed_entry,
        "reasons": reasons,
        "content": message.get("content", ""),
        "attachment_count": len(message.get("attachments", [])),
    }


def default_output_path(input_path: Path) -> Path:
    return input_path.with_name(f"{input_path.stem}.orders.jsonl")


def main() -> None:
    args = parse_args()
    input_path = Path(args.input_file)
    output_path = Path(args.output_file) if args.output_file else default_output_path(input_path)
    categories = {item.strip() for item in args.categories.split(",") if item.strip()}

    messages = load_messages(input_path)
    filtered: list[dict[str, Any]] = []

    for message in messages:
        record = filter_message(message, min_score=args.min_score)
        if record is None:
            continue
        if record["category"] not in categories:
            continue
        filtered.append(record)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    with output_path.open("w", encoding="utf-8") as handle:
        for record in filtered:
            handle.write(json.dumps(record, ensure_ascii=False) + "\n")

    counts = Counter(record["category"] for record in filtered)
    print(f"Scanned {len(messages)} messages")
    print(f"Matched {len(filtered)} order-like messages")
    for category in sorted(counts):
        print(f"- {category}: {counts[category]}")
    print(f"Wrote {output_path}")


if __name__ == "__main__":
    main()