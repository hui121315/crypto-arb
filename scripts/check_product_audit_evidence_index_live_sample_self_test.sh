#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/scripts" "$TMP/docs/live_samples"
cp "$ROOT/scripts/check_product_audit_evidence_index.sh" "$TMP/scripts/"
cp "$ROOT/scripts/check_live_order_runtime_acceptance.py" "$TMP/scripts/"
git -C "$TMP" init -q

DEFAULT_LIVE_HASH="hmac-sha256:162b8fa1015735430c68cef961bea09b41a2619d2512d5e399f972d533f9ad70"
OTHER_LIVE_HASH="hmac-sha256:0e485377d6e1d2492831ef4a2d3536840291d0bfdb803faa2422881ee639b3c6"
PUBLIC_SELF_TEST_HASH="hmac-sha256:3b217ad2e4d6d7149d4640c53388e8946da8cbe8a404b78da2855faee46b447b"
BASE_OBSERVED_AT_MS=1780000000000
BASE_PLACE_CHECKED_AT_MS=1779999999900
BASE_GENERATED_AT_MS=1780000000800
FUTURE_OBSERVED_AT_MS=$(($(date +%s) * 1000 + 600000))
FUTURE_PLACE_CHECKED_AT_MS=$((FUTURE_OBSERVED_AT_MS - 100))
FUTURE_GENERATED_AT_MS=$((FUTURE_OBSERVED_AT_MS + 800))

cat >"$TMP/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md" <<'DOC'
### 🟡 6.3 建议整改顺序

| PR | 状态 | 范围 | 验收标准 |
|---|---|---|---|
| `PR-M Settings/API Status UX` | 🟡 部分完成 | PR-M live sample gate self-test | 验证：self-test |

### 🟡 6.4 其它
DOC

write_snapshot() {
  local status="${1:-ok}"
  local place_hash="${2:-$DEFAULT_LIVE_HASH}"
  local cancel_hash="${3:-$place_hash}"
  local shape="${4:-with_companion}"
  local companion_row=""
  local row_count=1
  local companion_freshness_ms=$((BASE_GENERATED_AT_MS - BASE_OBSERVED_AT_MS))
  if [[ "$shape" == "with_companion" ]]; then
    row_count=2
    printf -v companion_row ',\n    {\n      "venue": "okx",\n      "operation": "order_finality",\n      "status": "ok",\n      "source": "run_finality",\n      "message": "订单终态回查样本已捕获",\n      "freshnessMs": %s,\n      "evidence": {\n        "method": "internal",\n        "path": "run_finality.refresh_pending_runs",\n        "checkedAt": "not_recorded",\n        "docVersion": "not_recorded",\n        "schemaHash": "not_recorded",\n        "fixtureId": "not_recorded",\n        "parserTest": "not_recorded",\n        "requestBuilderTest": "not_recorded",\n        "authKind": "internal_order_query",\n        "requestContext": [\n          "operation=order_finality",\n          "scanned_order_count=1",\n          "refreshed_order_count=1",\n          "remote_missing_count=0",\n          "skipped_terminal_count=0",\n          "refresh_failure_count=0",\n          "publish_failure_count=0"\n        ],\n        "docUrls": [],\n        "useCases": ["order_finality", "execution_run_finality", "close_run_finality"],\n        "dataKinds": ["order_state", "execution_run_finality", "close_run_finality"],\n        "rateScopes": [],\n        "weight": 0\n      },\n      "observedAtMs": %s\n    }' "$companion_freshness_ms" "$BASE_OBSERVED_AT_MS"
  fi
  cat >"$TMP/docs/live_samples/okx.json" <<JSON
{
  "rows": [
    {
      "venue": "okx",
      "operation": "order_write",
      "status": "$status",
      "source": "live_order_proof_runtime",
      "message": "live 下单/撤单远程证明已闭环",
      "supported": true,
      "configured": true,
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
          "sample_place_internal_order_id_hash=$place_hash",
          "sample_cancel_internal_order_id_hash=$cancel_hash",
          "sample_place_symbol=BTCUSDT",
          "sample_cancel_symbol=BTCUSDT",
          "sample_place_source=adapter_ack",
          "sample_cancel_source=order_query",
          "sample_place_checked_at_ms=$BASE_PLACE_CHECKED_AT_MS",
          "sample_cancel_checked_at_ms=$BASE_OBSERVED_AT_MS"
        ],
        "docUrls": [],
        "useCases": ["order_write", "live_place_cancel_remote_proof"],
        "dataKinds": ["order_ack", "cancel_request_ack", "cancel_finality"],
        "rateScopes": [],
        "weight": 0
      },
      "observedAtMs": $BASE_OBSERVED_AT_MS
    }${companion_row}
  ],
  "generatedAtMs": $BASE_GENERATED_AT_MS,
  "rowCount": $row_count,
  "attentionCount": 0
}
JSON
}

write_ledger() {
  cat >"$TMP/docs/PRODUCT_AUDIT_EVIDENCE.tsv" <<'TSV'
pr_id	evidence_type	artifact	command	notes
PR-M	runtime-acceptance-live-order-proof	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-acceptance-readiness	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-artifact-redaction-readiness	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-hashed-identity-readiness	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-committed-hash-identity-gate	scripts/check_product_audit_evidence_index.sh	true	self-test prerequisite row
PR-M	live-sample-hash-algorithm-prefix-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-evidence-index-verifier-execution	scripts/check_product_audit_evidence_index.sh	true	self-test prerequisite row
PR-M	live-sample-verifier-shape-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-artifact-integrity-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-git-tracked-artifact-gate	scripts/check_product_audit_evidence_index.sh	true	self-test prerequisite row
PR-M	live-sample-artifact-venue-binding-gate	scripts/check_product_audit_evidence_index.sh	true	self-test prerequisite row
PR-M	live-sample-command-artifact-exact-gate	scripts/check_product_audit_evidence_index.sh	true	self-test prerequisite row
PR-M	live-sample-envelope-raw-id-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-checked-at-timeline-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-symbol-context-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-committed-capture-context-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-committed-epoch-timestamp-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-committed-timestamp-window-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-companion-row-all-matches-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-companion-row-shape-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-self-test-fixture-reuse-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-evidence-contract-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-evidence-schema-exact-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-snapshot-row-schema-exact-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-native-transport-metadata-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-static-usability-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-supported-venue-suffix-gate	scripts/check_product_audit_evidence_index.sh	true	self-test prerequisite row
PR-M	live-order-proof-runtime-request-context-allowlist-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-proof-count-context-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-identity-family-consistency-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-identity-family-completeness-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-request-context-allowlist-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-request-context-non-empty-value-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-request-id-placeholder-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-non-live-marker-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-accepted-row-clean-status-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-evidence-request-id-match-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-committed-cancel-finality-source-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-order-proof-runtime-identity-family-parity-gate	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	credential-update-invalidates-live-order-proof	scripts/check_live_order_runtime_acceptance.py	true	self-test prerequisite row
PR-M	live-sample-acceptance:okx	docs/live_samples/okx.json	python3 scripts/check_live_order_runtime_acceptance.py docs/live_samples/okx.json --venue okx --require-hashed-identity	self-test live sample row
TSV
}

write_snapshot ok
write_ledger
git -C "$TMP" add docs/live_samples/okx.json
bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null

write_snapshot ok "$PUBLIC_SELF_TEST_HASH"
perl -0pi -e 's/"message": "live 下单\/撤单远程证明已闭环"/"message": "live proof accepted"/; s/"message": "订单终态回查样本已捕获"/"message": "captured finality row"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: public self-test fixture reuse was accepted\n' >&2
  exit 1
fi

write_snapshot ok "$DEFAULT_LIVE_HASH" "$DEFAULT_LIVE_HASH" without_companion
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: one-row committed fixture was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"attentionCount": 0/"attentionCount": 0,\n  "debugCapture": {"rawOrderResponse": "not allowed"}/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: extra snapshot key was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"message": "live 下单\/撤单远程证明已闭环"/"message": "live 下单\/撤单远程证明已闭环",\n      "debugRawOrderId": "live-order-1"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: extra accepted row key was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"message": "live 下单\/撤单远程证明已闭环"/"message": "live 下单\/撤单远程证明已闭环",\n      "problem": null/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: ok row with null problem was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"message": "live 下单\/撤单远程证明已闭环"/"message": "live 下单\/撤单远程证明已闭环",\n      "error": null/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: ok row with null error was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"freshnessMs": 800/"freshnessMs": 800,\n      "retryAfterMs": null/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: ok row with null retryAfterMs was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_checked_at_ms=1780000000000"/"sample_cancel_checked_at_ms=1780000000000",\n          "sample_place_request_id=null"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: null requestContext placeholder was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_checked_at_ms=1780000000000"/"sample_cancel_checked_at_ms=1780000000000",\n          "sample_place_request_id=[redacted]"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: redacted sample request id was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"authKind": "live_order_remote_proof"/"authKind": "live_order_remote_proof",\n        "requestId": "[redacted]"/; s/"sample_cancel_checked_at_ms=1780000000000"/"sample_cancel_checked_at_ms=1780000000000",\n          "sample_cancel_request_id=[redacted]"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: redacted evidence requestId was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"message": "live 下单\/撤单远程证明已闭环"/"message": "   "/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: blank accepted row message was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"message": "live 下单\/撤单远程证明已闭环"/"message": "paper live proof accepted"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: paper marker was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_checked_at_ms=1780000000000"/"sample_cancel_checked_at_ms=1780000000000",\n          "sample_place_request_id=sandbox-req-1"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: sandbox context marker was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"message": "订单终态回查样本已捕获"/"message": "testnet finality row"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: companion row testnet marker was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"venue": "okx"/"venue": "okx_testnet"/; s/"probe_scope=okx.order_write.live_place_cancel"/"probe_scope=okx_testnet.order_write.live_place_cancel"/' "$TMP/docs/live_samples/okx.json"
perl -0pi -e 's/PR-M\tlive-sample-acceptance:okx\t/PR-M\tlive-sample-acceptance:okx_testnet\t/; s/--venue okx /--venue okx_testnet /; s#docs/live_samples/okx.json#docs/live_samples/okx_testnet.json#g' "$TMP/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
mv "$TMP/docs/live_samples/okx.json" "$TMP/docs/live_samples/okx_testnet.json"
git -C "$TMP" add docs/live_samples/okx_testnet.json
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: testnet venue marker was accepted\n' >&2
  exit 1
fi
git -C "$TMP" rm --cached -fq docs/live_samples/okx_testnet.json
rm -f "$TMP/docs/live_samples/okx_testnet.json"
write_ledger

write_snapshot ok
perl -0pi -e 's/"sample_cancel_checked_at_ms=1780000000000"/"sample_cancel_checked_at_ms=1780000000000",\n          "sample_place_native_request_id=257"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: native id without transport was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_checked_at_ms=1780000000000"/"sample_cancel_checked_at_ms=1780000000000",\n          "sample_place_native_transport=[redacted]",\n          "sample_place_native_request_id=257"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: redacted native transport was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_checked_at_ms=1780000000000"/"sample_cancel_checked_at_ms=1780000000000",\n          "sample_place_native_transport=hyperliquid_ws_post",\n          "sample_place_native_response_id=[redacted]"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: redacted native response id was accepted\n' >&2
  exit 1
fi

git -C "$TMP" rm --cached -fq docs/live_samples/okx.json
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: untracked artifact was accepted\n' >&2
  exit 1
fi
git -C "$TMP" add docs/live_samples/okx.json

write_snapshot ok \
  "$DEFAULT_LIVE_HASH" \
  "$OTHER_LIVE_HASH"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: mismatched identity was accepted\n' >&2
  exit 1
fi

write_snapshot ok hmac-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: low-entropy hash was accepted\n' >&2
  exit 1
fi

write_snapshot ok 162b8fa1015735430c68cef961bea09b41a2619d2512d5e399f972d533f9ad70
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: bare committed hash was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_source=order_query"/"sample_cancel_source=order_query",\n          "sample_cancel_source=order_query"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: duplicate context key was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"generatedAtMs": 1780000000800/"generatedAtMs": 1800000000799/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: inconsistent timestamp was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"generatedAtMs": 1780000000800/"generatedAtMs": 1800/; s/"observedAtMs": 1780000000000/"observedAtMs": 1000/g; s/sample_place_checked_at_ms=1779999999900/sample_place_checked_at_ms=900/; s/sample_cancel_checked_at_ms=1780000000000/sample_cancel_checked_at_ms=1000/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: low epoch committed timestamp was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e "s/\"generatedAtMs\": 1780000000800/\"generatedAtMs\": $FUTURE_GENERATED_AT_MS/; s/\"observedAtMs\": 1780000000000/\"observedAtMs\": $FUTURE_OBSERVED_AT_MS/g; s/sample_place_checked_at_ms=1779999999900/sample_place_checked_at_ms=$FUTURE_PLACE_CHECKED_AT_MS/; s/sample_cancel_checked_at_ms=1780000000000/sample_cancel_checked_at_ms=$FUTURE_OBSERVED_AT_MS/" "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: future committed timestamp was accepted\n' >&2
  exit 1
fi

write_snapshot ok
python3 - "$TMP/docs/live_samples/okx.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
snapshot = json.loads(path.read_text(encoding="utf-8"))
snapshot["rows"][1]["observedAtMs"] = snapshot["generatedAtMs"] + 1
path.write_text(json.dumps(snapshot, indent=2) + "\n", encoding="utf-8")
PY
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: companion timestamp after generatedAtMs was accepted\n' >&2
  exit 1
fi

write_snapshot ok
python3 - "$TMP/docs/live_samples/okx.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
snapshot = json.loads(path.read_text(encoding="utf-8"))
extra = dict(snapshot["rows"][1])
extra["operation"] = "private_ws_order_stream"
extra["debugRawOrderId"] = "live-order-1"
snapshot["rows"].append(extra)
snapshot["rowCount"] = len(snapshot["rows"])
path.write_text(json.dumps(snapshot, indent=2) + "\n", encoding="utf-8")
PY
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: second companion row extra key was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"rowCount": 2/"rowCount": 3/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: rowCount mismatch was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"configured": true/"configured": false/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: configured=false static gate was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_checked_at_ms=1780000000000"/"sample_cancel_checked_at_ms=1780000000000",\n          "sample_place_request_id=   "/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: blank requestContext value was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"message": "live 下单\/撤单远程证明已闭环"/"message": "live 下单\/撤单远程证明已闭环",\n      "error": "stale live mutation failure"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: ok row with error was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"freshnessMs": 800/"freshnessMs": 800,\n      "retryAfterMs": 1000/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: ok row with retryAfterMs was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"authKind": "live_order_remote_proof"/"authKind": "live_order_remote_proof",\n        "requestId": "req-stale"/; s/"sample_cancel_checked_at_ms=1780000000000"/"sample_cancel_checked_at_ms=1780000000000",\n          "sample_place_request_id=req-place",\n          "sample_cancel_request_id=req-final"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: unmatched evidence requestId was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_source=order_query"/"sample_cancel_source=adapter_ack"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: adapter_ack cancel finality source was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_symbol=BTCUSDT"/"sample_cancel_symbol=BTCUSDT",\n          "sample_place_internal_order_id=raw-order-1"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: raw order id with hashed identity was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_checked_at_ms=1780000000000"/"sample_cancel_checked_at_ms=1799999999899"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: cancel-before-place checked_at timeline was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/,\n          "sample_cancel_checked_at_ms=1780000000000"//' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: missing sample checked_at was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/,\n          "sample_cancel_symbol=BTCUSDT"//' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: missing sample symbol was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_place_symbol=BTCUSDT"/"sample_place_symbol=[redacted]"/; s/"sample_cancel_symbol=BTCUSDT"/"sample_cancel_symbol=[redacted]"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: redacted sample symbol was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_symbol=BTCUSDT"/"sample_cancel_symbol=ETHUSDT"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: mismatched sample symbol was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/        "method": "internal",\n//' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: missing evidence method was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/        "checkedAt": "not_recorded",\n//' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: missing evidence metadata was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/        "authKind": "live_order_remote_proof"/        "debugRawOrderId": "live-order-1",\n        "authKind": "live_order_remote_proof"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: extra evidence key was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/, "live_place_cancel_remote_proof"//' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: missing live proof useCase was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"useCases": \["order_write", "live_place_cancel_remote_proof"\]/"useCases": ["order_write", "live_place_cancel_remote_proof", "debug_raw_order"]/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: extra evidence useCase was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/, "cancel_request_ack"//' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: missing cancel request dataKind was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"dataKinds": \["order_ack", "cancel_request_ack", "cancel_finality"\]/"dataKinds": ["order_ack", "cancel_request_ack", "cancel_finality", "debug_raw_order"]/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: extra evidence dataKind was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/,\n          "place_ack_count=1"//' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: missing proof count context was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"cancel_finality_count=1"/"cancel_finality_count=0"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: zero cancel finality count was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_symbol=BTCUSDT"/"sample_cancel_symbol=BTCUSDT",\n          "sample_place_exchange_order_id_hash=hmac-sha256:162b8fa1015735430c68cef961bea09b41a2619d2512d5e399f972d533f9ad70",\n          "sample_cancel_exchange_order_id_hash=hmac-sha256:0e485377d6e1d2492831ef4a2d3536840291d0bfdb803faa2422881ee639b3c6"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: conflicting extra hash identity was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_symbol=BTCUSDT"/"sample_cancel_symbol=BTCUSDT",\n          "sample_place_exchange_order_id_hash=hmac-sha256:162b8fa1015735430c68cef961bea09b41a2619d2512d5e399f972d533f9ad70"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: one-sided extra hash identity was accepted\n' >&2
  exit 1
fi

write_snapshot ok
perl -0pi -e 's/"sample_cancel_symbol=BTCUSDT"/"sample_cancel_symbol=BTCUSDT",\n          "debug_raw_order_id=live-order-1"/' "$TMP/docs/live_samples/okx.json"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: unknown requestContext key was accepted\n' >&2
  exit 1
fi

write_snapshot ok
write_ledger
cp "$TMP/docs/live_samples/okx.json" "$TMP/docs/outside.json"
perl -0pi -e 's#docs/live_samples/okx.json#docs/live_samples/../outside.json#g' "$TMP/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: escaped artifact path was accepted\n' >&2
  exit 1
fi

write_snapshot ok
write_ledger
cp "$TMP/docs/live_samples/okx.json" "$TMP/docs/live_samples/binance.json"
git -C "$TMP" add docs/live_samples/binance.json
perl -0pi -e 's#docs/live_samples/okx.json#docs/live_samples/binance.json#g' "$TMP/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: mismatched artifact venue basename was accepted\n' >&2
  exit 1
fi

write_snapshot ok
write_ledger
perl -0pi -e 's/"venue": "okx"/"venue": "unknowncex"/g; s/probe_scope=okx.order_write.live_place_cancel/probe_scope=unknowncex.order_write.live_place_cancel/' "$TMP/docs/live_samples/okx.json"
mv "$TMP/docs/live_samples/okx.json" "$TMP/docs/live_samples/unknowncex.json"
git -C "$TMP" add docs/live_samples/unknowncex.json
perl -0pi -e 's/PR-M\tlive-sample-acceptance:okx\t/PR-M\tlive-sample-acceptance:unknowncex\t/; s#docs/live_samples/okx.json#docs/live_samples/unknowncex.json#g; s/--venue okx /--venue unknowncex /' "$TMP/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: unsupported venue suffix was accepted\n' >&2
  exit 1
fi

write_ledger
write_snapshot warn
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: warn status was accepted\n' >&2
  exit 1
fi

write_snapshot ok
write_ledger
perl -0pi -e 's/ --require-hashed-identity//' "$TMP/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: missing hashed identity flag was accepted\n' >&2
  exit 1
fi

write_snapshot ok
write_ledger
perl -0pi -e 's/ --require-hashed-identity/ --require-hashed-identity; true/' "$TMP/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
  printf 'live sample evidence index self-test failed: non-canonical command was accepted\n' >&2
  exit 1
fi

printf 'OK live sample evidence index self-test\n'
