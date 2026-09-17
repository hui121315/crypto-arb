#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
FRONTEND_MANIFEST="$ROOT/frontend/Cargo.toml"
FRONTEND_ROOT="$ROOT/frontend/src/lib.rs"
EXECUTION_VIEW="$ROOT/frontend/src/panels/modules/execution/view.rs"
MODULE_REGISTRY="$ROOT/frontend/src/panels/modules/mod.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BF completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  backup_dir="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-bf.XXXXXX")"
  cp "$FRONTEND_MANIFEST" "$backup_dir/Cargo.toml"
  cp "$FRONTEND_ROOT" "$backup_dir/lib.rs"
  cp "$EXECUTION_VIEW" "$backup_dir/execution_view.rs"
  cp "$MODULE_REGISTRY" "$backup_dir/modules_mod.rs"
  restore() {
    cp "$backup_dir/Cargo.toml" "$FRONTEND_MANIFEST"
    cp "$backup_dir/lib.rs" "$FRONTEND_ROOT"
    cp "$backup_dir/execution_view.rs" "$EXECUTION_VIEW"
    cp "$backup_dir/modules_mod.rs" "$MODULE_REGISTRY"
    rm -rf "$backup_dir"
  }
  trap restore EXIT

  PR_BF_SKIP_TESTS=1 PR_BF_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  perl -0pi -e 's/\A/#![expect(unreachable_pub)]\n/' "$FRONTEND_ROOT"
  if PR_BF_SKIP_TESTS=1 PR_BF_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a frontend visibility lint expectation"
  fi
  cp "$backup_dir/lib.rs" "$FRONTEND_ROOT"

  perl -0pi -e \
    's/pub\(in crate::panels\) fn execution_module/#[component]\npub(in crate::panels) fn execution_module/' \
    "$EXECUTION_VIEW"
  if PR_BF_SKIP_TESTS=1 PR_BF_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a Leptos macro on a module registration wrapper"
  fi
  cp "$backup_dir/execution_view.rs" "$EXECUTION_VIEW"

  perl -0pi -e \
    's/pub\(in crate::panels\) use settings::\{select_credentials_tab, select_risk_tab, settings_module\};/pub(in crate::panels) use settings::{select_risk_tab, settings_module};/' \
    "$MODULE_REGISTRY"
  if PR_BF_SKIP_TESTS=1 PR_BF_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted removal of the Settings credential-tab registration"
  fi
  cp "$backup_dir/modules_mod.rs" "$MODULE_REGISTRY"

  perl -0pi -e 's/^unreachable_pub[^\n]*\n//m' "$FRONTEND_MANIFEST"
  if PR_BF_SKIP_TESTS=1 PR_BF_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted removal of the frontend unreachable_pub lint"
  fi

  printf 'PR-BF completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-BF Module Boundary Gate & Execution Selection Model"
verify_anchor = "`bash scripts/check_pr_bf_completion.sh --self-test`"
evidence = {
    "current-module-boundary": "scripts/check_frontend_module_boundaries.sh",
    "frontend-visibility-lint-contract": "frontend/Cargo.toml",
    "crate-root-zero-lint-expect": "frontend/src/lib.rs",
    "module-registration-boundary": "frontend/src/panels/modules/mod.rs",
    "component-wrapper-migration": "frontend/src/panels/modules/execution/view.rs",
    "execution-selection-model": "frontend/src/panels/modules/execution/selection.rs",
    "workstation-runtime-owner": "frontend/src/panels/workstation.rs",
    "module-boundary-regression": "scripts/check_frontend_module_boundaries_self_test.sh",
    "completion-governance": "scripts/check_pr_bf_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-BF completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate exactly once")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-BF\b", queue):
    fail("completed PR-BF remains in the local queue")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-15 PR-BF Module Boundary Gate & Execution Selection Model Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-BF"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence):
    fail(f"evidence type drift: expected={sorted(evidence)}, actual={sorted(indexed)}")
for kind, artifact in evidence.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence.values():
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

manifest = (root / "frontend/Cargo.toml").read_text(encoding="utf-8")
if not re.search(r"(?m)^unreachable_pub\s*=\s*\"warn\"\s*$", manifest):
    fail("frontend unreachable_pub lint is not enabled")

for source_path in (root / "frontend/src").rglob("*.rs"):
    source = source_path.read_text(encoding="utf-8")
    if re.search(r"(?m)^\s*#!?\[expect\(", source):
        fail(f"frontend lint expectation returned in {source_path.relative_to(root)}")

wrappers = {
    "frontend/src/panels/modules/execution/view.rs": "execution_module",
    "frontend/src/panels/modules/futures/view.rs": "futures_module",
    "frontend/src/panels/modules/opportunities/view.rs": "opportunities_module",
    "frontend/src/panels/modules/positions/view.rs": "positions_module",
    "frontend/src/panels/modules/review/view.rs": "review_module",
    "frontend/src/panels/modules/settings/view.rs": "settings_module",
}
for relative, function in wrappers.items():
    source = (root / relative).read_text(encoding="utf-8")
    signature = rf"pub\(in crate::panels\)\s+fn\s+{function}\b"
    if re.search(signature, source) is None:
        fail(f"{relative} lost plain module wrapper {function}")
    if re.search(rf"#\[component\]\s*{signature}", source):
        fail(f"{relative} put Leptos component generation back on {function}")

module_registry = (root / "frontend/src/panels/modules/mod.rs").read_text(encoding="utf-8")
for marker in (
    "create_execution_runtime, execution_module, ExecutionRuntime, ExecutionSelection",
    "create_futures_runtime, futures_module, FuturesRuntime",
    "create_opportunities_runtime, opportunities_module, OpportunitiesRuntime",
    "create_positions_runtime, positions_module, PositionsRuntime",
    "create_review_runtime, review_module, ReviewRuntime",
    "pub(in crate::panels) use settings::{select_credentials_tab, select_risk_tab, settings_module};",
):
    if marker not in module_registry:
        fail(f"module registration marker drifted: {marker}")

selection = (root / "frontend/src/panels/modules/execution/selection.rs").read_text(
    encoding="utf-8"
)
for marker in (
    "pub(in crate::panels) struct ExecutionSelection",
    "pub(in crate::panels) struct ExecutionSelectionSeed(ExecutionSelection);",
    "fn selection_keeps_execution_fields_only()",
):
    if marker not in selection:
        fail(f"execution selection marker drifted: {marker}")

workstation = (root / "frontend/src/panels/workstation.rs").read_text(encoding="utf-8")
if workstation.count("execution_runtime: create_execution_runtime(),") != 1:
    fail("Workstation must create exactly one canonical ExecutionRuntime")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo_gate.count("check_pr_bf_completion.sh") < 2:
    fail("repo gate must run the PR-BF completion contract in docs and all scopes")

print(f"OK PR-BF static completion contract ({len(evidence)} evidence types)")
PY

bash -n "$0"
bash "$ROOT/scripts/check_frontend_module_boundaries.sh"
bash "$ROOT/scripts/check_frontend_module_boundaries_self_test.sh"
bash "$ROOT/scripts/check_allow_debt.sh"

if [[ "${PR_BF_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" \
    cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" \
    cargo clippy --manifest-path "$ROOT/frontend/Cargo.toml" \
      --target wasm32-unknown-unknown --all-targets -- \
      -D warnings -W unreachable-pub -W private-interfaces
fi

if [[ "${PR_BF_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

printf 'PR-BF completion gate passed\n'
