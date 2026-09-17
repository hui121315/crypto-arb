#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

required=(
  "AGENTS.md"
  "README.md"
  "docs/CROSSLINE_IMPLEMENTATION_GUIDE.md"
  "docs/API_ROUTE_INVENTORY.tsv"
  "docs/PR_M_LIVE_ORDER_ACCEPTANCE_RUNBOOK.md"
)

retired=(
  "docs/CROSSLINE_PRODUCT_REFINEMENT.md"
  "docs/CROSSLINE_P0_TASKS.md"
  "docs/CROSSLINE_OMNI_PRD.md"
  "docs/PoC/README.md"
  "docs/PoC/PoC-001-black-scholes-vs-scipy.md"
  "docs/PoC/PoC-001-fixtures.json"
  "docs/PoC/PoC-001-generate-fixtures.py"
  "docs/live_samples/README.md"
  "docs/ARBITRAGE_PROFITABILITY_REFINEMENT.md"
  "docs/ARBITRAGE_SCAN_REFINEMENT.md"
  "docs/EXCHANGE_DATA_PIPELINE_REFINEMENT.md"
  "docs/EXECUTION_HEDGE_REFINEMENT.md"
  "docs/POSITIONS_RISK_REFINEMENT.md"
  "docs/SETTINGS_REFINEMENT.md"
  "docs/audit_doc_optimization_checklist.md"
  "docs/audit_history/EXCHANGE_DATA_PIPELINE_HISTORY.md"
  "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
  "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
  "docs/PRODUCT_AUDIT_COVERAGE.tsv"
  "docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
  "docs/milestones/M1-foundation.md"
  "docs/milestones/M2-exchange-foundation.md"
  "docs/milestones/M3-exchanges.md"
  "docs/milestones/V2-live-trading.md"
)

for file in "${required[@]}"; do
  if [ ! -s "$ROOT/$file" ]; then
    printf 'authority doc gate failed: missing or empty %s\n' "$file" >&2
    exit 1
  fi
done

rg -q '## 5\. Active Scope' "$ROOT/docs/CROSSLINE_IMPLEMENTATION_GUIDE.md" || {
  printf 'authority doc gate failed: implementation guide must provide executable active scope\n' >&2
  exit 1
}

for file in AGENTS.md DESIGN.md README.md docs/CROSSLINE_IMPLEMENTATION_GUIDE.md; do
  if rg -q 'PRODUCT_FULL_AUDIT_REFINEMENT|PRODUCT_AUDIT_(EVIDENCE|COVERAGE)' "$ROOT/$file"; then
    printf 'authority doc gate failed: active authority still depends on retired audit: %s\n' "$file" >&2
    exit 1
  fi
done

for file in "${retired[@]}"; do
  if [ -e "$ROOT/$file" ]; then
    printf 'authority doc gate failed: retired snapshot returned: %s\n' "$file" >&2
    exit 1
  fi
done

printf 'OK authority docs gate\n'
