#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
MODE_DTO="$ROOT/shared-types/src/live_trading.rs"
MODE_LABELS="$ROOT/frontend/src/panels/shared/execution_environment.rs"
SETTINGS="$ROOT/frontend/src/panels/modules/settings/view.rs"
ADAPTERS="$ROOT/frontend/src/panels/modules/settings/tabs/adapters.rs"
DIAGNOSTICS="$ROOT/frontend/src/panels/modules/settings/tabs/diagnostics.rs"
PREFLIGHT="$ROOT/frontend/src/panels/modules/execution/components/risk_preview/preflight.rs"
RISK_TESTS="$ROOT/frontend/src/panels/modules/execution/components/risk_preview/tests.rs"
E2E="$ROOT/test/e2e/data_pipeline.spec.ts"
MOCK="$ROOT/test/e2e/mock_api.mjs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-FU completion gate failed: %s\n' "$1" >&2
  exit 1
}

must_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  if ! rg -n -- "$pattern" "$@" >/dev/null; then
    fail "$label"
  fi
}

must_not_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  if rg -n -- "$pattern" "$@" >/dev/null; then
    fail "$label"
  fi
}

require_evidence() {
  local evidence_type="$1"
  if ! awk -F '\t' -v evidence_type="$evidence_type" '
    $1 == "PR-FU" && $2 == evidence_type { found = 1 }
    END { exit(found ? 0 : 1) }
  ' "$EVIDENCE"; then
    fail "missing PR-FU evidence type $evidence_type"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  labels_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-fu-labels.XXXXXX")"
  cp "$MODE_LABELS" "$labels_backup"
  restore() {
    cp "$labels_backup" "$MODE_LABELS"
    rm -f "$labels_backup"
  }
  trap restore EXIT

  perl -0pi -e 's/"模拟"/"Paper"/' "$MODE_LABELS"
  if bash "$0"; then
    fail "self-test unexpectedly accepted a compatibility mode label in product UI"
  fi

  printf 'PR-FU completion self-test passed\n'
  exit 0
fi

must_match \
  "shared execution modes no longer project to product environments" \
  'pub const fn environment\(self\) -> ExecutionEnvironment' \
  "$MODE_DTO"
must_match \
  "compatibility modes no longer collapse into Paper" \
  'Self::DryRun \| Self::Testnet => ExecutionEnvironment::Paper' \
  "$MODE_DTO"
must_match \
  "shared frontend mode labels are missing" \
  'execution_mode_label\(mode: ExecutionMode\)' \
  "$MODE_LABELS"
must_not_match \
  "shared frontend labels leak Paper/Live compatibility vocabulary" \
  '"Paper"|"Live"' \
  "$MODE_LABELS"
must_match \
  "settings did not migrate the legacy adapters slug to diagnostics" \
  '"adapters" => Self::Diagnostics' \
  "$SETTINGS"
must_not_match \
  "settings reintroduced a global adapter selection tab" \
  'Adapters|adapters_tab|"执行模式"' \
  "$SETTINGS"
must_match \
  "settings diagnostics no longer render the read-only environment panel" \
  'execution_environment_panel' \
  "$DIAGNOSTICS"
must_match \
  "environment panel no longer states the ticket-scoped submit boundary" \
  'HedgeTicket.*双腿预检' \
  "$ADAPTERS"
must_match \
  "environment and venue tables no longer have stable browser identities" \
  'data-settings-table="execution-environment"|data-settings-table="venue-capabilities"' \
  "$ADAPTERS"
must_match \
  "risk preview no longer exposes ticket-scoped leg availability" \
  'ticket_venue_availability_(summary|detail)|live_operation_health' \
  "$PREFLIGHT"
must_match \
  "ticket-scoped availability regression test is missing" \
  'ticket_venue_availability_is_scoped_to_live_ticket_preflight' \
  "$RISK_TESTS"
must_match \
  "opportunity product naming did not close" \
  '候选机会|搜索、筛选并解释可执行候选' \
  "$ROOT/frontend/src/panels/modules/opportunities/view.rs"
must_match \
  "futures product naming did not close" \
  '费率、期现与跨市场价差策略统一比较' \
  "$ROOT/frontend/src/panels/modules/futures/view.rs"
must_match \
  "PR-FU mock fixture is missing" \
  'e2e-pr-fu-settings-environment' \
  "$MOCK" \
  "$E2E"
must_match \
  "PR-FU browser proof is missing" \
  'settings credentials keep static adapter copy separate from runtime readiness' \
  "$E2E"
must_match \
  "execution card CSS split is missing from the manifest" \
  'skin/execution-cards\.css' \
  "$ROOT/frontend/styles/src/manifest.txt"
must_match \
  "surface card CSS split is missing from the manifest" \
  'skin/surface-cards\.css' \
  "$ROOT/frontend/styles/src/manifest.txt"
must_not_match \
  "execution card styles leaked back into the base execution skin" \
  'execution-confidence|execution-run-state|selected-summary' \
  "$ROOT/frontend/styles/src/skin/execution.css"

roadmap_row="$(rg -F '| `PR-FU Frontend Product Language, Mode Semantics & Visual Integrity Contract`' "$DOC" || true)"
[[ "$roadmap_row" == *"✅ 完成"* ]] || fail "roadmap row is not completed"
[[ "$roadmap_row" == *"剩余：无。"* ]] || fail "roadmap row still reports a remainder"
[[ "$roadmap_row" == *"check_pr_fu_completion.sh"* ]] || fail "roadmap row lacks the completion gate"
if rg -q '^1\. \*\*PR-FU ' "$DOC"; then
  fail "completed PR-FU remains at the queue head"
fi

for evidence_type in \
  execution-environment-projection \
  settings-environment-diagnostics \
  ticket-scoped-availability \
  product-naming-mock-browser \
  css-module-split \
  completion-governance-gate; do
  require_evidence "$evidence_type"
done

if [[ "${PR_FU_SKIP_TESTS:-0}" != "1" ]]; then
  cargo test -p shared-types --lib execution_modes_project_to_only_two_product_environments --no-fail-fast
  cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib execution_mode_and_environment_labels_hide_compatibility_variants --no-fail-fast
  cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib ticket_venue_availability_is_scoped_to_live_ticket_preflight --no-fail-fast
  cargo test -p api --bin crypto-arb-api live_operation_health_guard_scopes_all_evidence_to_ticket_venues --no-fail-fast
  bash "$ROOT/scripts/check_frontend_css_generated.sh"
  bash "$ROOT/scripts/product_copy_gate.sh"
  CI=1 npx playwright test "$E2E" --grep 'settings credentials keep static adapter copy separate from runtime readiness' --reporter=list
fi

printf 'OK PR-FU product language and mode completion gate\n'
