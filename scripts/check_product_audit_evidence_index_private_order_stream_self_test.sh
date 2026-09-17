#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p \
  "$TMP/scripts" \
  "$TMP/docs/live_samples" \
  "$TMP/test/e2e" \
  "$TMP/frontend/src/panels/modules/settings/tabs/venue_credentials" \
  "$TMP/frontend/src/panels/modules/settings/tabs/venue_credentials/format" \
  "$TMP/frontend/src/panels/modules/settings/tabs/venue_credentials/panels" \
  "$TMP/frontend/src/panels/modules/settings/tabs/venue_credentials/tests_credentials" \
  "$TMP/frontend/src/panels/modules/settings/tabs" \
  "$TMP/crates/api/src/routers/trading" \
  "$TMP/shared-types/src/venues"

cp "$ROOT/scripts/check_product_audit_evidence_index.sh" "$TMP/scripts/"
cp "$ROOT/scripts/check_product_audit_evidence_index_private_order_stream_self_test.sh" "$TMP/scripts/"
cp "$ROOT/scripts/check_private_order_stream_live_sample_acceptance.py" "$TMP/scripts/"
cp "$ROOT/scripts/check_live_order_runtime_acceptance.py" "$TMP/scripts/"
git -C "$TMP" init -q

SAMPLE="docs/live_samples/okx-private-order-stream.json"
LIVE_HASH="hmac-sha256:4c1e7b84a2f6d9b3c5e8f1029384756a0b1c2d3e4f5061728394a5b6c7d8e9f0"
NOW_MS="$(($(date +%s) * 1000))"
ORDER_OBSERVED_MS="$((NOW_MS - 800))"
STREAM_OBSERVED_MS="$((NOW_MS - 500))"
PLACE_CHECKED_MS="$((NOW_MS - 900))"

cat >"$TMP/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md" <<'DOC'
### 🟡 6.3 建议整改顺序

| PR | 状态 | 范围 | 验收标准 |
|---|---|---|---|
| `PR-FR API Status Evidence Matrix` | 🟡 部分完成 | PR-FR private order stream sample index self-test | 验证：self-test |

### 🟡 6.4 其它
DOC

write_marker_files() {
  cat >"$TMP/test/e2e/mock_api.mjs" <<'JS'
const markers = [
	  "/e2e-settings-credential-static-adapter-boundary",
	  "/e2e-settings-private-order-stream-ok-capture-readiness",
	  "/e2e-settings-selected-venue-trading-runtime-all-ok-local-capture-shaped",
	  "/e2e-private-order-stream-warning",
	  "validationEvidence",
  'kind: "order_permission"',
  'status: "unknown"',
  "credentialsAvailable: true",
  "liveWrite: true",
  'scenario === "private-order-stream-warning" ? [] : [privateWsAuthFailure()]',
	  "function settingsPrivateOrderStreamOk()",
	  "function settingsCredentialOrderPermissionOkLocalCapture()",
	  "function settingsOrderWriteOkLocalCapture()",
	  "function settingsPrivateOrderStreamOkLocalCapture()",
	  "function settingsOrderFinalityOkLocalCapture()",
	  'source: "private_ws_runtime"',
	  'source: "credential_validation"',
	  'source: "live_order_proof_runtime"',
	  'source: "run_finality_runtime"',
	  '"official_evidence=order_state_stream"',
	  '"local_ui_ci_gate=true"',
	  '"not_real_exchange_live_sample=true"',
	  '"req-settings-private-order-stream-ok"',
	  '"req-settings-runtime-all-ok-order-write"',
	  '"req-private-order-stream-stale"',
	];
void markers;
JS
  cat >"$TMP/test/e2e/data_pipeline.spec.ts" <<'JS'
const markers = [
	  "settings credentials keep static adapter copy separate from runtime readiness",
	  "settings credentials surface private order stream capture readiness",
	  "settings credentials surface local capture-shaped trading runtime 4/4 ok gate",
	  "okx · 当前运行态 0/4 正常 · 4/4 待处理",
	  "okx · 当前运行态 1/4 正常 · 3/4 待处理",
	  "okx · 当前运行态 4/4 正常 · 0/4 待处理",
	  "当前可用",
	  "const privateOrderStreamRow = page",
	  "const tradingRuntimePanel = page",
	  "const permissionRow = tradingRuntimePanel",
	  "const writeRow = tradingRuntimePanel",
	  "const privateOrderStreamRow = tradingRuntimePanel",
	  "const orderFinalityRow = tradingRuntimePanel",
	  "credential_probe:order_permission",
	  "order_write",
	  "private_ws_order_stream",
	  "order_finality",
	  "credential_validation",
	  "live_order_proof_runtime",
	  "private_ws_runtime",
	  "run_finality_runtime",
	  "freshness 800ms",
	  "3/3",
	  "request_id req-settings-private-order-stream-ok",
	  "request_id req-settings-runtime-all-ok-order-permission",
	  "request_id req-settings-runtime-all-ok-order-write",
	  "request_id req-settings-runtime-all-ok-private-stream",
	  "request_id req-settings-runtime-all-ok-order-finality",
	  "local_ui_ci_gate=true",
	  "not_real_exchange_live_sample=true",
	  "official_evidence=order_state_stream",
  "okx 暂无私有订单流运行态记录",
  "私有订单事件流需要运行态样本",
  'expect(privateOrderStreamRow).not.toContainText("可下单")',
  'expect(privateOrderStreamRow).not.toContainText("权限验证完整")',
  "const orderFinalityRow = page",
  "const writeRow = tradingRuntimePanel.locator",
  "const orderFinalityRow = tradingRuntimePanel",
  "order_finality",
  "run_finality",
  "okx 暂无订单终态回查运行态记录",
  "未决订单产生后由 REST/WS 终态回查写入",
  'expect(orderFinalityRow).not.toContainText("权限验证完整")',
  "top status bar surfaces private order stream runtime warning",
];
void markers;
JS
  cat >"$TMP/frontend/src/panels/modules/settings/tabs/venue_credentials/format.rs" <<'TXT'
format module marker
TXT
  cat >"$TMP/frontend/src/panels/modules/settings/tabs/venue_credentials/format/status.rs" <<'TXT'
{configured}/{} 字段已填写 / {missing} / {validation} / 当前状态待运行态证据 / {write_support} / {}
当前运行态 {ready}/{total} 正常 · {attention}/{total} 待处理
TXT
  cat >"$TMP/frontend/src/panels/modules/settings/tabs/venue_credentials/capability.rs" <<'TXT'
静态写单声明；仍需 order_permission/private WS/order_finality 证据。
static_capability_summary_does_not_claim_live_readiness
TXT
  cat >"$TMP/frontend/src/panels/modules/settings/tabs/venue_credentials/tests_credentials/ws.rs" <<'TXT'
credential_field_label_never_claims_validation
TXT
  cat >"$TMP/frontend/src/panels/modules/settings/tabs/adapters.rs" <<'TXT'
字段组已补齐
可选路由；下单仍需票据级权限与运行态证据
TXT
  cat >"$TMP/frontend/src/panels/modules/settings/tabs/venue_credentials/panels.rs" <<'TXT'
panels module marker
TXT
  cat >"$TMP/frontend/src/panels/modules/settings/tabs/venue_credentials/panels/operation_health.rs" <<'TXT'
safe/noop 不授予 live_write
写单运行态需 live place/cancel/finality 证据
VenueOperationKind::PrivateWsOrderStream => "private_ws_runtime"
VenueOperationKind::OrderFinality => "run_finality"
TXT
  cat >"$TMP/crates/api/src/routers/trading/adapters.rs" <<'TXT'
至少补齐一个交易所 API 字段组
TXT
  cat >"$TMP/shared-types/src/venues/operation_kind_labels.rs" <<'TXT'
Self::OrderWrite => "写单运行态"
TXT
  cat >"$TMP/scripts/product_copy_gate.sh" <<'TXT'
Settings copy must keep credential/static adapter/runtime evidence separate
TXT
}

write_snapshot() {
  local stream_status="${1:-ok}"
  local problem_block=""
  if [[ "$stream_status" != "ok" ]]; then
    problem_block=', "problem": {"code": "PRIVATE_WS_ORDER_STREAM_STALE"}'
  fi
  cat >"$TMP/$SAMPLE" <<JSON
{
  "rows": [
    {
      "venue": "okx",
      "operation": "order_write",
      "status": "ok",
      "source": "live_order_proof_runtime",
      "message": "real exchange order write proof captured",
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
          "sample_place_internal_order_id_hash=$LIVE_HASH",
          "sample_cancel_internal_order_id_hash=$LIVE_HASH",
          "sample_place_symbol=BTCUSDT",
          "sample_cancel_symbol=BTCUSDT",
          "sample_place_source=adapter_ack",
          "sample_cancel_source=private_ws_order",
          "sample_place_checked_at_ms=$PLACE_CHECKED_MS",
          "sample_cancel_checked_at_ms=$ORDER_OBSERVED_MS"
        ],
        "docUrls": [],
        "useCases": ["order_write", "live_place_cancel_remote_proof"],
        "dataKinds": ["order_ack", "cancel_request_ack", "cancel_finality"],
        "rateScopes": [],
        "weight": 0
      },
      "observedAtMs": $ORDER_OBSERVED_MS
    },
    {
      "venue": "okx",
      "operation": "private_ws_order_stream",
      "status": "$stream_status",
      "source": "private_ws_runtime",
      "message": "real private order stream sample captured",
      "supported": true,
      "configured": true,
      "requested": 1,
      "rows": 1,
      "freshnessMs": 500,
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
          "fixture_id=not_recorded"
        ],
        "docUrls": ["https://www.okx.com/docs-v5/en/#order-book-trading-trade-ws-order-channel"],
        "useCases": ["private_ws_runtime", "order_stream"],
        "dataKinds": ["order_state_stream"],
        "rateScopes": [],
        "weight": 0
      },
      "observedAtMs": $STREAM_OBSERVED_MS$problem_block
    }
  ],
  "generatedAtMs": $NOW_MS,
  "rowCount": 2,
  "attentionCount": 0
}
JSON
}

write_ledger() {
  cat >"$TMP/docs/PRODUCT_AUDIT_EVIDENCE.tsv" <<TSV
pr_id	evidence_type	artifact	command	notes
PR-FR	settings-credential-static-adapter-copy-boundary-gate	test/e2e/data_pipeline.spec.ts	node --check test/e2e/mock_api.mjs; node --check test/e2e/data_pipeline.spec.ts; CI=1 npm run test:e2e:data-pipeline -- --grep "settings credentials keep static adapter copy separate from runtime readiness"; bash scripts/check_product_audit_evidence_index.sh	self-test prerequisite row
PR-FR	settings-selected-venue-trading-runtime-missing-evidence-browser-gate	test/e2e/data_pipeline.spec.ts	node --check test/e2e/mock_api.mjs; node --check test/e2e/data_pipeline.spec.ts; CI=1 npm run test:e2e:data-pipeline -- --grep "settings credentials keep static adapter copy separate from runtime readiness"; bash scripts/check_product_audit_evidence_index.sh	self-test prerequisite row
PR-FR	settings-private-order-stream-ok-capture-readiness-browser-gate	test/e2e/data_pipeline.spec.ts	node --check test/e2e/mock_api.mjs; node --check test/e2e/data_pipeline.spec.ts; CI=1 npm run test:e2e:data-pipeline -- --grep "settings credentials surface private order stream capture readiness"; bash scripts/check_product_audit_evidence_index.sh	self-test prerequisite row
PR-FR	settings-selected-venue-trading-runtime-all-ok-local-capture-shaped-browser-gate	test/e2e/data_pipeline.spec.ts	node --check test/e2e/mock_api.mjs; node --check test/e2e/data_pipeline.spec.ts; CI=1 npm run test:e2e:data-pipeline -- --grep "settings credentials surface local capture-shaped trading runtime 4/4 ok gate"; bash scripts/check_product_audit_evidence_index.sh	self-test prerequisite row
PR-FR	private-ws-order-stream-topbar-browser-gate	test/e2e/data_pipeline.spec.ts	node --check test/e2e/mock_api.mjs; node --check test/e2e/data_pipeline.spec.ts; CI=1 npm run test:e2e:data-pipeline -- --grep "top status bar surfaces private order stream runtime warning"; bash scripts/check_product_audit_evidence_index.sh	self-test prerequisite row
PR-FR	private-order-stream-live-sample-acceptance-gate	scripts/check_private_order_stream_live_sample_acceptance.py	python3 scripts/check_private_order_stream_live_sample_acceptance.py --self-test; bash scripts/check_product_audit_evidence_index.sh	self-test prerequisite row
PR-FR	private-order-stream-live-sample-index-gate	scripts/check_product_audit_evidence_index_private_order_stream_self_test.sh	bash -n scripts/check_product_audit_evidence_index_private_order_stream_self_test.sh; python3 -m py_compile scripts/check_private_order_stream_live_sample_acceptance.py; bash scripts/check_product_audit_evidence_index_private_order_stream_self_test.sh; bash scripts/check_product_audit_evidence_index.sh	self-test prerequisite row
PR-FR	private-order-stream-live-sample-acceptance:okx	$SAMPLE	python3 scripts/check_private_order_stream_live_sample_acceptance.py $SAMPLE --venue okx --require-hashed-identity	self-test private order stream live sample row
TSV
}

expect_rejected() {
  local label="$1"
  if bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null 2>&1; then
    printf 'private order stream evidence index self-test failed: %s was accepted\n' "$label" >&2
    exit 1
  fi
}

write_marker_files
write_snapshot ok
write_ledger
git -C "$TMP" add "$SAMPLE"
bash "$TMP/scripts/check_product_audit_evidence_index.sh" >/dev/null

git -C "$TMP" rm --cached -fq "$SAMPLE"
expect_rejected "untracked artifact"
git -C "$TMP" add "$SAMPLE"

cp "$TMP/$SAMPLE" "$TMP/docs/live_samples/binance-private-order-stream.json"
git -C "$TMP" add docs/live_samples/binance-private-order-stream.json
perl -0pi -e 's#docs/live_samples/okx-private-order-stream.json#docs/live_samples/binance-private-order-stream.json#g' "$TMP/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
expect_rejected "mismatched artifact venue basename"
write_ledger

perl -0pi -e 's/ --require-hashed-identity/ --require-hashed-identity; true/' "$TMP/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
expect_rejected "non-canonical verifier command"
write_ledger

write_snapshot warn
expect_rejected "warn private order stream sample"
write_snapshot ok

perl -0pi -e 's/private-order-stream-live-sample-acceptance:okx/private-order-stream-live-sample-acceptance:unknowncex/; s/--venue okx/--venue unknowncex/' "$TMP/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
expect_rejected "unsupported venue suffix"

printf 'OK private order stream evidence index self-test\n'
