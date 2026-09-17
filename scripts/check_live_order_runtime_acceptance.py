#!/usr/bin/env python3
"""Fail-closed verifier for captured live order_write runtime proof snapshots."""

from __future__ import annotations

import argparse
import json
import re
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


SOURCE = "live_order_proof_runtime"
OPERATION = "order_write"
EVIDENCE_METHOD = "internal"
EVIDENCE_PATH = "live_order_proof.runtime"
AUTH_KIND = "live_order_remote_proof"
UNRECORDED_EVIDENCE_MARKER = "not_recorded"
DEFAULT_MAX_AGE_MS = 120_000
REQUIRED_EVIDENCE_USE_CASES = {"order_write", "live_place_cancel_remote_proof"}
REQUIRED_EVIDENCE_DATA_KINDS = {
    "order_ack",
    "cancel_request_ack",
    "cancel_finality",
}
EVIDENCE_UNRECORDED_KEYS = (
    "checkedAt",
    "docVersion",
    "schemaHash",
    "fixtureId",
    "parserTest",
    "requestBuilderTest",
)
REQUIRED_EVIDENCE_KEYS = {
    "method",
    "path",
    *EVIDENCE_UNRECORDED_KEYS,
    "authKind",
    "requestContext",
    "docUrls",
    "useCases",
    "dataKinds",
    "rateScopes",
    "weight",
}
OPTIONAL_EVIDENCE_KEYS = {"requestId"}
SNAPSHOT_REQUIRED_KEYS = {
    "rows",
    "generatedAtMs",
    "rowCount",
    "attentionCount",
}
SNAPSHOT_OPTIONAL_KEYS = {"retryAfterMs"}
ACCEPTED_ORDER_WRITE_ROW_REQUIRED_KEYS = {
    "venue",
    "operation",
    "status",
    "source",
    "message",
    "supported",
    "configured",
    "requested",
    "rows",
    "freshnessMs",
    "evidence",
    "observedAtMs",
}
ACCEPTED_ORDER_WRITE_ROW_OPTIONAL_KEYS = {"latencyMs", "latencyP95Ms"}
COMMITTED_COMPANION_ROW_REQUIRED_KEYS = {
    "venue",
    "operation",
    "status",
    "source",
    "message",
    "freshnessMs",
    "evidence",
    "observedAtMs",
}
COMMITTED_COMPANION_ROW_OPTIONAL_KEYS = {
    "configured",
    "latencyMs",
    "latencyP95Ms",
    "requested",
    "rows",
    "supported",
}
HASH_ID_PATTERN = re.compile(r"^(?:(?:hmac-)?sha256:)?[0-9a-f]{64}$")
HASH_ID_WITH_ALGORITHM_PATTERN = re.compile(r"^(?:hmac-)?sha256:[0-9a-f]{64}$")
REDACTED_ID_VALUES = {
    "***",
    "<redacted>",
    "[redacted]",
    "masked",
    "redacted",
    "removed",
    "xxx",
}
INVALID_CONTEXT_PLACEHOLDER_VALUES = {
    "n/a",
    "na",
    "none",
    "null",
    "undefined",
}
NON_LIVE_MARKER_PATTERN = re.compile(
    r"(^|[^a-z0-9])"
    r"(paper|testnet|test-net|sandbox|demo|mock|simulated|simulation|dry[_-]?run)"
    r"([^a-z0-9]|$)",
    re.IGNORECASE,
)
COMMITTED_COMPANION_OPERATIONS = {
    "order_finality",
    "order_permission",
    "private_ws_order_stream",
}
HASH_IDENTITY_DIGEST = (
    "hmac-sha256:"
    "162b8fa1015735430c68cef961bea09b41a2619d2512d5e399f972d533f9ad70"
)
PUBLIC_SELF_TEST_HASH_DIGESTS = {
    "3b217ad2e4d6d7149d4640c53388e8946da8cbe8a404b78da2855faee46b447b",
    "2cd068eec50d1dca1fd2509ab24316e5245650bb29f363898668b423239755ee",
}
PUBLIC_SELF_TEST_ACCEPTED_MESSAGES = {"live proof accepted"}
PUBLIC_SELF_TEST_COMPANION_MESSAGES = {"captured finality row"}
MIN_COMMITTED_SAMPLE_EPOCH_MS = 1_735_689_600_000
MAX_COMMITTED_SAMPLE_FUTURE_SKEW_MS = 300_000
COMMITTED_SAMPLE_OBSERVED_AT_MS = 1_780_000_000_000
COMMITTED_SAMPLE_PLACE_CHECKED_AT_MS = COMMITTED_SAMPLE_OBSERVED_AT_MS - 100
SENSITIVE_KEY_NAMES = {
    "accesskey",
    "accesstoken",
    "apikey",
    "apisecret",
    "authorization",
    "bearer",
    "cookie",
    "kcapikey",
    "kcapipassphrase",
    "kcapisign",
    "okaccesskey",
    "okaccesspassphrase",
    "okaccesssign",
    "passphrase",
    "password",
    "privatekey",
    "refreshtoken",
    "secret",
    "secretkey",
    "sig",
    "signature",
    "token",
    "xapikey",
    "xbapiapikey",
    "xbapisign",
    "xmbxapikey",
}
SENSITIVE_VALUE_MARKERS = (
    "access-key:",
    "access-key=",
    "access_key=",
    "accesskey:",
    "accesskey=",
    "api-key:",
    "api-key=",
    "apikey=",
    "apikey:",
    "api_key=",
    "apisecret=",
    "api_secret=",
    "authorization:",
    "bearer ",
    "cookie:",
    "kc-api-key:",
    "kc-api-key=",
    "kc-api-passphrase:",
    "kc-api-passphrase=",
    "kc-api-sign:",
    "kc-api-sign=",
    "ok-access-key:",
    "ok-access-key=",
    "ok-access-passphrase:",
    "ok-access-passphrase=",
    "ok-access-sign:",
    "ok-access-sign=",
    "passphrase:",
    "passphrase=",
    "privatekey=",
    "private_key=",
    "secretkey=",
    "secret_key=",
    "signature=",
    "x-api-key:",
    "x-api-key=",
    "x-bapi-api-key:",
    "x-bapi-api-key=",
    "x-mbx-apikey:",
    "x-mbx-apikey=",
)
PLACE_SAMPLE_SOURCES = {"adapter_ack"}
CANCEL_SAMPLE_SOURCES = {
    "adapter_ack",
    "order_query",
    "private_ws_non_user_cancel",
    "private_ws_order",
}
COMMITTED_CANCEL_FINALITY_SOURCES = {
    "order_query",
    "private_ws_non_user_cancel",
    "private_ws_order",
}
RAW_IDENTITY_PAIRS = (
    ("sample_place_internal_order_id", "sample_cancel_internal_order_id"),
    ("sample_place_exchange_order_id", "sample_cancel_exchange_order_id"),
    ("sample_place_client_order_id", "sample_cancel_client_order_id"),
)
HASH_IDENTITY_PAIRS = (
    ("sample_place_internal_order_id_hash", "sample_cancel_internal_order_id_hash"),
    ("sample_place_exchange_order_id_hash", "sample_cancel_exchange_order_id_hash"),
    ("sample_place_client_order_id_hash", "sample_cancel_client_order_id_hash"),
)
REQUIRED_CONTEXT_KEYS = {
    "probe_scope",
    "probe_source",
    "live_place_remote_proof",
    "live_cancel_remote_proof",
    "place_ack_count",
    "cancel_requested_count",
    "cancel_finality_count",
    "sample_place_symbol",
    "sample_cancel_symbol",
    "sample_place_source",
    "sample_cancel_source",
    "sample_place_checked_at_ms",
    "sample_cancel_checked_at_ms",
}
OPTIONAL_SAMPLE_CONTEXT_KEYS = {
    "sample_place_request_id",
    "sample_cancel_request_id",
    "sample_place_native_transport",
    "sample_cancel_native_transport",
    "sample_place_native_request_id",
    "sample_cancel_native_request_id",
    "sample_place_native_response_id",
    "sample_cancel_native_response_id",
}
REQUEST_ID_CONTEXT_KEYS = ("sample_place_request_id", "sample_cancel_request_id")
NATIVE_TRANSPORT_CONTEXT_GROUPS = (
    (
        "place",
        "sample_place_native_transport",
        ("sample_place_native_request_id", "sample_place_native_response_id"),
    ),
    (
        "cancel",
        "sample_cancel_native_transport",
        ("sample_cancel_native_request_id", "sample_cancel_native_response_id"),
    ),
)


class AcceptanceError(Exception):
    pass


@dataclass(frozen=True)
class Acceptance:
    venue: str
    place_identity: str
    cancel_identity: str
    freshness_ms: int


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Validate a captured /api/system/venue-operation-health JSON snapshot "
            "contains redacted live order_write runtime proof."
        )
    )
    parser.add_argument(
        "snapshot",
        nargs="?",
        help="Path to captured JSON snapshot, or '-' for stdin.",
    )
    parser.add_argument("--venue", help="Venue whose order_write row must be accepted.")
    parser.add_argument(
        "--max-age-ms",
        type=int,
        default=DEFAULT_MAX_AGE_MS,
        help="Maximum allowed row freshnessMs for accepted runtime proof.",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="Run built-in accepted and rejected contract fixtures.",
    )
    parser.add_argument(
        "--require-hashed-identity",
        action="store_true",
        help=(
            "Require place/cancel identity proof to use stable sha256 or "
            "hmac-sha256 hashes instead of raw order ids."
        ),
    )
    args = parser.parse_args()

    if args.self_test:
        run_self_test()
        print("OK live order runtime proof verifier self-test")
        return 0

    if not args.snapshot or not args.venue:
        parser.error("snapshot and --venue are required unless --self-test is used")

    snapshot = load_snapshot(args.snapshot)
    require_snapshot_envelope(snapshot)
    require_redacted_snapshot(snapshot)
    acceptance = require_acceptance(
        snapshot,
        args.venue,
        args.max_age_ms,
        require_hashed_identity=args.require_hashed_identity,
    )
    print(
        "OK live order runtime proof snapshot "
        f"venue={acceptance.venue} "
        f"place={acceptance.place_identity} "
        f"cancel={acceptance.cancel_identity} "
        f"freshness_ms={acceptance.freshness_ms}"
    )
    return 0


def load_snapshot(path: str) -> dict[str, Any]:
    try:
        if path == "-":
            return json.load(sys.stdin)
        return json.loads(Path(path).read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise SystemExit(f"live order runtime proof failed: invalid JSON: {exc}") from exc
    except OSError as exc:
        raise SystemExit(f"live order runtime proof failed: cannot read {path}: {exc}") from exc


def require_snapshot_envelope(snapshot: Any) -> None:
    if not isinstance(snapshot, dict):
        raise AcceptanceError("snapshot must be a JSON object")
    require_key_set(
        snapshot,
        "snapshot",
        SNAPSHOT_REQUIRED_KEYS,
        SNAPSHOT_OPTIONAL_KEYS,
    )
    rows = snapshot.get("rows")
    if not isinstance(rows, list):
        raise AcceptanceError("snapshot.rows must be an array")
    if not all(isinstance(row, dict) for row in rows):
        raise AcceptanceError("snapshot.rows must contain only objects")
    require_count(snapshot, "rowCount", len(rows))
    attention_count = sum(1 for row in rows if row.get("status") != "ok")
    require_count(snapshot, "attentionCount", attention_count)
    if "retryAfterMs" in snapshot:
        require_non_negative_int(snapshot, "retryAfterMs")


def require_acceptance(
    snapshot: dict[str, Any],
    venue: str,
    max_age_ms: int,
    *,
    require_hashed_identity: bool = False,
) -> Acceptance:
    if max_age_ms < 0:
        raise AcceptanceError("--max-age-ms must be non-negative")
    rows = snapshot.get("rows")
    if not isinstance(rows, list):
        raise AcceptanceError("snapshot.rows must be an array")
    require_no_snapshot_non_live_markers(snapshot)
    if require_hashed_identity:
        require_committed_capture_shape(snapshot, rows, venue)

    row = find_order_write_row(rows, venue)
    require_key_set(
        row,
        "accepted order_write row",
        ACCEPTED_ORDER_WRITE_ROW_REQUIRED_KEYS,
        ACCEPTED_ORDER_WRITE_ROW_OPTIONAL_KEYS,
    )
    require_required_non_empty_string(row, "message", "accepted order_write row")
    for latency_key in ACCEPTED_ORDER_WRITE_ROW_OPTIONAL_KEYS:
        if latency_key in row:
            require_non_negative_int(row, latency_key)
    freshness_ms = require_freshness(row, max_age_ms)
    require_time_consistency(snapshot, row, freshness_ms, max_age_ms)
    require_equal(row, "status", "ok")
    require_equal(row, "source", SOURCE)
    require_bool_true(row, "supported")
    require_bool_true(row, "configured")
    require_count(row, "requested", 2)
    require_count(row, "rows", 2)
    require_absent(row, "problem")
    require_absent(row, "error")
    require_absent(row, "retryAfterMs")

    evidence = row.get("evidence")
    if not isinstance(evidence, dict):
        raise AcceptanceError("accepted order_write row must carry evidence")
    require_evidence_contract(evidence)

    context = evidence.get("requestContext")
    if not isinstance(context, list) or not all(isinstance(item, str) for item in context):
        raise AcceptanceError("evidence.requestContext must be a string array")
    context_map = context_values(context)
    require_request_context_allowlist(
        context_map,
        require_hashed_identity=require_hashed_identity,
    )
    require_request_id_metadata(evidence, context_map)
    require_evidence_request_id_matches_sample(evidence, context_map)

    require_context(context_map, "probe_source", SOURCE)
    require_probe_scope(context_map, venue)
    require_context(context_map, "live_place_remote_proof", "ok")
    require_context(context_map, "live_cancel_remote_proof", "ok")
    require_context_count_at_least(context_map, "place_ack_count", 1)
    require_context_non_negative_int(context_map, "cancel_requested_count")
    require_context_count_at_least(context_map, "cancel_finality_count", 1)
    require_native_transport_metadata(context_map)

    place_identity, cancel_identity = require_matching_identity(
        context_map,
        require_hashed_identity=require_hashed_identity,
    )
    require_context_one_of(context_map, "sample_place_source", PLACE_SAMPLE_SOURCES)
    cancel_source = require_context_one_of(
        context_map,
        "sample_cancel_source",
        CANCEL_SAMPLE_SOURCES,
    )
    if require_hashed_identity and cancel_source not in COMMITTED_CANCEL_FINALITY_SOURCES:
        expected = ", ".join(sorted(COMMITTED_CANCEL_FINALITY_SOURCES))
        raise AcceptanceError(
            "committed live sample cancel/finality source must be one of "
            f"{expected}, got {cancel_source!r}"
        )
    require_sample_time_consistency(row, context_map)
    if require_hashed_identity:
        require_committed_epoch_timestamps(snapshot, row, context_map)
        require_no_self_test_fixture_sentinels(snapshot, row, context_map)
    require_no_non_live_markers(row, context_map)

    return Acceptance(
        venue=row.get("venue", venue),
        place_identity=place_identity,
        cancel_identity=cancel_identity,
        freshness_ms=freshness_ms,
    )


def find_order_write_row(rows: list[Any], venue: str) -> dict[str, Any]:
    venue_key = normalize(venue)
    matches = [
        row
        for row in rows
        if isinstance(row, dict)
        and normalize(str(row.get("venue", ""))) == venue_key
        and row.get("operation") == OPERATION
    ]
    if not matches:
        raise AcceptanceError(f"missing {venue}.{OPERATION} row")
    if len(matches) > 1:
        raise AcceptanceError(f"expected one {venue}.{OPERATION} row, got {len(matches)}")
    return matches[0]


def require_committed_capture_shape(
    snapshot: dict[str, Any],
    rows: list[Any],
    venue: str,
) -> None:
    venue_key = normalize(venue)
    companion_operations = ", ".join(sorted(COMMITTED_COMPANION_OPERATIONS))
    generated_at_ms = require_ms(snapshot, "generatedAtMs")
    now_ms = current_unix_ms()
    companion_rows: list[tuple[dict[str, Any], str]] = []
    for row in rows:
        if not isinstance(row, dict):
            continue
        if normalize(str(row.get("venue", ""))) != venue_key:
            continue
        operation = row.get("operation")
        if isinstance(operation, str) and operation in COMMITTED_COMPANION_OPERATIONS:
            companion_rows.append((row, operation))
    if companion_rows:
        for row, operation in companion_rows:
            require_committed_companion_row(row, operation, generated_at_ms, now_ms)
        return
    raise AcceptanceError(
        "committed live sample must include a same-venue captured companion row "
        f"with operation one of {companion_operations}"
    )


def require_committed_companion_row(
    row: dict[str, Any],
    operation: str,
    generated_at_ms: int,
    now_ms: int,
) -> None:
    label = f"companion {operation} row"
    require_key_set(
        row,
        label,
        COMMITTED_COMPANION_ROW_REQUIRED_KEYS,
        COMMITTED_COMPANION_ROW_OPTIONAL_KEYS,
    )
    require_equal(row, "status", "ok")
    require_required_non_empty_string(row, "source", label)
    require_required_non_empty_string(row, "message", label)
    require_absent_from(row, "problem", label)
    require_absent_from(row, "error", label)
    require_absent_from(row, "retryAfterMs", label)
    observed_at_ms = require_ms(row, "observedAtMs")
    freshness_ms = require_ms(row, "freshnessMs")
    require_committed_timestamp_window(
        f"companion {operation}.observedAtMs",
        observed_at_ms,
        now_ms,
    )
    if observed_at_ms > generated_at_ms:
        raise AcceptanceError(
            f"companion {operation}.observedAtMs must not be later than "
            "snapshot.generatedAtMs"
        )
    computed_freshness_ms = generated_at_ms - observed_at_ms
    if freshness_ms != computed_freshness_ms:
        raise AcceptanceError(
            f"{label}.freshnessMs must equal generatedAtMs - observedAtMs, got "
            f"freshnessMs={freshness_ms} computed={computed_freshness_ms}"
        )
    evidence = row.get("evidence")
    if not isinstance(evidence, dict):
        raise AcceptanceError(f"{label}.evidence must be an object")
    require_committed_companion_evidence(evidence, operation)


def require_committed_companion_evidence(
    evidence: dict[str, Any],
    operation: str,
) -> None:
    label = f"companion {operation} evidence"
    require_evidence_key_set(evidence, label)
    require_required_non_empty_string(evidence, "method", label)
    require_required_non_empty_string(evidence, "path", label)
    require_optional_non_empty_string(evidence, "requestId", label)
    require_string_array(evidence, "requestContext", label, require_non_empty=True)
    require_string_array(evidence, "docUrls", label)
    require_string_array(evidence, "useCases", label, require_non_empty=True)
    require_string_array(evidence, "dataKinds", label, require_non_empty=True)
    require_string_array(evidence, "rateScopes", label)
    require_non_negative_int(evidence, "weight")


def require_freshness(row: dict[str, Any], max_age_ms: int) -> int:
    value = row.get("freshnessMs")
    if isinstance(value, bool) or not isinstance(value, int):
        raise AcceptanceError("accepted order_write row must carry numeric freshnessMs")
    if value < 0:
        raise AcceptanceError(f"accepted order_write proof has negative freshnessMs={value}")
    if value > max_age_ms:
        raise AcceptanceError(
            f"accepted order_write proof is stale: freshnessMs={value} max={max_age_ms}"
        )
    return value


def require_time_consistency(
    snapshot: dict[str, Any],
    row: dict[str, Any],
    freshness_ms: int,
    max_age_ms: int,
) -> None:
    generated_at_ms = require_ms(snapshot, "generatedAtMs")
    observed_at_ms = require_ms(row, "observedAtMs")
    if generated_at_ms < observed_at_ms:
        raise AcceptanceError(
            "snapshot generatedAtMs must be greater than or equal to row observedAtMs"
        )
    computed_freshness = generated_at_ms - observed_at_ms
    if computed_freshness != freshness_ms:
        raise AcceptanceError(
            "freshnessMs must equal generatedAtMs - observedAtMs, got "
            f"freshnessMs={freshness_ms} computed={computed_freshness}"
        )
    if computed_freshness > max_age_ms:
        raise AcceptanceError(
            f"accepted order_write proof is stale by timestamp: "
            f"freshnessMs={computed_freshness} max={max_age_ms}"
        )


def require_ms(container: dict[str, Any], key: str) -> int:
    value = container.get(key)
    if isinstance(value, bool) or not isinstance(value, int):
        raise AcceptanceError(f"{key} must be numeric milliseconds, got {value!r}")
    if value < 0:
        raise AcceptanceError(f"{key} must be non-negative, got {value}")
    return value


def require_count(container: dict[str, Any], key: str, expected: int) -> None:
    actual = container.get(key)
    if isinstance(actual, bool) or not isinstance(actual, int):
        raise AcceptanceError(f"{key} must be numeric {expected}, got {actual!r}")
    if actual != expected:
        raise AcceptanceError(f"{key} must be {expected}, got {actual!r}")


def require_non_negative_int(container: dict[str, Any], key: str) -> None:
    value = container.get(key)
    if isinstance(value, bool) or not isinstance(value, int):
        raise AcceptanceError(f"{key} must be numeric milliseconds, got {value!r}")
    if value < 0:
        raise AcceptanceError(f"{key} must be non-negative, got {value}")


def require_equal(container: dict[str, Any], key: str, expected: str) -> None:
    actual = container.get(key)
    if actual != expected:
        raise AcceptanceError(f"{key} must be {expected!r}, got {actual!r}")


def require_bool_true(container: dict[str, Any], key: str) -> None:
    actual = container.get(key)
    if not isinstance(actual, bool):
        raise AcceptanceError(f"{key} must be boolean true, got {actual!r}")
    if not actual:
        raise AcceptanceError(f"{key} must be true for accepted order_write proof")


def require_absent(container: dict[str, Any], key: str) -> None:
    require_absent_from(container, key, "accepted order_write row")


def require_absent_from(container: dict[str, Any], key: str, label: str) -> None:
    if key in container:
        raise AcceptanceError(f"{label} must not carry {key}")


def require_key_set(
    container: dict[str, Any],
    label: str,
    required: set[str],
    optional: set[str],
) -> None:
    present = set(container)
    missing = sorted(required.difference(present))
    unknown = sorted(present.difference(required, optional))
    if missing:
        raise AcceptanceError(f"{label} missing required keys: {', '.join(missing)}")
    if unknown:
        raise AcceptanceError(f"{label} contains non-acceptance keys: {', '.join(unknown)}")


def require_evidence_contract(evidence: dict[str, Any]) -> None:
    require_evidence_key_set(evidence)
    require_equal(evidence, "method", EVIDENCE_METHOD)
    require_equal(evidence, "path", EVIDENCE_PATH)
    for key in EVIDENCE_UNRECORDED_KEYS:
        require_equal(evidence, key, UNRECORDED_EVIDENCE_MARKER)
    require_equal(evidence, "authKind", AUTH_KIND)
    require_optional_non_empty_string(evidence, "requestId")
    require_string_array_exact(
        evidence,
        "useCases",
        REQUIRED_EVIDENCE_USE_CASES,
    )
    require_string_array_exact(
        evidence,
        "dataKinds",
        REQUIRED_EVIDENCE_DATA_KINDS,
    )
    require_empty_string_array(evidence, "docUrls")
    require_empty_string_array(evidence, "rateScopes")
    require_count(evidence, "weight", 0)


def require_evidence_key_set(
    evidence: dict[str, Any],
    label: str = "evidence",
) -> None:
    present = set(evidence)
    missing = sorted(REQUIRED_EVIDENCE_KEYS.difference(present))
    unknown = sorted(present.difference(REQUIRED_EVIDENCE_KEYS, OPTIONAL_EVIDENCE_KEYS))
    if missing:
        raise AcceptanceError(
            f"{label} missing required contract keys: " + ", ".join(missing)
        )
    if unknown:
        raise AcceptanceError(
            f"{label} contains non-runtime contract keys: " + ", ".join(unknown)
        )


def require_evidence_request_id_matches_sample(
    evidence: dict[str, Any],
    context: dict[str, str],
) -> None:
    request_id = evidence.get("requestId")
    if request_id is None:
        return
    sample_request_ids = {
        context[key].strip()
        for key in ("sample_place_request_id", "sample_cancel_request_id")
        if context.get(key, "").strip()
    }
    if request_id.strip() not in sample_request_ids:
        raise AcceptanceError(
            "evidence.requestId must match sample_place_request_id or "
            "sample_cancel_request_id when present"
        )


def require_request_id_metadata(
    evidence: dict[str, Any],
    context: dict[str, str],
) -> None:
    request_id = evidence.get("requestId")
    if request_id is not None:
        require_not_placeholder_context_value(
            "evidence.requestId",
            request_id,
            "request id",
        )
    for key in REQUEST_ID_CONTEXT_KEYS:
        value = context.get(key, "").strip()
        if value:
            require_not_placeholder_context_value(key, value, "request id")


def require_string_array_exact(
    container: dict[str, Any],
    key: str,
    expected: set[str],
) -> None:
    values = container.get(key)
    if not isinstance(values, list) or not all(isinstance(item, str) for item in values):
        raise AcceptanceError(f"evidence.{key} must be a string array")
    duplicates = sorted({item for item in values if values.count(item) > 1})
    if duplicates:
        raise AcceptanceError(
            f"evidence.{key} must not contain duplicate values: "
            + ", ".join(duplicates)
        )
    present = set(values)
    missing = sorted(expected.difference(present))
    extra = sorted(present.difference(expected))
    if missing:
        raise AcceptanceError(
            f"evidence.{key} missing required values: {', '.join(missing)}"
        )
    if extra:
        raise AcceptanceError(
            f"evidence.{key} contains non-runtime values: {', '.join(extra)}"
        )


def require_optional_non_empty_string(
    container: dict[str, Any],
    key: str,
    label: str = "evidence",
) -> None:
    if key not in container:
        return
    value = container.get(key)
    if not isinstance(value, str) or not value.strip():
        raise AcceptanceError(f"{label}.{key} must be a non-empty string when present")


def require_required_non_empty_string(
    container: dict[str, Any],
    key: str,
    label: str,
) -> str:
    value = container.get(key)
    if not isinstance(value, str) or not value.strip():
        raise AcceptanceError(f"{label}.{key} must be a non-empty string")
    stripped = value.strip()
    if stripped.lower() in INVALID_CONTEXT_PLACEHOLDER_VALUES:
        raise AcceptanceError(f"{label}.{key} must not use placeholder value {stripped!r}")
    require_not_placeholder_context_value(f"{label}.{key}", stripped, key)
    return stripped


def require_string_array(
    container: dict[str, Any],
    key: str,
    label: str,
    require_non_empty: bool = False,
) -> list[str]:
    values = container.get(key)
    if not isinstance(values, list) or not all(isinstance(item, str) for item in values):
        raise AcceptanceError(f"{label}.{key} must be a string array")
    if require_non_empty and not values:
        raise AcceptanceError(f"{label}.{key} must not be empty")
    for item in values:
        if not item.strip():
            raise AcceptanceError(f"{label}.{key} must not contain blank values")
        require_not_placeholder_context_value(f"{label}.{key}", item.strip(), key)
    return values


def require_empty_string_array(container: dict[str, Any], key: str) -> None:
    values = container.get(key)
    if not isinstance(values, list) or not all(isinstance(item, str) for item in values):
        raise AcceptanceError(f"evidence.{key} must be a string array")
    if values:
        raise AcceptanceError(f"evidence.{key} must be empty for internal live proof")


def require_context(context: dict[str, str], key: str, expected: str) -> None:
    actual = context.get(key)
    if actual != expected:
        raise AcceptanceError(f"context {key} must be {expected!r}, got {actual!r}")


def require_context_key(context: dict[str, str], key: str) -> str:
    value = context.get(key, "").strip()
    if not value:
        raise AcceptanceError(f"context {key} is required")
    return value


def require_context_one_of(
    context: dict[str, str],
    key: str,
    allowed: set[str],
) -> str:
    value = require_context_key(context, key)
    if value not in allowed:
        expected = ", ".join(sorted(allowed))
        raise AcceptanceError(f"context {key} must be one of {expected}, got {value!r}")
    return value


def require_context_count_at_least(
    context: dict[str, str],
    key: str,
    minimum: int,
) -> int:
    value = require_context_non_negative_int(context, key)
    if value < minimum:
        raise AcceptanceError(f"context {key} must be at least {minimum}, got {value}")
    return value


def require_context_non_negative_int(context: dict[str, str], key: str) -> int:
    value = require_context_key(context, key)
    if not re.fullmatch(r"\d+", value):
        raise AcceptanceError(f"context {key} must be a non-negative integer")
    return int(value)


def require_probe_scope(context: dict[str, str], venue: str) -> None:
    expected = f"{normalize(venue)}.{OPERATION}.live_place_cancel"
    actual = context.get("probe_scope", "").strip().lower()
    if actual != expected:
        raise AcceptanceError(
            f"context probe_scope must be {expected!r}, got {actual!r}"
        )


def require_matching_identity(
    context: dict[str, str],
    *,
    require_hashed_identity: bool = False,
) -> tuple[str, str]:
    place_symbol = require_context_key(context, "sample_place_symbol")
    cancel_symbol = require_context_key(context, "sample_cancel_symbol")
    require_not_placeholder_context_value(
        "sample_place_symbol",
        place_symbol,
        "sample symbol",
    )
    require_not_placeholder_context_value(
        "sample_cancel_symbol",
        cancel_symbol,
        "sample symbol",
    )
    if place_symbol.upper() != cancel_symbol.upper():
        raise AcceptanceError(
            "place/cancel proof symbols differ: "
            f"{place_symbol!r} != {cancel_symbol!r}"
        )

    accepted_identity: tuple[str, str] | None = None
    if not require_hashed_identity:
        for place_key, cancel_key in RAW_IDENTITY_PAIRS:
            place = context.get(place_key, "").strip()
            cancel = context.get(cancel_key, "").strip()
            require_identity_pair_completeness(place_key, cancel_key, place, cancel)
            if not place or not cancel:
                continue
            require_not_redacted_identity(place_key, place)
            require_not_redacted_identity(cancel_key, cancel)
            if place != cancel:
                raise AcceptanceError(
                    f"context {place_key}/{cancel_key} must match, got "
                    f"{place!r} != {cancel!r}"
                )
            if accepted_identity is None:
                accepted_identity = (place, cancel)
    else:
        for place_key, cancel_key in RAW_IDENTITY_PAIRS:
            for key in (place_key, cancel_key):
                if context.get(key, "").strip():
                    raise AcceptanceError(
                        f"context {key} must not be present when hashed identity "
                        "is required"
                    )

    for place_key, cancel_key in HASH_IDENTITY_PAIRS:
        place = context.get(place_key, "").strip().lower()
        cancel = context.get(cancel_key, "").strip().lower()
        require_identity_pair_completeness(place_key, cancel_key, place, cancel)
        if not place or not cancel:
            continue
        require_supported_hash_identity(
            place_key,
            place,
            require_algorithm_prefix=require_hashed_identity,
        )
        require_supported_hash_identity(
            cancel_key,
            cancel,
            require_algorithm_prefix=require_hashed_identity,
        )
        if place != cancel:
            raise AcceptanceError(
                f"context {place_key}/{cancel_key} must match, got "
                f"{place!r} != {cancel!r}"
            )
        if accepted_identity is None:
            accepted_identity = (place, cancel)

    if accepted_identity is not None:
        return accepted_identity

    if require_hashed_identity:
        raise AcceptanceError(
            "committed live sample proof must share a stable hashed place/cancel "
            "order identity"
        )
    raise AcceptanceError(
        "place/cancel proof must share a non-empty raw or stable hashed order identity"
    )


def require_identity_pair_completeness(
    place_key: str,
    cancel_key: str,
    place: str,
    cancel: str,
) -> None:
    if bool(place) != bool(cancel):
        raise AcceptanceError(
            f"context {place_key}/{cancel_key} must both be present or both absent"
        )


def require_supported_hash_identity(
    key: str,
    value: str,
    *,
    require_algorithm_prefix: bool = False,
) -> None:
    if require_algorithm_prefix and not HASH_ID_WITH_ALGORITHM_PATTERN.fullmatch(value):
        raise AcceptanceError(
            f"context {key} must include a sha256: or hmac-sha256: prefix, "
            f"got {value!r}"
        )
    if not HASH_ID_PATTERN.fullmatch(value):
        raise AcceptanceError(
            f"context {key} must be hmac-sha256:<64 hex>, sha256:<64 hex>, "
            f"or 64 hex, got {value!r}"
        )
    digest = value.rsplit(":", 1)[-1]
    if len(set(digest)) <= 2:
        raise AcceptanceError(
            f"context {key} uses a low-entropy placeholder hash, not a stable "
            "order identity proof"
        )


def require_sample_time_consistency(
    row: dict[str, Any],
    context: dict[str, str],
) -> None:
    observed_at_ms = require_ms(row, "observedAtMs")
    place_checked_at_ms = require_context_ms(context, "sample_place_checked_at_ms")
    cancel_checked_at_ms = require_context_ms(context, "sample_cancel_checked_at_ms")
    if cancel_checked_at_ms < place_checked_at_ms:
        raise AcceptanceError(
            "sample_cancel_checked_at_ms must be greater than or equal to "
            "sample_place_checked_at_ms"
        )
    expected_observed_at_ms = max(place_checked_at_ms, cancel_checked_at_ms)
    if observed_at_ms != expected_observed_at_ms:
        raise AcceptanceError(
            "row observedAtMs must equal max(sample_place_checked_at_ms, "
            "sample_cancel_checked_at_ms), got "
            f"observedAtMs={observed_at_ms} expected={expected_observed_at_ms}"
        )


def require_committed_epoch_timestamps(
    snapshot: dict[str, Any],
    row: dict[str, Any],
    context: dict[str, str],
) -> None:
    generated_at_ms = require_ms(snapshot, "generatedAtMs")
    observed_at_ms = require_ms(row, "observedAtMs")
    place_checked_at_ms = require_context_ms(context, "sample_place_checked_at_ms")
    cancel_checked_at_ms = require_context_ms(context, "sample_cancel_checked_at_ms")
    now_ms = current_unix_ms()
    require_committed_timestamp_window("snapshot.generatedAtMs", generated_at_ms, now_ms)
    require_committed_timestamp_window("row.observedAtMs", observed_at_ms, now_ms)
    require_committed_timestamp_window(
        "context sample_place_checked_at_ms",
        place_checked_at_ms,
        now_ms,
    )
    require_committed_timestamp_window(
        "context sample_cancel_checked_at_ms",
        cancel_checked_at_ms,
        now_ms,
    )


def require_no_self_test_fixture_sentinels(
    snapshot: dict[str, Any],
    row: dict[str, Any],
    context: dict[str, str],
) -> None:
    findings: list[str] = []
    for place_key, cancel_key in HASH_IDENTITY_PAIRS:
        for key in (place_key, cancel_key):
            value = context.get(key, "").strip().lower()
            if not value:
                continue
            digest = value.rsplit(":", 1)[-1]
            if digest in PUBLIC_SELF_TEST_HASH_DIGESTS:
                findings.append(f"context {key} uses public self-test hash")
    message = row.get("message")
    if (
        isinstance(message, str)
        and message.strip().lower() in PUBLIC_SELF_TEST_ACCEPTED_MESSAGES
    ):
        findings.append("accepted row message uses public self-test text")
    accepted_venue = normalize(str(row.get("venue", "")))
    for companion in snapshot.get("rows", []):
        if not isinstance(companion, dict):
            continue
        if normalize(str(companion.get("venue", ""))) != accepted_venue:
            continue
        if companion.get("operation") not in COMMITTED_COMPANION_OPERATIONS:
            continue
        companion_message = companion.get("message")
        if (
            isinstance(companion_message, str)
            and companion_message.strip().lower() in PUBLIC_SELF_TEST_COMPANION_MESSAGES
        ):
            operation = companion.get("operation")
            findings.append(
                f"companion {operation} message uses public self-test text"
            )
    if findings:
        raise AcceptanceError(
            "committed live sample reuses public verifier self-test fixture sentinels: "
            + "; ".join(findings[:5])
        )


def current_unix_ms() -> int:
    return int(time.time() * 1000)


def require_committed_timestamp_window(label: str, value: int, now_ms: int) -> None:
    require_epoch_ms(label, value)
    max_allowed_ms = now_ms + MAX_COMMITTED_SAMPLE_FUTURE_SKEW_MS
    if value > max_allowed_ms:
        raise AcceptanceError(
            f"{label}={value} is later than verifier time plus allowed skew "
            f"for committed live sample evidence"
        )


def require_epoch_ms(label: str, value: int) -> None:
    if value < MIN_COMMITTED_SAMPLE_EPOCH_MS:
        raise AcceptanceError(
            f"{label}={value} is not a plausible Unix epoch millisecond timestamp "
            "for committed live sample evidence"
        )


def require_context_ms(context: dict[str, str], key: str) -> int:
    value = require_context_key(context, key)
    if not re.fullmatch(r"\d+", value):
        raise AcceptanceError(f"context {key} must be non-negative milliseconds")
    return int(value)


def require_not_redacted_identity(key: str, value: str) -> None:
    require_not_placeholder_context_value(key, value, "identity proof")


def require_not_placeholder_context_value(key: str, value: str, purpose: str) -> None:
    normalized = value.strip().lower()
    if normalized in REDACTED_ID_VALUES or set(normalized) <= {"*"}:
        raise AcceptanceError(
            f"context {key} uses a generic redaction placeholder, not a {purpose}"
        )


def require_native_transport_metadata(context: dict[str, str]) -> None:
    for side, transport_key, id_keys in NATIVE_TRANSPORT_CONTEXT_GROUPS:
        transport = context.get(transport_key, "").strip()
        if transport:
            require_not_placeholder_context_value(
                transport_key,
                transport,
                f"{side} native transport",
            )
        for id_key in id_keys:
            value = context.get(id_key, "").strip()
            if not value:
                continue
            require_not_placeholder_context_value(
                id_key,
                value,
                f"{side} native transport id",
            )
            if not transport:
                raise AcceptanceError(
                    f"context {id_key} requires {transport_key} when native "
                    "transport metadata is present"
                )


def require_no_non_live_markers(
    row: dict[str, Any],
    context: dict[str, str],
) -> None:
    candidates: list[tuple[str, str]] = []
    for key in ("venue", "message"):
        value = row.get(key)
        if isinstance(value, str):
            candidates.append((f"row.{key}", value))
    candidates.extend((f"context {key}", value) for key, value in context.items())
    for label, value in candidates:
        if NON_LIVE_MARKER_PATTERN.search(value):
            raise AcceptanceError(
                f"{label} contains non-live sample marker {value!r}; "
                "committed live sample acceptance requires real live evidence"
            )


def require_no_snapshot_non_live_markers(value: Any) -> None:
    findings: list[str] = []
    collect_non_live_marker_findings(value, "$", findings)
    if findings:
        raise AcceptanceError(
            "snapshot contains non-live sample marker: " + "; ".join(findings[:5])
        )


def collect_non_live_marker_findings(value: Any, path: str, findings: list[str]) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            collect_non_live_marker_findings(child, f"{path}.{key}", findings)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            collect_non_live_marker_findings(child, f"{path}[{index}]", findings)
    elif isinstance(value, str) and NON_LIVE_MARKER_PATTERN.search(value):
        findings.append(f"{path}={value!r}")


def context_values(context: list[str]) -> dict[str, str]:
    values: dict[str, str] = {}
    for item in context:
        key, separator, value = item.partition("=")
        key = key.strip()
        if not separator or not key:
            raise AcceptanceError(
                f"evidence.requestContext item must be key=value, got {item!r}"
            )
        if not value.strip():
            raise AcceptanceError(
                f"evidence.requestContext key {key!r} must have a non-empty value"
            )
        if value.strip().lower() in INVALID_CONTEXT_PLACEHOLDER_VALUES:
            raise AcceptanceError(
                f"evidence.requestContext key {key!r} must not use placeholder value "
                f"{value.strip()!r}"
            )
        if key in values:
            raise AcceptanceError(f"evidence.requestContext has duplicate key {key!r}")
        values[key] = value.strip()
    return values


def require_request_context_allowlist(
    context: dict[str, str],
    *,
    require_hashed_identity: bool,
) -> None:
    allowed = set(REQUIRED_CONTEXT_KEYS)
    allowed.update(OPTIONAL_SAMPLE_CONTEXT_KEYS)
    allowed.update(key for pair in HASH_IDENTITY_PAIRS for key in pair)
    if not require_hashed_identity:
        allowed.update(key for pair in RAW_IDENTITY_PAIRS for key in pair)

    unknown = sorted(set(context).difference(allowed))
    if unknown:
        raise AcceptanceError(
            "evidence.requestContext contains non-acceptance keys: "
            + ", ".join(unknown)
        )


def normalize(venue: str) -> str:
    return venue.strip().lower()


def require_redacted_snapshot(value: Any) -> None:
    findings: list[str] = []
    collect_sensitive_findings(value, "$", findings)
    if findings:
        raise AcceptanceError(
            "snapshot contains sensitive material: " + "; ".join(findings[:5])
        )


def collect_sensitive_findings(value: Any, path: str, findings: list[str]) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            key_text = str(key)
            child_path = f"{path}.{key_text}"
            if normalize_key(key_text) in SENSITIVE_KEY_NAMES:
                findings.append(f"sensitive key {child_path}")
            collect_sensitive_findings(child, child_path, findings)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            collect_sensitive_findings(child, f"{path}[{index}]", findings)
    elif isinstance(value, str):
        lower = value.lower()
        if any(marker in lower for marker in SENSITIVE_VALUE_MARKERS):
            findings.append(f"sensitive value at {path}")


def normalize_key(key: str) -> str:
    return "".join(char for char in key.lower() if char.isalnum())


def run_self_test() -> None:
    accepted = accepted_fixture()
    require_snapshot_envelope(accepted)
    require_redacted_snapshot(accepted)
    require_acceptance(accepted, "okx", DEFAULT_MAX_AGE_MS)
    hash_accepted = hashed_identity_fixture()
    require_snapshot_envelope(hash_accepted)
    require_redacted_snapshot(hash_accepted)
    require_acceptance(
        hash_accepted,
        "okx",
        DEFAULT_MAX_AGE_MS,
        require_hashed_identity=True,
    )
    request_id_accepted = matching_evidence_request_id_fixture()
    require_snapshot_envelope(request_id_accepted)
    require_redacted_snapshot(request_id_accepted)
    require_acceptance(
        request_id_accepted,
        "okx",
        DEFAULT_MAX_AGE_MS,
        require_hashed_identity=True,
    )
    latency_accepted = accepted_row_latency_fixture()
    require_snapshot_envelope(latency_accepted)
    require_redacted_snapshot(latency_accepted)
    require_acceptance(latency_accepted, "okx", DEFAULT_MAX_AGE_MS)
    native_metadata_accepted = native_transport_metadata_fixture()
    require_snapshot_envelope(native_metadata_accepted)
    require_redacted_snapshot(native_metadata_accepted)
    require_acceptance(
        native_metadata_accepted,
        "okx",
        DEFAULT_MAX_AGE_MS,
        require_hashed_identity=True,
    )
    try:
        require_acceptance(
            accepted,
            "okx",
            DEFAULT_MAX_AGE_MS,
            require_hashed_identity=True,
        )
    except AcceptanceError:
        pass
    else:
        raise AcceptanceError("raw identity accepted when hashed identity is required")
    try:
        hash_plus_raw_identity = hash_plus_raw_identity_fixture()
        require_snapshot_envelope(hash_plus_raw_identity)
        require_redacted_snapshot(hash_plus_raw_identity)
        require_acceptance(
            hash_plus_raw_identity,
            "okx",
            DEFAULT_MAX_AGE_MS,
            require_hashed_identity=True,
        )
    except AcceptanceError:
        pass
    else:
        raise AcceptanceError("mixed raw+hashed identity accepted for committed sample")
    committed_rejected = [
        (
            "one-row committed fixture",
            committed_one_row_fixture(),
            "okx",
        ),
        (
            "low epoch committed timestamp",
            committed_low_epoch_time_fixture(),
            "okx",
        ),
        (
            "future committed timestamp",
            committed_future_time_fixture(),
            "okx",
        ),
        (
            "companion after generatedAtMs",
            committed_companion_after_generated_fixture(),
            "okx",
        ),
        (
            "minimal committed companion row",
            committed_minimal_companion_fixture(),
            "okx",
        ),
        (
            "committed companion freshness mismatch",
            committed_companion_freshness_mismatch_fixture(),
            "okx",
        ),
        (
            "committed companion empty evidence context",
            committed_companion_empty_evidence_context_fixture(),
            "okx",
        ),
        (
            "committed second companion extra key",
            committed_second_companion_extra_key_fixture(),
            "okx",
        ),
        (
            "adapter ack cancel finality source",
            committed_adapter_ack_cancel_finality_fixture(),
            "okx",
        ),
        (
            "bare committed hash identity",
            committed_bare_hash_identity_fixture(),
            "okx",
        ),
        (
            "public self-test fixture sentinel reuse",
            committed_public_self_test_fixture_reuse_fixture(),
            "okx",
        ),
    ]
    for label, fixture, venue in committed_rejected:
        try:
            require_snapshot_envelope(fixture)
            require_redacted_snapshot(fixture)
            require_acceptance(
                fixture,
                venue,
                DEFAULT_MAX_AGE_MS,
                require_hashed_identity=True,
            )
        except AcceptanceError:
            continue
        raise AcceptanceError(f"committed self-test fixture unexpectedly accepted: {label}")
    rejected = [
        ("non-object snapshot", [], "okx"),
        ("non-object rows", rows_with_non_object_fixture(), "okx"),
        ("row count mismatch", row_count_mismatch_fixture(), "okx"),
        ("attention count mismatch", attention_count_mismatch_fixture(), "okx"),
        ("snapshot extra key", snapshot_extra_key_fixture(), "okx"),
        ("invalid retryAfterMs", invalid_retry_after_fixture(), "okx"),
        ("missing row", {"rows": [], "rowCount": 0, "attentionCount": 0}, "okx"),
        ("warn row", warn_fixture(), "okx"),
        ("unsupported static gate", unsupported_static_gate_fixture(), "okx"),
        ("unconfigured static gate", unconfigured_static_gate_fixture(), "okx"),
        ("missing static gate", missing_static_gate_fixture(), "okx"),
        ("accepted row extra key", accepted_row_extra_key_fixture(), "okx"),
        ("accepted row null problem", accepted_row_null_problem_fixture(), "okx"),
        ("accepted row null error", accepted_row_null_error_fixture(), "okx"),
        ("accepted row null retryAfterMs", accepted_row_null_retry_after_fixture(), "okx"),
        ("accepted row null latency", accepted_row_null_latency_fixture(), "okx"),
        ("accepted row error", accepted_row_error_fixture(), "okx"),
        ("accepted row retryAfterMs", accepted_row_retry_after_fixture(), "okx"),
        ("mismatched identity", mismatched_identity_fixture(), "okx"),
        ("missing sample symbol", missing_sample_symbol_fixture(), "okx"),
        ("mismatched sample symbol", mismatched_sample_symbol_fixture(), "okx"),
        ("redacted sample symbol", redacted_sample_symbol_fixture(), "okx"),
        ("blank accepted row message", blank_row_message_fixture(), "okx"),
        ("placeholder accepted row message", placeholder_row_message_fixture(), "okx"),
        ("redacted accepted row message", redacted_row_message_fixture(), "okx"),
        ("redacted raw identity", redacted_raw_identity_fixture(), "okx"),
        ("mismatched hash identity", mismatched_hash_identity_fixture(), "okx"),
        ("conflicting extra raw identity", conflicting_extra_raw_identity_fixture(), "okx"),
        ("conflicting extra hash identity", conflicting_extra_hash_identity_fixture(), "okx"),
        ("one-sided raw identity", one_sided_raw_identity_fixture(), "okx"),
        ("one-sided hash identity", one_sided_hash_identity_fixture(), "okx"),
        ("unknown context key", unknown_context_key_fixture(), "okx"),
        ("invalid hash identity", invalid_hash_identity_fixture(), "okx"),
        ("low entropy hash identity", low_entropy_hash_identity_fixture(), "okx"),
        ("duplicate context key", duplicate_context_key_fixture(), "okx"),
        ("malformed context item", malformed_context_item_fixture(), "okx"),
        ("blank context value", blank_context_value_fixture(), "okx"),
        ("null context placeholder", null_context_placeholder_fixture(), "okx"),
        ("unmatched evidence requestId", unmatched_evidence_request_id_fixture(), "okx"),
        ("redacted sample request id", redacted_sample_request_id_fixture(), "okx"),
        ("redacted evidence requestId", redacted_evidence_request_id_fixture(), "okx"),
        ("asterisk sample request id", asterisk_sample_request_id_fixture(), "okx"),
        ("non-live row message", non_live_row_message_fixture(), "okx"),
        ("non-live context marker", non_live_context_marker_fixture(), "okx"),
        ("non-live companion row", non_live_companion_row_fixture(), "okx"),
        ("non-live venue", non_live_venue_fixture(), "okx-testnet"),
        ("missing generatedAtMs", missing_generated_at_fixture(), "okx"),
        ("missing observedAtMs", missing_observed_at_fixture(), "okx"),
        ("inconsistent freshness timestamp", inconsistent_time_fixture(), "okx"),
        ("future observed timestamp", future_observed_fixture(), "okx"),
        ("missing sample checked_at", missing_sample_checked_at_fixture(), "okx"),
        ("invalid sample checked_at", invalid_sample_checked_at_fixture(), "okx"),
        ("cancel before place", cancel_before_place_fixture(), "okx"),
        ("sample observedAt mismatch", sample_observed_at_mismatch_fixture(), "okx"),
        ("missing evidence method", missing_evidence_method_fixture(), "okx"),
        ("missing evidence metadata", missing_evidence_metadata_fixture(), "okx"),
        ("extra evidence key", extra_evidence_key_fixture(), "okx"),
        ("missing evidence use case", missing_evidence_use_case_fixture(), "okx"),
        ("extra evidence use case", extra_evidence_use_case_fixture(), "okx"),
        ("missing evidence data kind", missing_evidence_data_kind_fixture(), "okx"),
        ("extra evidence data kind", extra_evidence_data_kind_fixture(), "okx"),
        ("non-empty internal doc urls", non_empty_doc_urls_fixture(), "okx"),
        ("invalid evidence weight", invalid_evidence_weight_fixture(), "okx"),
        ("missing proof count", missing_proof_count_fixture(), "okx"),
        ("zero place proof count", zero_place_proof_count_fixture(), "okx"),
        ("zero cancel finality count", zero_cancel_finality_count_fixture(), "okx"),
        ("invalid proof count", invalid_proof_count_fixture(), "okx"),
        ("native request id without transport", native_request_without_transport_fixture(), "okx"),
        ("redacted native transport", redacted_native_transport_fixture(), "okx"),
        ("redacted native response id", redacted_native_response_id_fixture(), "okx"),
        ("mismatched probe scope", mismatched_probe_scope_fixture(), "okx"),
        ("sensitive key", sensitive_key_fixture(), "okx"),
        ("exchange header sensitive key", exchange_header_sensitive_key_fixture(), "okx"),
        ("sensitive value", sensitive_value_fixture(), "okx"),
        ("exchange header sensitive value", exchange_header_sensitive_value_fixture(), "okx"),
        ("negative freshness", accepted_fixture(freshness_ms=-1), "okx"),
        ("bool freshness", bool_freshness_fixture(), "okx"),
        ("stale row", accepted_fixture(freshness_ms=DEFAULT_MAX_AGE_MS + 1), "okx"),
        ("incomplete proof counts", incomplete_counts_fixture(), "okx"),
        ("bool proof counts", bool_counts_fixture(), "okx"),
        ("invalid place source", invalid_place_source_fixture(), "okx"),
        ("invalid cancel source", invalid_cancel_source_fixture(), "okx"),
    ]
    for label, fixture, venue in rejected:
        try:
            require_snapshot_envelope(fixture)
            require_redacted_snapshot(fixture)
            require_acceptance(fixture, venue, DEFAULT_MAX_AGE_MS)
        except AcceptanceError:
            continue
        raise AcceptanceError(f"self-test fixture unexpectedly accepted: {label}")


def accepted_fixture(
    freshness_ms: int = 800,
    observed_at_ms: int = 1000,
) -> dict[str, Any]:
    generated_at_ms = observed_at_ms + freshness_ms
    place_checked_at_ms = observed_at_ms - 100
    return {
        "rows": [
            {
                "venue": "okx",
                "operation": OPERATION,
                "status": "ok",
                "source": SOURCE,
                "message": "live 下单/撤单远程证明已闭环",
                "supported": True,
                "configured": True,
                "requested": 2,
                "rows": 2,
                "freshnessMs": freshness_ms,
                "evidence": {
                    "method": "internal",
                    "path": EVIDENCE_PATH,
                    "checkedAt": UNRECORDED_EVIDENCE_MARKER,
                    "docVersion": UNRECORDED_EVIDENCE_MARKER,
                    "schemaHash": UNRECORDED_EVIDENCE_MARKER,
                    "fixtureId": UNRECORDED_EVIDENCE_MARKER,
                    "parserTest": UNRECORDED_EVIDENCE_MARKER,
                    "requestBuilderTest": UNRECORDED_EVIDENCE_MARKER,
                    "authKind": AUTH_KIND,
                    "requestContext": [
                        "probe_scope=okx.order_write.live_place_cancel",
                        f"probe_source={SOURCE}",
                        "live_place_remote_proof=ok",
                        "live_cancel_remote_proof=ok",
                        "place_ack_count=1",
                        "cancel_requested_count=1",
                        "cancel_finality_count=1",
                        "sample_place_internal_order_id=live-order-1",
                        "sample_cancel_internal_order_id=live-order-1",
                        "sample_place_symbol=BTCUSDT",
                        "sample_cancel_symbol=BTCUSDT",
                        "sample_place_source=adapter_ack",
                        "sample_cancel_source=order_query",
                        f"sample_place_checked_at_ms={place_checked_at_ms}",
                        f"sample_cancel_checked_at_ms={observed_at_ms}",
                    ],
                    "docUrls": [],
                    "useCases": [
                        "order_write",
                        "live_place_cancel_remote_proof",
                    ],
                    "dataKinds": [
                        "order_ack",
                        "cancel_request_ack",
                        "cancel_finality",
                    ],
                    "rateScopes": [],
                    "weight": 0,
                },
                "observedAtMs": observed_at_ms,
            }
        ],
        "generatedAtMs": generated_at_ms,
        "rowCount": 1,
        "attentionCount": 0,
    }


def warn_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["status"] = "warn"
    fixture["rows"][0]["problem"] = {"code": "HEDGE_PRE_TRADE_REJECTED"}
    fixture["attentionCount"] = 1
    return fixture


def unsupported_static_gate_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["supported"] = False
    return fixture


def unconfigured_static_gate_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["configured"] = False
    return fixture


def missing_static_gate_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    del fixture["rows"][0]["supported"]
    return fixture


def accepted_row_error_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["error"] = "stale live mutation failure"
    return fixture


def accepted_row_retry_after_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["retryAfterMs"] = 1000
    return fixture


def accepted_row_extra_key_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["debugRawOrderId"] = "live-order-1"
    return fixture


def accepted_row_null_problem_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["problem"] = None
    return fixture


def accepted_row_null_error_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["error"] = None
    return fixture


def accepted_row_null_retry_after_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["retryAfterMs"] = None
    return fixture


def accepted_row_latency_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["latencyMs"] = 34
    fixture["rows"][0]["latencyP95Ms"] = 89
    return fixture


def accepted_row_null_latency_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["latencyMs"] = None
    return fixture


def mismatched_identity_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    context = fixture["rows"][0]["evidence"]["requestContext"]
    fixture["rows"][0]["evidence"]["requestContext"] = [
        "sample_cancel_internal_order_id=other-order"
        if item == "sample_cancel_internal_order_id=live-order-1"
        else item
        for item in context
    ]
    return fixture


def redacted_raw_identity_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    context = fixture["rows"][0]["evidence"]["requestContext"]
    fixture["rows"][0]["evidence"]["requestContext"] = [
        "sample_place_internal_order_id=[redacted]"
        if item == "sample_place_internal_order_id=live-order-1"
        else "sample_cancel_internal_order_id=[redacted]"
        if item == "sample_cancel_internal_order_id=live-order-1"
        else item
        for item in context
    ]
    return fixture


def missing_sample_symbol_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    context = fixture["rows"][0]["evidence"]["requestContext"]
    fixture["rows"][0]["evidence"]["requestContext"] = [
        item for item in context if item != "sample_cancel_symbol=BTCUSDT"
    ]
    return fixture


def mismatched_sample_symbol_fixture() -> dict[str, Any]:
    return replace_context(
        accepted_fixture(),
        "sample_cancel_symbol=BTCUSDT",
        "sample_cancel_symbol=ETHUSDT",
    )


def redacted_sample_symbol_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture = replace_context(
        fixture,
        "sample_place_symbol=BTCUSDT",
        "sample_place_symbol=[redacted]",
    )
    return replace_context(
        fixture,
        "sample_cancel_symbol=BTCUSDT",
        "sample_cancel_symbol=[redacted]",
    )


def blank_row_message_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["message"] = "   "
    return fixture


def placeholder_row_message_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["message"] = "none"
    return fixture


def redacted_row_message_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["message"] = "[redacted]"
    return fixture


def hashed_identity_fixture() -> dict[str, Any]:
    fixture = apply_hashed_identity(
        accepted_fixture(observed_at_ms=COMMITTED_SAMPLE_OBSERVED_AT_MS)
    )
    return add_committed_companion_row(fixture)


def committed_one_row_fixture() -> dict[str, Any]:
    return apply_hashed_identity(accepted_fixture())


def apply_hashed_identity(fixture: dict[str, Any]) -> dict[str, Any]:
    context = [
        item
        for item in fixture["rows"][0]["evidence"]["requestContext"]
        if not item.startswith("sample_place_internal_order_id=")
        and not item.startswith("sample_cancel_internal_order_id=")
    ]
    context.extend(
        [
            f"sample_place_internal_order_id_hash={HASH_IDENTITY_DIGEST}",
            f"sample_cancel_internal_order_id_hash={HASH_IDENTITY_DIGEST}",
        ]
    )
    fixture["rows"][0]["evidence"]["requestContext"] = context
    return fixture


def add_committed_companion_row(fixture: dict[str, Any]) -> dict[str, Any]:
    observed_at_ms = COMMITTED_SAMPLE_OBSERVED_AT_MS
    freshness_ms = fixture["generatedAtMs"] - observed_at_ms
    fixture["rows"].append(
        {
            "venue": "okx",
            "operation": "order_finality",
            "status": "ok",
            "source": "run_finality",
            "message": "订单终态回查样本已捕获",
            "freshnessMs": freshness_ms,
            "evidence": {
                "method": "internal",
                "path": "run_finality.refresh_pending_runs",
                "checkedAt": UNRECORDED_EVIDENCE_MARKER,
                "docVersion": UNRECORDED_EVIDENCE_MARKER,
                "schemaHash": UNRECORDED_EVIDENCE_MARKER,
                "fixtureId": UNRECORDED_EVIDENCE_MARKER,
                "parserTest": UNRECORDED_EVIDENCE_MARKER,
                "requestBuilderTest": UNRECORDED_EVIDENCE_MARKER,
                "authKind": "internal_order_query",
                "requestContext": [
                    "operation=order_finality",
                    "scanned_order_count=1",
                    "refreshed_order_count=1",
                    "remote_missing_count=0",
                    "skipped_terminal_count=0",
                    "refresh_failure_count=0",
                    "publish_failure_count=0",
                ],
                "docUrls": [],
                "useCases": [
                    "order_finality",
                    "execution_run_finality",
                    "close_run_finality",
                ],
                "dataKinds": [
                    "order_state",
                    "execution_run_finality",
                    "close_run_finality",
                ],
                "rateScopes": [],
                "weight": 0,
            },
            "observedAtMs": observed_at_ms,
        }
    )
    fixture["rowCount"] = len(fixture["rows"])
    fixture["attentionCount"] = sum(
        1 for row in fixture["rows"] if row.get("status") != "ok"
    )
    return fixture


def committed_low_epoch_time_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["generatedAtMs"] = 1800
    fixture["rows"][0]["observedAtMs"] = 1000
    fixture["rows"][1]["observedAtMs"] = 1000
    fixture = replace_context_key(fixture, "sample_place_checked_at_ms", "900")
    return replace_context_key(fixture, "sample_cancel_checked_at_ms", "1000")


def committed_future_time_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    observed_at_ms = current_unix_ms() + MAX_COMMITTED_SAMPLE_FUTURE_SKEW_MS + 60_000
    fixture["generatedAtMs"] = observed_at_ms + 800
    fixture["rows"][0]["observedAtMs"] = observed_at_ms
    fixture["rows"][1]["observedAtMs"] = observed_at_ms
    fixture = replace_context_key(
        fixture,
        "sample_place_checked_at_ms",
        str(observed_at_ms - 100),
    )
    return replace_context_key(
        fixture,
        "sample_cancel_checked_at_ms",
        str(observed_at_ms),
    )


def committed_companion_after_generated_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][1]["observedAtMs"] = fixture["generatedAtMs"] + 1
    return fixture


def committed_minimal_companion_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][1] = {
        "venue": "okx",
        "operation": "order_finality",
        "status": "ok",
        "source": "run_finality",
        "message": "订单终态回查样本已捕获",
        "observedAtMs": COMMITTED_SAMPLE_OBSERVED_AT_MS,
    }
    return fixture


def committed_companion_freshness_mismatch_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][1]["freshnessMs"] += 1
    return fixture


def committed_companion_empty_evidence_context_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][1]["evidence"]["requestContext"] = []
    return fixture


def committed_second_companion_extra_key_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    companion = dict(fixture["rows"][1])
    companion["operation"] = "private_ws_order_stream"
    companion["debugRawOrderId"] = "live-order-1"
    fixture["rows"].append(companion)
    fixture["rowCount"] = len(fixture["rows"])
    fixture["attentionCount"] = sum(
        1 for row in fixture["rows"] if row.get("status") != "ok"
    )
    return fixture


def hash_plus_raw_identity_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].extend(
        [
            "sample_place_internal_order_id=live-order-1",
            "sample_cancel_internal_order_id=live-order-1",
        ]
    )
    return fixture


def matching_evidence_request_id_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestId"] = "req-final"
    fixture["rows"][0]["evidence"]["requestContext"].extend(
        [
            "sample_place_request_id=req-place",
            "sample_cancel_request_id=req-final",
        ]
    )
    return fixture


def native_transport_metadata_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].extend(
        [
            "sample_place_native_transport=hyperliquid_ws_post",
            "sample_place_native_request_id=257",
            "sample_place_native_response_id=257",
        ]
    )
    return fixture


def committed_adapter_ack_cancel_finality_fixture() -> dict[str, Any]:
    return replace_context(
        hashed_identity_fixture(),
        "sample_cancel_source=order_query",
        "sample_cancel_source=adapter_ack",
    )


def committed_bare_hash_identity_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    digest = "162b8fa1015735430c68cef961bea09b41a2619d2512d5e399f972d533f9ad70"
    context = fixture["rows"][0]["evidence"]["requestContext"]
    fixture["rows"][0]["evidence"]["requestContext"] = [
        f"sample_place_internal_order_id_hash={digest}"
        if item.startswith("sample_place_internal_order_id_hash=")
        else f"sample_cancel_internal_order_id_hash={digest}"
        if item.startswith("sample_cancel_internal_order_id_hash=")
        else item
        for item in context
    ]
    return fixture


def committed_public_self_test_fixture_reuse_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    context = fixture["rows"][0]["evidence"]["requestContext"]
    public_hash = (
        "hmac-sha256:"
        "3b217ad2e4d6d7149d4640c53388e8946da8cbe8a404b78da2855faee46b447b"
    )
    fixture["rows"][0]["message"] = "live proof accepted"
    fixture["rows"][1]["message"] = "captured finality row"
    fixture["rows"][0]["evidence"]["requestContext"] = [
        f"sample_place_internal_order_id_hash={public_hash}"
        if item.startswith("sample_place_internal_order_id_hash=")
        else f"sample_cancel_internal_order_id_hash={public_hash}"
        if item.startswith("sample_cancel_internal_order_id_hash=")
        else item
        for item in context
    ]
    return fixture


def rows_with_non_object_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"].append("not-a-row")
    fixture["rowCount"] = 2
    return fixture


def row_count_mismatch_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rowCount"] = 2
    return fixture


def attention_count_mismatch_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["attentionCount"] = 1
    return fixture


def invalid_retry_after_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["retryAfterMs"] = True
    return fixture


def snapshot_extra_key_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["debugCapture"] = {"rawOrderResponse": "not allowed"}
    return fixture


def mismatched_hash_identity_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    context = fixture["rows"][0]["evidence"]["requestContext"]
    fixture["rows"][0]["evidence"]["requestContext"] = [
        "sample_cancel_internal_order_id_hash=hmac-sha256:" + "b" * 64
        if item.startswith("sample_cancel_internal_order_id_hash=")
        else item
        for item in context
    ]
    return fixture


def conflicting_extra_raw_identity_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].extend(
        [
            "sample_place_exchange_order_id=exchange-order-1",
            "sample_cancel_exchange_order_id=exchange-order-2",
        ]
    )
    return fixture


def conflicting_extra_hash_identity_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].extend(
        [
            "sample_place_exchange_order_id_hash=hmac-sha256:"
            "162b8fa1015735430c68cef961bea09b41a2619d2512d5e399f972d533f9ad70",
            "sample_cancel_exchange_order_id_hash=hmac-sha256:"
            "0e485377d6e1d2492831ef4a2d3536840291d0bfdb803faa2422881ee639b3c6",
        ]
    )
    return fixture


def one_sided_raw_identity_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].append(
        "sample_place_exchange_order_id=exchange-order-1"
    )
    return fixture


def one_sided_hash_identity_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].append(
        "sample_place_exchange_order_id_hash=hmac-sha256:"
        "162b8fa1015735430c68cef961bea09b41a2619d2512d5e399f972d533f9ad70"
    )
    return fixture


def unknown_context_key_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].append(
        "debug_raw_order_id=live-order-1"
    )
    return fixture


def invalid_hash_identity_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    context = fixture["rows"][0]["evidence"]["requestContext"]
    fixture["rows"][0]["evidence"]["requestContext"] = [
        "sample_place_internal_order_id_hash=redacted-order-1"
        if item.startswith("sample_place_internal_order_id_hash=")
        else item
        for item in context
    ]
    return fixture


def low_entropy_hash_identity_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    context = fixture["rows"][0]["evidence"]["requestContext"]
    fixture["rows"][0]["evidence"]["requestContext"] = [
        "sample_place_internal_order_id_hash=hmac-sha256:" + "a" * 64
        if item.startswith("sample_place_internal_order_id_hash=")
        else "sample_cancel_internal_order_id_hash=hmac-sha256:" + "a" * 64
        if item.startswith("sample_cancel_internal_order_id_hash=")
        else item
        for item in context
    ]
    return fixture


def duplicate_context_key_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].append(
        "sample_cancel_source=order_query"
    )
    return fixture


def malformed_context_item_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].append("sample_without_value")
    return fixture


def blank_context_value_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].append(
        "sample_place_request_id=   "
    )
    return fixture


def null_context_placeholder_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].append(
        "sample_place_request_id=null"
    )
    return fixture


def unmatched_evidence_request_id_fixture() -> dict[str, Any]:
    fixture = matching_evidence_request_id_fixture()
    fixture["rows"][0]["evidence"]["requestId"] = "req-stale"
    return fixture


def redacted_sample_request_id_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].append(
        "sample_place_request_id=[redacted]"
    )
    return fixture


def redacted_evidence_request_id_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestId"] = "[redacted]"
    fixture["rows"][0]["evidence"]["requestContext"].append(
        "sample_cancel_request_id=[redacted]"
    )
    return fixture


def asterisk_sample_request_id_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].append(
        "sample_cancel_request_id=***"
    )
    return fixture


def non_live_row_message_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["message"] = "paper live proof accepted"
    return fixture


def non_live_context_marker_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].append(
        "sample_place_request_id=sandbox-req-1"
    )
    return fixture


def non_live_companion_row_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][1]["message"] = "testnet finality row"
    return fixture


def non_live_venue_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["venue"] = "okx-testnet"
    fixture["rows"][0]["evidence"]["requestContext"] = [
        "probe_scope=okx-testnet.order_write.live_place_cancel"
        if item == "probe_scope=okx.order_write.live_place_cancel"
        else item
        for item in fixture["rows"][0]["evidence"]["requestContext"]
    ]
    return fixture


def missing_generated_at_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    del fixture["generatedAtMs"]
    return fixture


def missing_observed_at_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    del fixture["rows"][0]["observedAtMs"]
    return fixture


def inconsistent_time_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["generatedAtMs"] = fixture["rows"][0]["observedAtMs"] + 799
    return fixture


def future_observed_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["observedAtMs"] = fixture["generatedAtMs"] + 1
    return fixture


def missing_sample_checked_at_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    context = fixture["rows"][0]["evidence"]["requestContext"]
    fixture["rows"][0]["evidence"]["requestContext"] = [
        item for item in context if not item.startswith("sample_cancel_checked_at_ms=")
    ]
    return fixture


def invalid_sample_checked_at_fixture() -> dict[str, Any]:
    return replace_context_key(
        hashed_identity_fixture(),
        "sample_cancel_checked_at_ms",
        "true",
    )


def cancel_before_place_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture = replace_context_key(
        fixture,
        "sample_place_checked_at_ms",
        str(COMMITTED_SAMPLE_OBSERVED_AT_MS),
    )
    return replace_context_key(
        fixture,
        "sample_cancel_checked_at_ms",
        str(COMMITTED_SAMPLE_PLACE_CHECKED_AT_MS),
    )


def sample_observed_at_mismatch_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["observedAtMs"] = COMMITTED_SAMPLE_OBSERVED_AT_MS - 1
    fixture["generatedAtMs"] = fixture["rows"][0]["observedAtMs"] + 800
    return fixture


def missing_evidence_method_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    del fixture["rows"][0]["evidence"]["method"]
    return fixture


def missing_evidence_metadata_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    del fixture["rows"][0]["evidence"]["checkedAt"]
    return fixture


def extra_evidence_key_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["debugRawOrderId"] = "live-order-1"
    return fixture


def missing_evidence_use_case_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["useCases"] = ["order_write"]
    return fixture


def extra_evidence_use_case_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["useCases"].append("debug_raw_order")
    return fixture


def missing_evidence_data_kind_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["dataKinds"] = ["order_ack", "cancel_finality"]
    return fixture


def extra_evidence_data_kind_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["dataKinds"].append("debug_raw_order")
    return fixture


def non_empty_doc_urls_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["docUrls"] = ["https://example.invalid"]
    return fixture


def invalid_evidence_weight_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["weight"] = 1
    return fixture


def missing_proof_count_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    context = fixture["rows"][0]["evidence"]["requestContext"]
    fixture["rows"][0]["evidence"]["requestContext"] = [
        item for item in context if item != "place_ack_count=1"
    ]
    return fixture


def zero_place_proof_count_fixture() -> dict[str, Any]:
    return replace_context(
        hashed_identity_fixture(),
        "place_ack_count=1",
        "place_ack_count=0",
    )


def zero_cancel_finality_count_fixture() -> dict[str, Any]:
    return replace_context(
        hashed_identity_fixture(),
        "cancel_finality_count=1",
        "cancel_finality_count=0",
    )


def invalid_proof_count_fixture() -> dict[str, Any]:
    return replace_context(
        hashed_identity_fixture(),
        "cancel_requested_count=1",
        "cancel_requested_count=true",
    )


def native_request_without_transport_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].append(
        "sample_place_native_request_id=257"
    )
    return fixture


def redacted_native_transport_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].extend(
        [
            "sample_place_native_transport=[redacted]",
            "sample_place_native_request_id=257",
        ]
    )
    return fixture


def redacted_native_response_id_fixture() -> dict[str, Any]:
    fixture = hashed_identity_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].extend(
        [
            "sample_place_native_transport=hyperliquid_ws_post",
            "sample_place_native_response_id=[redacted]",
        ]
    )
    return fixture


def mismatched_probe_scope_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    context = fixture["rows"][0]["evidence"]["requestContext"]
    fixture["rows"][0]["evidence"]["requestContext"] = [
        "probe_scope=bybit.order_write.live_place_cancel"
        if item == "probe_scope=okx.order_write.live_place_cancel"
        else item
        for item in context
    ]
    return fixture


def sensitive_key_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["evidence"]["apiSecret"] = "should-not-be-committed"
    return fixture


def exchange_header_sensitive_key_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["evidence"]["OK-ACCESS-KEY"] = "should-not-be-committed"
    return fixture


def sensitive_value_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].append(
        "debug_header=Authorization: Bearer should-not-be-committed"
    )
    return fixture


def exchange_header_sensitive_value_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["evidence"]["requestContext"].append(
        "debug_header=OK-ACCESS-KEY: should-not-be-committed"
    )
    return fixture


def bool_freshness_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["freshnessMs"] = True
    return fixture


def incomplete_counts_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["requested"] = 1
    fixture["rows"][0]["rows"] = 1
    return fixture


def bool_counts_fixture() -> dict[str, Any]:
    fixture = accepted_fixture()
    fixture["rows"][0]["requested"] = True
    fixture["rows"][0]["rows"] = True
    return fixture


def invalid_place_source_fixture() -> dict[str, Any]:
    return replace_context(
        accepted_fixture(),
        "sample_place_source=adapter_ack",
        "sample_place_source=manual_fixture",
    )


def invalid_cancel_source_fixture() -> dict[str, Any]:
    return replace_context(
        accepted_fixture(),
        "sample_cancel_source=order_query",
        "sample_cancel_source=manual_fixture",
    )


def replace_context(fixture: dict[str, Any], old: str, new: str) -> dict[str, Any]:
    context = fixture["rows"][0]["evidence"]["requestContext"]
    fixture["rows"][0]["evidence"]["requestContext"] = [
        new if item == old else item for item in context
    ]
    return fixture


def replace_context_key(fixture: dict[str, Any], key: str, value: str) -> dict[str, Any]:
    context = fixture["rows"][0]["evidence"]["requestContext"]
    prefix = f"{key}="
    fixture["rows"][0]["evidence"]["requestContext"] = [
        f"{prefix}{value}" if item.startswith(prefix) else item for item in context
    ]
    return fixture


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except AcceptanceError as exc:
        print(f"live order runtime proof failed: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc
