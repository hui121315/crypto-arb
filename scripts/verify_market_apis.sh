#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${API_BASE:-}" ]]; then
  API_BASE="http://127.0.0.1:8000"
fi
API_TIMEOUT_SECS="${API_TIMEOUT_SECS:-120}"
ALLOW_RUNTIME_SKIP="${ALLOW_RUNTIME_SKIP:-0}"

validate_payload() {
  local mode="$1"
  local label="$2"
  local path="$3"
  python3 - "$mode" "$label" "$path" <<'PY'
import json
import sys

P0_KINDS = {
    "perp_cross",
    "perp_price_spread",
    "spot_perp",
    "cross_spot_perp",
    "spot_cross",
}
LIVE_KINDS = {"perp_cross", "perp_price_spread"}


class ContractError(Exception):
    pass


def row_count(body):
    if isinstance(body, list):
        return len(body)
    if not isinstance(body, dict):
        return 0
    for key in ("opportunities", "tokens", "rows"):
        if isinstance(body.get(key), list):
            return len(body[key])
    data = body.get("data")
    if isinstance(data, dict) and isinstance(data.get("ticks"), list):
        return len(data["ticks"])
    return 0


def opportunity_strategies(body):
    rows = body.get("rows") if isinstance(body, dict) else None
    if not isinstance(rows, list):
        raise ContractError("missing_rows")
    seen = set()
    for row in rows:
        kind = row.get("strategyKind") if isinstance(row, dict) else None
        if not isinstance(kind, str):
            raise ContractError("missing_strategy_kind")
        seen.add(kind)
    unknown = sorted(seen - P0_KINDS)
    if unknown:
        raise ContractError(f"unknown_strategies={','.join(unknown)}")
    if not rows:
        raise ContractError("empty_rows")
    return rows, seen


def main_strategy_kinds(body):
    if not isinstance(body, list):
        raise ContractError("missing_strategy_rows")
    indexed = {}
    for row in body:
        kind = row.get("kind") if isinstance(row, dict) else None
        if not isinstance(kind, str):
            raise ContractError("missing_strategy_kind")
        if kind in indexed:
            raise ContractError(f"duplicate_strategy={kind}")
        indexed[kind] = row
    actual = set(indexed)
    if actual != P0_KINDS:
        raise ContractError(
            f"strategy_set expected={','.join(sorted(P0_KINDS))} "
            f"actual={','.join(sorted(actual))}"
        )
    for kind, row in indexed.items():
        for field in (
            "implemented",
            "frontendEnabled",
            "enginePresent",
            "dataContractReady",
            "executionSupported",
        ):
            if row.get(field) is not True:
                raise ContractError(f"{kind}.{field}=false")
        expected_live = kind in LIVE_KINDS
        if row.get("liveExecutionSupported") is not expected_live:
            raise ContractError(
                f"{kind}.liveExecutionSupported="
                f"{row.get('liveExecutionSupported')!r} expected={expected_live}"
            )
    return indexed


def self_test():
    assert row_count({"data": {"ticks": [{"symbol": "BTC/USDT"}]}}) == 1
    rows = [{"strategyKind": kind} for kind in sorted(P0_KINDS)]
    _, seen = opportunity_strategies({"rows": rows})
    assert seen == P0_KINDS
    contracts = []
    for kind in sorted(P0_KINDS):
        contracts.append(
            {
                "kind": kind,
                "implemented": True,
                "frontendEnabled": True,
                "enginePresent": True,
                "dataContractReady": True,
                "executionSupported": True,
                "liveExecutionSupported": kind in LIVE_KINDS,
            }
        )
    main_strategy_kinds(contracts)
    invalid = [dict(row) for row in contracts]
    invalid[0]["liveExecutionSupported"] = not invalid[0]["liveExecutionSupported"]
    try:
        main_strategy_kinds(invalid)
    except ContractError:
        return
    raise ContractError("self_test_accepted_invalid_live_capability")


mode, label, path = sys.argv[1:4]
if mode == "self-test":
    try:
        self_test()
    except (AssertionError, ContractError) as error:
        print(f"market API contract self-test failed: {error}")
        sys.exit(1)
    print("market API contract self-test passed")
    sys.exit(0)

with open(path, "r", encoding="utf-8") as fh:
    body = json.load(fh)

try:
    if mode == "count":
        count = row_count(body)
        if count <= 0:
            raise ContractError("empty_rows")
        print(f"local {label} ok count={count}")
    elif mode == "opportunities":
        rows, seen = opportunity_strategies(body)
        print(
            f"local {label} ok count={len(rows)} "
            f"strategies={','.join(sorted(seen))}"
        )
    elif mode == "main-kinds":
        indexed = main_strategy_kinds(body)
        live = sorted(
            kind
            for kind, row in indexed.items()
            if row["liveExecutionSupported"]
        )
        print(
            f"local {label} ok count={len(indexed)} "
            f"live={','.join(live)}"
        )
    else:
        raise ContractError(f"unknown_mode={mode}")
except ContractError as error:
    print(f"local {label} fail reason={error}")
    sys.exit(1)
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  validate_payload self-test self-test /dev/null
  exit 0
fi

cargo run -q -p exchange --example verify_spot_apis

if ! curl -fsS -m 2 "$API_BASE/health" >/dev/null; then
  printf 'local api skipped base=%s reason=health_unreachable\n' "$API_BASE"
  if [[ "$ALLOW_RUNTIME_SKIP" == "1" ]]; then
    exit 0
  fi
  exit 1
fi

probe_json_count() {
  local label="$1"
  local path="$2"
  local exposure="${3:-required}"
  local tmp="${TMPDIR:-/tmp}/crossline-${label}.json"
  local code
  code="$(curl -sS -m "$API_TIMEOUT_SECS" -o "$tmp" -w '%{http_code}' "$API_BASE$path")"
  if [[ "$code" == "404" && "$exposure" == "default_off" ]]; then
    printf 'local %s skipped reason=default_off enable=APP_API_SURFACE__SPOT_V1=true\n' "$label"
    return 0
  fi
  if [[ "$code" != "200" ]]; then
    printf 'local %s fail status=%s\n' "$label" "$code"
    return 1
  fi
  validate_payload count "$label" "$tmp"
}

probe_main_strategy_contract() {
  local label="main_strategy_kinds"
  local tmp="${TMPDIR:-/tmp}/crossline-${label}.json"
  local code
  code="$(curl -sS -m "$API_TIMEOUT_SECS" -o "$tmp" -w '%{http_code}' "$API_BASE/api/strategy/main-kinds")"
  if [[ "$code" != "200" ]]; then
    printf 'local %s fail status=%s\n' "$label" "$code"
    return 1
  fi
  validate_payload main-kinds "$label" "$tmp"
}

probe_p0_opportunities() {
  local label="futures_merged"
  local path="/api/v3/arbitrage/opportunities/list?pageSize=50&fast=true&sortKey=score&strategy=perp_cross,perp_price_spread,spot_perp,cross_spot_perp,spot_cross"
  local tmp="${TMPDIR:-/tmp}/crossline-${label}.json"
  local code
  code="$(curl -sS -m "$API_TIMEOUT_SECS" -o "$tmp" -w '%{http_code}' "$API_BASE$path")"
  if [[ "$code" != "200" ]]; then
    printf 'local %s fail status=%s\n' "$label" "$code"
    return 1
  fi
  validate_payload opportunities "$label" "$tmp"
}

probe_main_strategy_contract
probe_json_count spot_ticks '/api/v1/spot/ticks?symbol=BTC' default_off
probe_p0_opportunities
