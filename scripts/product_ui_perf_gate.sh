#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TRANSPORT_PROBE="$ROOT/frontend/src/api/rest/transport/probe.rs"

bash "$ROOT/scripts/check_frontend_table_budget_self_test.sh"
bash "$ROOT/scripts/check_frontend_table_budget.sh"

if rg -n 'OpportunitySelection|to_opportunity_selection|from_selection\(selection\.get\(\)\)|RwSignal<OpportunitySelection>' \
  "$ROOT/frontend/src/panels" --glob '*.rs'; then
  printf 'product UI perf gate failed: execution handoff must use ExecutionSelection directly\n' >&2
  exit 1
fi

if rg -n 'pub selection: ExecutionSelection' \
  "$ROOT/frontend/src/panels/modules/execution/data/preview/model.rs" \
  || rg -n 'selection: &ExecutionSelection' \
    "$ROOT/frontend/src/panels/modules/execution/data/preview/response.rs"; then
  printf 'product UI perf gate failed: preview API seed must not carry full ExecutionSelection\n' >&2
  exit 1
fi

CONFIRM_ACTIONS="$ROOT/frontend/src/panels/modules/execution/data/actions.rs"
if ! rg -q 'struct ConfirmHedgeSeed' "$CONFIRM_ACTIONS" \
  || ! rg -q 'pub seed: ConfirmHedgeSeed' "$CONFIRM_ACTIONS" \
  || ! rg -q 'pub request: HedgeConfirmApiRequest' "$CONFIRM_ACTIONS" \
  || ! rg -q "pub mode_label: &'static str" "$CONFIRM_ACTIONS" \
  || rg -n 'mode_label: String|pub idempotency_key: String|pub ticket_id: Option<String>|confirm_hedge_task\([^)]*opportunity_id: String' \
    "$CONFIRM_ACTIONS" \
  || rg -n 'mode_label\.to_string\(\)' \
    "$ROOT/frontend/src/panels/modules/execution" --glob '*.rs'; then
  printf 'product UI perf gate failed: confirm API seed must stay separate from UI label allocation\n' >&2
  exit 1
fi

if ! rg -Uq '<table[[:space:]]+class="clean-table opportunity-table"[[:space:]]+data-table-budget="server-page"' \
    "$ROOT/frontend/src/panels/modules/opportunities/components/opportunity_table.rs" \
  || ! rg -Uq '<table[[:space:]]+class="clean-table futures-table"[[:space:]]+data-table-budget="server-page"[[:space:]]+aria-label="期货套利候选"' \
    "$ROOT/frontend/src/panels/modules/futures/components/opportunity_table.rs"; then
  printf 'product UI perf gate failed: opportunity hot-path tables must declare server-page render budget\n' >&2
  exit 1
fi

if ! rg -q 'test:e2e:perf-smoke' "$ROOT/package.json" \
  || ! rg -q 'e2e-large-tables' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'large table perf smoke keeps opportunity switch list-only and bounded' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'OPPORTUNITY_SWITCH_RENDER_BUDGET_MS' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'OPPORTUNITY_LIST_BROWSER_CLONE_BUDGET_MS' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'OPPORTUNITY_LIST_BROWSER_PARSE_BUDGET_MS' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'OPPORTUNITY_LIST_WASM_DECODE_BUDGET_MS' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'OPPORTUNITY_TABLE_RAF_COMMIT_BUDGET_MS' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'OPPORTUNITY_SWITCH_FRAME_GAP_BUDGET_MS' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'OPPORTUNITY_SWITCH_LONG_TASK_BUDGET_MS' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'OPPORTUNITY_SWITCH_HEAP_DELTA_BUDGET_BYTES' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'OPPORTUNITY_LIST_PAYLOAD_BUDGET_BYTES' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q '__crosslineOpportunityHotPath' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'cloneReadMs' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'jsonParseMs' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q '__crosslineWasmSerdeMetrics' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'wasm_decode_probe_start' "$ROOT/frontend/src/api/rest/transport.rs" \
  || ! rg -q 'resp\.json::<T>\(\)\.await' "$ROOT/frontend/src/api/rest/transport.rs" \
  || ! rg -q 'wasm_decode_probe_finish\(decode_started_at_ms, decoded\.is_ok\(\)\)' "$ROOT/frontend/src/api/rest/transport.rs" \
  || ! rg -q 'path: "/api/v3/arbitrage/opportunities/list"' "$TRANSPORT_PROBE" \
  || ! rg -q 'startOpportunityHotPathProbe' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'stopOpportunityHotPathProbe' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'longTaskMs' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'heapDeltaBytes' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'maxFrameGapMs' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q '51-100 / 500' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q '1-50 / 500' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q '1-50 / 1000' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'opportunity-layout table.clean-table tbody tr' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'legacyWideRequests' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q '200 条' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'PR-CO visual smoke' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'VISUAL_SMOKE_VIEWPORTS' "$ROOT/test/e2e/data_pipeline.spec.ts" \
  || ! rg -q 'expectVisualShellStable' "$ROOT/test/e2e/data_pipeline.spec.ts"; then
  printf 'product UI perf gate failed: large table browser smoke contract is missing\n' >&2
  exit 1
fi

printf 'OK product UI perf gate\n'
