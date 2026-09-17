#!/usr/bin/env python3
"""Verify committed live samples include real private order-stream evidence."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any


OPERATION = "private_ws_order_stream"
SOURCE = "private_ws_runtime"
ORDER_WRITE_OPERATION = "order_write"
MAX_FUTURE_SKEW_MS = 300_000
NON_LIVE_MARKER = re.compile(
    r"(^|[^a-z0-9])"
    r"(paper|testnet|test-net|sandbox|demo|mock|simulated|simulation|dry[_-]?run)"
    r"([^a-z0-9]|$)",
    re.IGNORECASE,
)
PLACEHOLDERS = {"", "***", "<redacted>", "[redacted]", "masked", "redacted"}
SENSITIVE_KEYS = {
    "apikey",
    "apisecret",
    "authorization",
    "bearer",
    "cookie",
    "passphrase",
    "privatekey",
    "secret",
    "signature",
    "token",
}


class GateError(Exception):
    pass


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Validate that a committed /api/system/venue-operation-health live "
            "sample passes the order_write verifier and also includes a clean "
            "private_ws_order_stream companion row."
        )
    )
    parser.add_argument("snapshot", nargs="?", help="Captured JSON snapshot")
    parser.add_argument("--venue", help="Venue to validate")
    parser.add_argument("--max-age-ms", type=int, default=120_000)
    parser.add_argument("--require-hashed-identity", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    try:
        if args.self_test:
            run_self_test()
            print("OK private order stream live sample verifier self-test")
            return 0
        if not args.snapshot or not args.venue:
            parser.error("snapshot and --venue are required unless --self-test is used")
        run_base_verifier(
            Path(args.snapshot),
            args.venue,
            args.max_age_ms,
            require_hashed_identity=args.require_hashed_identity,
        )
        snapshot = load_snapshot(Path(args.snapshot))
        require_private_order_stream_sample(snapshot, args.venue, args.max_age_ms)
        print(f"OK private order stream live sample venue={normalize(args.venue)}")
        return 0
    except GateError as exc:
        print(f"private order stream live sample failed: {exc}", file=sys.stderr)
        return 1


def run_base_verifier(
    snapshot: Path,
    venue: str,
    max_age_ms: int,
    *,
    require_hashed_identity: bool,
) -> None:
    script = Path(__file__).with_name("check_live_order_runtime_acceptance.py")
    command = [
        sys.executable,
        str(script),
        str(snapshot),
        "--venue",
        venue,
        "--max-age-ms",
        str(max_age_ms),
    ]
    if require_hashed_identity:
        command.append("--require-hashed-identity")
    result = subprocess.run(
        command,
        cwd=Path(__file__).resolve().parents[1],
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip().splitlines()
        suffix = f": {detail[0]}" if detail else ""
        raise GateError(f"base live order verifier failed{suffix}")


def load_snapshot(path: Path) -> dict[str, Any]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise GateError(f"invalid JSON: {exc}") from exc
    except OSError as exc:
        raise GateError(f"cannot read {path}: {exc}") from exc
    if not isinstance(data, dict):
        raise GateError("snapshot must be a JSON object")
    return data


def require_private_order_stream_sample(
    snapshot: dict[str, Any],
    venue: str,
    max_age_ms: int,
) -> None:
    if max_age_ms < 0:
        raise GateError("--max-age-ms must be non-negative")
    rows = snapshot.get("rows")
    if not isinstance(rows, list) or not all(isinstance(row, dict) for row in rows):
        raise GateError("snapshot.rows must be an array of objects")
    require_no_sensitive_or_non_live(snapshot)
    order_row = one_row(rows, venue, ORDER_WRITE_OPERATION)
    stream_row = one_row(rows, venue, OPERATION)
    generated_at_ms = require_int(snapshot, "generatedAtMs", "snapshot")
    order_observed_at_ms = require_int(order_row, "observedAtMs", "order_write row")
    row_observed_at_ms = require_int(stream_row, "observedAtMs", "private order stream row")
    freshness_ms = require_int(stream_row, "freshnessMs", "private order stream row")
    if row_observed_at_ms < order_observed_at_ms:
        raise GateError(
            "private order stream observedAtMs must not be earlier than order_write observedAtMs"
        )
    if generated_at_ms < row_observed_at_ms:
        raise GateError("snapshot.generatedAtMs must not be earlier than stream observedAtMs")
    if generated_at_ms - row_observed_at_ms != freshness_ms:
        raise GateError("private order stream freshnessMs must equal generatedAtMs - observedAtMs")
    if freshness_ms > max_age_ms:
        raise GateError(f"private order stream sample is stale: {freshness_ms} > {max_age_ms}")
    require_status_shape(stream_row)
    require_evidence(stream_row.get("evidence"))
    require_not_future("snapshot.generatedAtMs", generated_at_ms)
    require_not_future("private order stream observedAtMs", row_observed_at_ms)


def one_row(rows: list[dict[str, Any]], venue: str, operation: str) -> dict[str, Any]:
    venue_key = normalize(venue)
    matches = [
        row
        for row in rows
        if normalize(str(row.get("venue", ""))) == venue_key
        and row.get("operation") == operation
    ]
    if not matches:
        raise GateError(f"missing {venue_key}.{operation} row")
    if len(matches) > 1:
        raise GateError(f"expected one {venue_key}.{operation} row, got {len(matches)}")
    return matches[0]


def require_status_shape(row: dict[str, Any]) -> None:
    if row.get("status") != "ok":
        raise GateError("private order stream row must be status=ok")
    if row.get("source") != SOURCE:
        raise GateError(f"private order stream source must be {SOURCE}")
    if row.get("supported") is not True:
        raise GateError("private order stream row must carry supported=true")
    if row.get("configured") is not True:
        raise GateError("private order stream row must carry configured=true")
    rows = require_int(row, "rows", "private order stream row")
    if rows < 1:
        raise GateError("private order stream row must carry rows>=1")
    if "requested" in row:
        requested = require_int(row, "requested", "private order stream row")
        if requested < rows:
            raise GateError("private order stream requested must be >= rows")
    for key in ("problem", "error", "retryAfterMs"):
        if key in row:
            raise GateError(f"private order stream ok row must not carry {key}")
    message = row.get("message")
    if not isinstance(message, str) or not message.strip():
        raise GateError("private order stream row must carry a non-empty message")


def require_evidence(value: Any) -> None:
    if not isinstance(value, dict):
        raise GateError("private order stream row must carry evidence")
    for key in (
        "method",
        "path",
        "checkedAt",
        "docVersion",
        "parserTest",
        "requestBuilderTest",
        "authKind",
    ):
        require_non_placeholder_string(value, key, "private order stream evidence")
    if value["method"] != "WS":
        raise GateError("private order stream evidence.method must be WS")
    request_id = value.get("requestId")
    if request_id is not None:
        require_not_placeholder("private order stream evidence.requestId", request_id)
    context = require_string_array(value, "requestContext")
    context_map = context_values(context)
    if context_map.get("runtime_operation") != OPERATION:
        raise GateError("private order stream evidence must carry runtime_operation context")
    if not context_map.get("ws_operation"):
        raise GateError("private order stream evidence must carry ws_operation context")
    if context_map.get("schema_hash") != "not_recorded":
        raise GateError("private order stream schema_hash context must remain not_recorded")
    if context_map.get("fixture_id") != "not_recorded":
        raise GateError("private order stream fixture_id context must remain not_recorded")
    doc_urls = require_string_array(value, "docUrls")
    if not any(url.startswith("https://") for url in doc_urls):
        raise GateError("private order stream evidence must carry official HTTPS docs")
    use_cases = set(require_string_array(value, "useCases"))
    if not {"private_ws_runtime", "order_stream"}.issubset(use_cases):
        raise GateError("private order stream evidence.useCases must include private_ws_runtime/order_stream")
    data_kinds = set(require_string_array(value, "dataKinds"))
    if "order_state_stream" not in data_kinds:
        raise GateError("private order stream evidence.dataKinds must include order_state_stream")
    weight = value.get("weight")
    if isinstance(weight, bool) or not isinstance(weight, int) or weight < 0:
        raise GateError("private order stream evidence.weight must be a non-negative integer")


def require_string_array(container: dict[str, Any], key: str) -> list[str]:
    values = container.get(key)
    if not isinstance(values, list) or not all(isinstance(item, str) for item in values):
        raise GateError(f"{key} must be a string array")
    if not values:
        raise GateError(f"{key} must not be empty")
    for item in values:
        require_not_placeholder(key, item)
    return values


def context_values(items: list[str]) -> dict[str, str]:
    values: dict[str, str] = {}
    for item in items:
        key, separator, value = item.partition("=")
        if not separator or not key.strip() or not value.strip():
            raise GateError(f"requestContext item must be key=value, got {item!r}")
        if key in values:
            raise GateError(f"requestContext has duplicate key {key!r}")
        values[key.strip()] = value.strip()
    return values


def require_non_placeholder_string(container: dict[str, Any], key: str, label: str) -> None:
    value = container.get(key)
    if not isinstance(value, str) or not value.strip():
        raise GateError(f"{label}.{key} must be a non-empty string")
    require_not_placeholder(f"{label}.{key}", value)
    if key in {"checkedAt", "docVersion", "parserTest", "requestBuilderTest", "authKind"}:
        if value == "not_recorded":
            raise GateError(f"{label}.{key} must not be not_recorded")


def require_not_placeholder(label: str, value: str) -> None:
    normalized = value.strip().lower()
    if normalized in PLACEHOLDERS or set(normalized) <= {"*"}:
        raise GateError(f"{label} uses placeholder value")


def require_int(container: dict[str, Any], key: str, label: str) -> int:
    value = container.get(key)
    if isinstance(value, bool) or not isinstance(value, int):
        raise GateError(f"{label}.{key} must be an integer")
    if value < 0:
        raise GateError(f"{label}.{key} must be non-negative")
    return value


def require_not_future(label: str, value: int) -> None:
    if value > int(time.time() * 1000) + MAX_FUTURE_SKEW_MS:
        raise GateError(f"{label} is future-dated")


def require_no_sensitive_or_non_live(value: Any) -> None:
    findings: list[str] = []
    collect_findings(value, "$", findings)
    if findings:
        raise GateError("; ".join(findings[:5]))


def collect_findings(value: Any, path: str, findings: list[str]) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            normalized_key = "".join(char for char in str(key).lower() if char.isalnum())
            if normalized_key in SENSITIVE_KEYS:
                findings.append(f"sensitive key {path}.{key}")
            collect_findings(child, f"{path}.{key}", findings)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            collect_findings(child, f"{path}[{index}]", findings)
    elif isinstance(value, str):
        if NON_LIVE_MARKER.search(value):
            findings.append(f"non-live marker {path}={value!r}")


def normalize(venue: str) -> str:
    return venue.strip().lower()


def run_self_test() -> None:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "okx.json"
        write_fixture(path)
        run_base_verifier(path, "okx", 120_000, require_hashed_identity=True)
        snapshot = load_snapshot(path)
        require_private_order_stream_sample(snapshot, "okx", 120_000)
        assert_rejected("missing stream", path, lambda data: data["rows"].pop())
        assert_rejected(
            "warn stream",
            path,
            lambda data: data["rows"][1].update({"status": "warn", "problem": {"code": "x"}}),
        )
        assert_rejected("zero rows", path, lambda data: data["rows"][1].update({"rows": 0}))
        assert_rejected(
            "stale stream",
            path,
            lambda data: data["rows"][1].update({"freshnessMs": 130_000}),
        )
        assert_rejected(
            "missing ws operation",
            path,
            lambda data: data["rows"][1]["evidence"].update({"requestContext": [
                "runtime_operation=private_ws_order_stream",
                "schema_hash=not_recorded",
                "fixture_id=not_recorded",
            ]}),
        )
        assert_rejected(
            "not recorded parser",
            path,
            lambda data: data["rows"][1]["evidence"].update({"parserTest": "not_recorded"}),
        )
        assert_rejected(
            "non-live marker",
            path,
            lambda data: data["rows"][1].update({"message": "testnet order stream"}),
        )


def assert_rejected(label: str, path: Path, mutate: Any) -> None:
    data = fixture()
    mutate(data)
    path.write_text(json.dumps(data, ensure_ascii=False), encoding="utf-8")
    try:
        require_private_order_stream_sample(load_snapshot(path), "okx", 120_000)
    except GateError:
        return
    raise GateError(f"self-test accepted {label}")


def write_fixture(path: Path) -> None:
    path.write_text(json.dumps(fixture(), ensure_ascii=False), encoding="utf-8")


def fixture() -> dict[str, Any]:
    generated_at_ms = int(time.time() * 1000)
    stream_observed_at_ms = generated_at_ms - 500
    order_observed_at_ms = generated_at_ms - 800
    return {
        "rows": [
            order_write_row(order_observed_at_ms),
            private_stream_row(stream_observed_at_ms, generated_at_ms),
        ],
        "generatedAtMs": generated_at_ms,
        "rowCount": 2,
        "attentionCount": 0,
    }


def order_write_row(observed_at_ms: int) -> dict[str, Any]:
    return {
        "venue": "okx",
        "operation": "order_write",
        "status": "ok",
        "source": "live_order_proof_runtime",
        "message": "live 下单/撤单远程证明已闭环",
        "supported": True,
        "configured": True,
        "requested": 2,
        "rows": 2,
        "freshnessMs": 800,
        "evidence": {
            "method": "internal",
            "path": "live_order_proof.runtime",
            "checkedAt": "not_recorded",
            "docVersion": "not_recorded",
            "schemaHash": "not_recorded",
            "fixtureId": "not_recorded",
            "parserTest": "not_recorded",
            "requestBuilderTest": "not_recorded",
            "authKind": "live_order_remote_proof",
            "requestContext": [
                "probe_scope=okx.order_write.live_place_cancel",
                "probe_source=live_order_proof_runtime",
                "live_place_remote_proof=ok",
                "live_cancel_remote_proof=ok",
                "place_ack_count=1",
                "cancel_requested_count=1",
                "cancel_finality_count=1",
                "sample_place_internal_order_id_hash=hmac-sha256:162b8fa1015735430c68cef961bea09b41a2619d2512d5e399f972d533f9ad70",
                "sample_cancel_internal_order_id_hash=hmac-sha256:162b8fa1015735430c68cef961bea09b41a2619d2512d5e399f972d533f9ad70",
                "sample_place_symbol=BTCUSDT",
                "sample_cancel_symbol=BTCUSDT",
                "sample_place_source=adapter_ack",
                "sample_cancel_source=private_ws_order",
                f"sample_place_checked_at_ms={observed_at_ms - 100}",
                f"sample_cancel_checked_at_ms={observed_at_ms}",
            ],
            "docUrls": [],
            "useCases": ["order_write", "live_place_cancel_remote_proof"],
            "dataKinds": ["order_ack", "cancel_request_ack", "cancel_finality"],
            "rateScopes": [],
            "weight": 0,
        },
        "observedAtMs": observed_at_ms,
    }


def private_stream_row(observed_at_ms: int, generated_at_ms: int) -> dict[str, Any]:
    return {
        "venue": "okx",
        "operation": OPERATION,
        "status": "ok",
        "source": SOURCE,
        "message": "私有 WS 收到订单事件",
        "supported": True,
        "configured": True,
        "requested": 1,
        "rows": 1,
        "freshnessMs": generated_at_ms - observed_at_ms,
        "evidence": {
            "method": "WS",
            "path": "wss://ws.okx.com:8443/ws/v5/private#orders",
            "checkedAt": "2026-07-02",
            "docVersion": "okx-private-orders-ws-2026-07-02",
            "schemaHash": "not_recorded",
            "fixtureId": "not_recorded",
            "parserTest": "parses_order_channel_update",
            "requestBuilderTest": "subscribe_payloads_match_official_private_channels",
            "authKind": "login",
            "requestContext": [
                "runtime_operation=private_ws_order_stream",
                "ws_operation=orders",
                "support_status=ready",
                "capability_supported=true",
                "product=swap",
                "schema_hash=not_recorded",
                "fixture_id=not_recorded",
            ],
            "docUrls": ["https://www.okx.com/docs-v5/en/#order-book-trading-trade-ws-order-channel"],
            "useCases": ["private_ws_runtime", "order_stream"],
            "dataKinds": ["order_state_stream"],
            "rateScopes": [],
            "weight": 0,
        },
        "observedAtMs": observed_at_ms,
    }


if __name__ == "__main__":
    raise SystemExit(main())
