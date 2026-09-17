#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
HISTORY="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
COVERAGE="$ROOT/docs/PRODUCT_AUDIT_COVERAGE.tsv"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"
TRACE_GATE="$ROOT/scripts/check_active_roadmap_traceability.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-CL completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  backup_dir="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-cl.XXXXXX")"
  cp "$REPO_GATE" "$backup_dir/verify_repo_gates.sh"
  cp "$EVIDENCE" "$backup_dir/PRODUCT_AUDIT_EVIDENCE.tsv"
  restore() {
    cp "$backup_dir/verify_repo_gates.sh" "$REPO_GATE"
    cp "$backup_dir/PRODUCT_AUDIT_EVIDENCE.tsv" "$EVIDENCE"
    rm -rf "$backup_dir"
  }
  trap restore EXIT

  PR_CL_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  perl -0pi -e 's/check_active_roadmap_traceability\.sh/check_detached_traceability.sh/g' \
    "$REPO_GATE"
  if PR_CL_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a detached roadmap traceability gate"
  fi
  cp "$backup_dir/verify_repo_gates.sh" "$REPO_GATE"

  awk -F '\t' '$1 != "PR-AA"' "$EVIDENCE" >"$EVIDENCE.tmp"
  mv "$EVIDENCE.tmp" "$EVIDENCE"
  if PR_CL_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted an active partial row without evidence"
  fi
  cp "$backup_dir/PRODUCT_AUDIT_EVIDENCE.tsv" "$EVIDENCE"

  if ! bash "$ROOT/scripts/check_refinement_status_drift.sh" --self-test >/dev/null; then
    fail "refinement status drift fixtures failed"
  fi

  printf 'PR-CL completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" "$DOC" "$HISTORY" "$EVIDENCE" "$COVERAGE" "$REPO_GATE" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
doc_path = Path(sys.argv[2])
history_path = Path(sys.argv[3])
evidence_path = Path(sys.argv[4])
coverage_path = Path(sys.argv[5])
repo_gate_path = Path(sys.argv[6])
title = "PR-CL Documentation Authority & Completion Evidence Governance"
verify_anchor = "`bash scripts/check_pr_cl_completion.sh --self-test`"
required = {
    "authority-refinement-truth": "scripts/check_refinement_status_drift.sh",
    "actionable-status-governance": "scripts/check_actionable_status_markers.sh",
    "active-roadmap-traceability": "scripts/check_active_roadmap_traceability.sh",
    "completion-evidence-index": "scripts/check_product_audit_evidence_index.sh",
    "runtime-artifact-boundary": "scripts/verify_repo_gates.sh",
    "completion-governance": "scripts/check_pr_cl_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-CL completion gate failed: {message}")


doc = doc_path.read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-CL\b", queue):
    fail("completed PR-CL remains in the local queue")

history = history_path.read_text(encoding="utf-8")
if "## 2026-07-15 PR-CL Documentation Authority & Completion Evidence Governance Closure" not in history:
    fail("history closure appendix is missing")

with evidence_path.open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-CL"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(required):
    fail(f"evidence type drift: expected={sorted(required)}, actual={sorted(indexed)}")
for kind, artifact in required.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

with coverage_path.open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in required.values():
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

repo_gate = repo_gate_path.read_text(encoding="utf-8")
for marker, minimum in (
    ("check_refinement_status_drift.sh", 2),
    ("check_actionable_status_markers.sh", 2),
    ("check_product_audit_evidence_index.sh", 2),
    ("check_active_roadmap_traceability.sh", 2),
    ("check_pr_cl_completion.sh", 2),
    ("fail_if_tracked_runtime_artifacts", 2),
):
    if repo_gate.count(marker) < minimum:
        fail(f"repo gate lost required marker: {marker}")

print(f"OK PR-CL static completion contract ({len(required)} evidence types)")
PY

tracked_docs=(
  AGENTS.md
  README.md
  docs/CROSSLINE_IMPLEMENTATION_GUIDE.md
  docs/PRODUCT_FULL_AUDIT_REFINEMENT.md
  docs/PRODUCT_AUDIT_EVIDENCE.tsv
  docs/PRODUCT_AUDIT_COVERAGE.tsv
  docs/API_ROUTE_INVENTORY.tsv
  docs/audit_history/PRODUCT_AUDIT_HISTORY.md
  docs/PR_M_LIVE_ORDER_ACCEPTANCE_RUNBOOK.md
  scripts/check_active_roadmap_traceability.sh
  scripts/check_pr_cl_completion.sh
)
for path in "${tracked_docs[@]}"; do
  git -C "$ROOT" ls-files --error-unmatch "$path" >/dev/null 2>&1 || \
    fail "required authority artifact is not tracked: $path"
done

if git -C "$ROOT" ls-files \
  -- '*.sqlite' '*.sqlite-*' '*.sqlite3' '*.db' '*.db-*' '*.db3' '*.jsonl' \
  | rg -v '(^|/)fixtures/' >/dev/null; then
  fail "tracked runtime data artifact escaped the fixture boundary"
fi

bash "$TRACE_GATE"

if [[ "${PR_CL_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_authority_docs.sh"
  bash "$ROOT/scripts/check_refinement_status_drift.sh"
  bash "$ROOT/scripts/check_actionable_status_markers.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

printf 'PR-CL completion gate passed\n'
