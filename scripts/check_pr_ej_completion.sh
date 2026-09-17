#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
BUDGET="$ROOT/scripts/check_wasm_budget.sh"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-EJ completion gate failed: %s\n' "$1" >&2
  exit 1
}

must_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  rg -n -- "$pattern" "$@" >/dev/null || fail "$label"
}

require_evidence() {
  local evidence_type="$1"
  awk -F '\t' -v evidence_type="$evidence_type" '
    $1 == "PR-EJ" && $2 == evidence_type { found = 1 }
    END { exit(found ? 0 : 1) }
  ' "$EVIDENCE" || fail "missing PR-EJ evidence type $evidence_type"
}

if [[ "$MODE" == "--self-test" ]]; then
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ej-wasm.XXXXXX")"
  cp "$BUDGET" "$backup"
  restore() {
    cp "$backup" "$BUDGET"
    rm -f "$backup"
  }
  trap restore EXIT

  perl -0pi -e 's/^GZIP_BUDGET_BYTES=/REMOVED_GZIP_BUDGET_BYTES=/m' "$BUDGET"
  if PR_EJ_SKIP_DESTRUCTIVE=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test unexpectedly accepted a missing gzip budget"
  fi
  printf 'PR-EJ completion self-test passed\n'
  exit 0
fi

must_match "docs scope does not run refinement truth" \
  'check_refinement_status_drift\.sh' "$REPO_GATE"
must_match "docs scope does not run actionable status governance" \
  'check_actionable_status_markers\.sh' "$REPO_GATE"
must_match "repo gate does not run the product boundary" \
  'product_copy_gate\.sh' "$REPO_GATE"
must_match "repo gate does not run external API evidence" \
  'check_exchange_evidence_debt\.sh' "$REPO_GATE"
must_match "repo gate does not run frontend DTO single-source checks" \
  'check_frontend_dto_mirror\.sh' "$REPO_GATE"
must_match "repo gate does not run unified Wasm budget contract" \
  'check_wasm_budget_contract\.sh' "$REPO_GATE"
must_match "repo gate does not run PR-EH problem/load-state completion" \
  'check_pr_eh_completion\.sh' "$REPO_GATE"
must_match "repo gate does not run PR-EI audit/idempotency completion" \
  'check_pr_ei_completion\.sh' "$REPO_GATE"

for evidence_type in \
  authority-refinement-truth \
  actionable-status-governance \
  product-boundary \
  external-api-evidence \
  problem-loadstate-action-boundary \
  high-risk-audit-idempotency \
  dto-single-source \
  wasm-three-stage-budget \
  completion-governance; do
  require_evidence "$evidence_type"
done

roadmap_row="$(rg -F '| `PR-EJ Documentation Authority & Verification Gate Contract`' "$DOC" || true)"
[[ "$roadmap_row" == *"✅ 完成"* ]] || fail "roadmap row is not completed"
[[ "$roadmap_row" == *"剩余：无。"* ]] || fail "roadmap row still reports a remainder"
queue_section="$(sed -n '/^### 🟡 6\.5/,/^## /p' "$DOC")"
[[ "$queue_section" != *'**PR-EJ Documentation Authority'* ]] || \
  fail "completed PR-EJ remains in the queue"

bash "$ROOT/scripts/check_authority_docs.sh"
bash "$ROOT/scripts/check_refinement_status_drift.sh"
bash "$ROOT/scripts/check_actionable_status_markers.sh"
bash "$ROOT/scripts/product_copy_gate.sh"
bash "$ROOT/scripts/check_route_inventory.sh"
bash "$ROOT/scripts/check_exchange_evidence_debt.sh"
bash "$ROOT/scripts/check_exchange_operation_evidence_matrix.sh"
bash "$ROOT/scripts/check_frontend_module_boundaries.sh"
bash "$ROOT/scripts/check_frontend_dto_mirror.sh"
PR_EH_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eh_completion.sh"
PR_EI_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ei_completion.sh"
bash "$ROOT/scripts/check_mutation_audit_contract.sh"
bash "$ROOT/scripts/check_wasm_budget_contract.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"

if [[ "${PR_EJ_SKIP_DESTRUCTIVE:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_refinement_status_drift.sh" --self-test
  bash "$ROOT/scripts/check_frontend_module_boundaries_self_test.sh"
  bash "$ROOT/scripts/check_exchange_evidence_debt.sh" --self-test
  bash "$ROOT/scripts/check_release_qa_contract.sh" --self-test
fi

printf 'OK PR-EJ documentation authority and verification completion contract\n'
