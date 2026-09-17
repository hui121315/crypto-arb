#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="${TMPDIR:-/tmp}/crossline-repo-gates.$$"
trap 'rm -f "$TMP"' EXIT

fail_if_matches() {
  local label="$1"
  local pattern="$2"
  shift 2
  if rg -n "$pattern" "$@" >"$TMP"; then
    printf 'repo gate failed: %s\n' "$label" >&2
    cat "$TMP" >&2
    exit 1
  fi
}

run_docs() {
  bash "$ROOT/scripts/check_authority_docs.sh"
  bash "$ROOT/scripts/check_doc_path_references.sh"
}

run_architecture() {
  fail_if_matches \
    "frontend must not redefine shared DTOs" \
    'struct\s+(PortfolioSummary|PositionRow|RiskSnapshot|SystemHealth|VenueQuality|StrategyPerformance|ExecutedTrade|MissedOpportunity)\b' \
    "$ROOT/frontend/src" --type rust
  fail_if_matches \
    "shared state must not use forbidden locks" \
    'RwLock<HashMap|RwLock<Snapshot|std::sync::Mutex' \
    "$ROOT/crates" --type rust
  fail_if_matches \
    "frontend must not add chart or icon dependencies" \
    '"(d3|echarts|chart\.js|lightweight-charts|highcharts|lucide|tabler|charming|plotly)"' \
    "$ROOT/frontend/Cargo.toml"
  fail_if_matches \
    "skin and base CSS must use design tokens" \
    '#[0-9A-Fa-f]{6}' \
    "$ROOT/frontend/styles/src" --glob '!**/legacy/**' --glob '!**/tokens.css'
  fail_if_matches \
    "API routers must not unwrap or expect" \
    '\.unwrap\(\)|\.expect\(' \
    "$ROOT/crates/api/src/routers" --type rust

  bash "$ROOT/scripts/check_crate_root_boundaries.sh"
  bash "$ROOT/scripts/check_frontend_module_boundaries.sh"
  bash "$ROOT/scripts/check_frontend_dto_mirror.sh"
  bash "$ROOT/scripts/check_module_size.sh"
  bash "$ROOT/scripts/check_allow_debt.sh"
  bash "$ROOT/scripts/check_panic_result_debt.sh"
}

run_product_contracts() {
  bash "$ROOT/scripts/check_route_inventory.sh"
  bash "$ROOT/scripts/check_route_endpoint_metadata.sh"
  bash "$ROOT/scripts/check_route_runtime_policy.sh"
  bash "$ROOT/scripts/check_mutation_audit_contract.sh"
  bash "$ROOT/scripts/check_exchange_evidence_debt.sh"
  bash "$ROOT/scripts/check_exchange_operation_evidence_matrix.sh"
  PRODUCT_EXTENSIONS_SKIP_TESTS=1 \
    bash "$ROOT/scripts/check_product_extensions_completion.sh"
  bash "$ROOT/scripts/check_release_qa_contract.sh"
}

run_ui_contracts() {
  bash "$ROOT/scripts/product_copy_gate.sh"
  bash "$ROOT/scripts/build_frontend_css.sh" >/dev/null
  bash "$ROOT/scripts/check_frontend_css_generated.sh"
  bash "$ROOT/scripts/product_ui_perf_gate.sh"
}

case "${VERIFY_REPO_GATES_SCOPE:-all}" in
  docs)
    run_docs
    printf 'OK current documentation gates\n'
    ;;
  debt)
    run_architecture
    printf 'OK current architecture and debt gates\n'
    ;;
  all|"")
    run_docs
    run_architecture
    run_product_contracts
    run_ui_contracts
    printf 'OK current repository static gates\n'
    ;;
  *)
    printf 'repo gate failed: unknown VERIFY_REPO_GATES_SCOPE=%s\n' \
      "${VERIFY_REPO_GATES_SCOPE}" >&2
    exit 2
    ;;
esac
