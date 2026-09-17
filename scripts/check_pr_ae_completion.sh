#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
ENCODING="$ROOT/frontend/src/api/rest/encoding.rs"
TRADING="$ROOT/frontend/src/api/rest/trading.rs"
BROWSER="$ROOT/test/e2e/pr_dl_transport_runtime.spec.ts"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AE completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  backup_dir="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-ae.XXXXXX")"
  for file in "$DOC" "$EVIDENCE" "$ENCODING" "$TRADING" "$BROWSER"; do
    cp "$file" "$backup_dir/$(basename "$file")"
  done
  restore() {
    cp "$backup_dir/$(basename "$DOC")" "$DOC"
    cp "$backup_dir/$(basename "$EVIDENCE")" "$EVIDENCE"
    cp "$backup_dir/$(basename "$ENCODING")" "$ENCODING"
    cp "$backup_dir/$(basename "$TRADING")" "$TRADING"
    cp "$backup_dir/$(basename "$BROWSER")" "$BROWSER"
  }
  cleanup() {
    restore
    rm -rf "$backup_dir"
  }
  trap cleanup EXIT

  PR_AE_SKIP_TESTS=1 PR_AE_SKIP_UPSTREAM=1 PR_AE_SKIP_BROWSER_LIST=1 bash "$0" >/dev/null

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-AE Frontend API Client & Contract Runtime` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-AE self-test setup failed: roadmap row drifted")
path.write_text(source.replace(marker, marker.replace("✅ 完成", "🟡 部分完成"), 1), encoding="utf-8")
PY
  if PR_AE_SKIP_TESTS=1 PR_AE_SKIP_UPSTREAM=1 PR_AE_SKIP_BROWSER_LIST=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a downgraded roadmap row"
  fi
  restore

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-AE\tpath-component-encoding\t"):
        rows.pop(index)
        path.write_text("\n".join(rows) + "\n", encoding="utf-8")
        break
else:
    raise SystemExit("PR-AE self-test setup failed: path evidence row missing")
PY
  if PR_AE_SKIP_TESTS=1 PR_AE_SKIP_UPSTREAM=1 PR_AE_SKIP_BROWSER_LIST=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted an incomplete evidence matrix"
  fi
  restore

  python3 - "$TRADING" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    let id = encode_path_segment(id);\n"
if source.count(marker) != 2:
    raise SystemExit("PR-AE self-test setup failed: trading path marker drifted")
path.write_text(source.replace(marker, "    let id = id.to_owned();\n", 1), encoding="utf-8")
PY
  if PR_AE_SKIP_TESTS=1 PR_AE_SKIP_UPSTREAM=1 PR_AE_SKIP_BROWSER_LIST=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a raw trading path identifier"
  fi
  restore

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'test("PR-DL websocket subscribe correlates browser request id before marking connected"'
if source.count(marker) != 1:
    raise SystemExit("PR-AE self-test setup failed: browser marker drifted")
path.write_text(source.replace(marker, marker.replace("test(", "test.skip("), 1), encoding="utf-8")
PY
  if PR_AE_SKIP_TESTS=1 PR_AE_SKIP_UPSTREAM=1 PR_AE_SKIP_BROWSER_LIST=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped browser authority"
  fi
  restore

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "### 🟡 6.5 下一步执行队列"
if source.count(marker) != 1:
    raise SystemExit("PR-AE self-test setup failed: queue heading drifted")
stale = "\n\n1. **PR-AE Frontend API Client & Contract Runtime** — stale completed row"
path.write_text(source.replace(marker, marker + stale, 1), encoding="utf-8")
PY
  if PR_AE_SKIP_TESTS=1 PR_AE_SKIP_UPSTREAM=1 PR_AE_SKIP_BROWSER_LIST=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted completed PR-AE at the queue head"
  fi
  restore

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-AF Frontend Opportunity View Model Semantics` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-AE self-test setup failed: PR-AF roadmap row drifted")
path.write_text(source.replace(marker, marker.replace("✅ 完成", "🟡 部分完成"), 1), encoding="utf-8")
PY
  if PR_AE_SKIP_TESTS=1 PR_AE_SKIP_UPSTREAM=1 PR_AE_SKIP_BROWSER_LIST=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted unfinished PR-AF outside the queue head"
  fi
  restore

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "### 🟡 6.5 下一步执行队列"
if source.count(marker) != 1:
    raise SystemExit("PR-AE self-test setup failed: queue heading drifted")
stale = "\n\n1. **PR-AF Frontend Opportunity View Model Semantics** — stale completed row"
path.write_text(source.replace(marker, marker + stale, 1), encoding="utf-8")
PY
  if PR_AE_SKIP_TESTS=1 PR_AE_SKIP_UPSTREAM=1 PR_AE_SKIP_BROWSER_LIST=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted completed PR-AF in the local queue"
  fi

  printf 'PR-AE completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-AE Frontend API Client & Contract Runtime"
verify_anchor = "`bash scripts/check_pr_ae_completion.sh --self-test`"
authorities = (
    "PR-BY Frontend ApiProblem & LoadState Contract",
    "PR-DL Frontend Transport Runtime & ApiProblem Contract",
    "PR-DT ApiProblem & Request Correlation Contract",
    "PR-EX Frontend LoadState, ActionState & Envelope Runtime",
    "PR-CN Frontend Workstation State & Navigation Runtime Contract",
)
evidence_contract = {
    "app-runtime-context": ("frontend/src/state/context.rs", "api::base"),
    "api-base-auth-source": ("frontend/src/api/base.rs", "api::base"),
    "rest-reactive-runtime": ("frontend/src/api/rest.rs", "api::rest"),
    "ws-reactive-runtime": ("frontend/src/api/ws_runtime.rs", "api::ws_runtime"),
    "shared-request-id": ("frontend/src/api/request_id.rs", "api::request_id"),
    "rest-request-context": ("frontend/src/api/rest/transport.rs", "api::rest"),
    "shared-load-state": ("frontend/src/state/load_state.rs", "load_state"),
    "shared-action-state": ("shared-types/src/actions/state.rs", "action_state"),
    "dto-single-source": ("scripts/check_frontend_dto_mirror.sh", "check_frontend_dto_mirror.sh"),
    "path-component-encoding": ("frontend/src/api/rest/encoding.rs", "api::rest"),
    "arbitrage-path-matrix": ("frontend/src/api/rest/arbitrage/paths.rs", "api::rest"),
    "trading-path-matrix": ("frontend/src/api/rest/trading.rs", "api::rest"),
    "portfolio-path-matrix": ("frontend/src/api/rest/portfolio_system/paths.rs", "api::rest"),
    "query-component-matrix": ("frontend/src/api/rest/portfolio_system/envelope.rs", "api::rest"),
    "api-problem-authority": ("scripts/check_pr_by_completion.sh", "check_pr_by_completion.sh"),
    "transport-authority": ("scripts/check_pr_dl_completion.sh", "check_pr_dl_completion.sh"),
    "correlation-authority": ("scripts/check_pr_dt_completion.sh", "check_pr_dt_completion.sh"),
    "workspace-authority": ("scripts/check_pr_cn_completion.sh", "check_pr_cn_completion.sh"),
    "transport-browser": ("test/e2e/pr_dl_transport_runtime.spec.ts", "test:e2e:pr-dl"),
    "load-state-browser": ("test/e2e/pr_ft_load_state.spec.ts", "test:e2e:pr-ft"),
    "workspace-browser": ("test/e2e/pr_cn_workspace_runtime.spec.ts", "test:e2e:pr-cn"),
    "repo-gate-wiring": ("scripts/verify_repo_gates.sh", "verify_repo_gates.sh"),
    "completion-governance": ("scripts/check_pr_ae_completion.sh", "check_pr_ae_completion.sh --self-test"),
}
source_markers = {
    "frontend/src/state/context.rs": ("provide_ws_runtime", "with_base_signal_and_auth"),
    "frontend/src/api/rest.rs": ("mod encoding;", "encode_path_segment", "encode_query_component"),
    "frontend/src/api/ws_runtime.rs": ("api_auth_token: RwSignal<String>", "PendingSubscribe", '"requestId": request_id'),
    "frontend/src/api/rest/encoding.rs": ("fn encode_component", "component_encoding_preserves_only_unreserved_bytes"),
    "frontend/src/api/rest/arbitrage/paths.rs": ("hedge_preview_path", "hedge_confirm_path", "orderbook_path"),
    "frontend/src/api/rest/trading.rs": ("fn action_run_path", "fn cancel_order_path", "let id = encode_path_segment(id);"),
    "frontend/src/api/rest/portfolio_system.rs": ("mod paths;", "close_position_path", "close_run_compensation_path"),
    "frontend/src/api/rest/portfolio_system/paths.rs": ("close_position_path", "close_run_compensation_path", "encode_path_segment"),
    "scripts/check_frontend_dto_mirror.sh": ("ApiError", "MutationRequestContext", "only ApiError and MutationRequestContext"),
}
browser_contract = {
    "test/e2e/pr_dl_transport_runtime.spec.ts": (
        "PR-DL websocket subscribe correlates browser request id before marking connected",
        "PR-DL mismatched websocket ack stays typed without hiding REST fallback rows",
    ),
    "test/e2e/pr_ft_load_state.spec.ts": (
        "PR-FT opportunities cold error remains typed and leaves no fake opportunity",
        "PR-FT review cold error remains typed and leaves no fake execution record",
        "PR-FT settings credentials cold error remains typed and hides static success panels",
        "PR-FT execution preview cold error remains typed and blocks submit",
    ),
    "test/e2e/pr_cn_workspace_runtime.spec.ts": (
        "PR-CN restores symbol strategy and cursor deep link across module switches",
        "PR-CN routes explicit opportunity and run context into the execution query",
        "PR-CN preserves execution draft pending evidence and outcome across unmount",
        "PR-CN exposes one typed problem inline in navigation and global toast",
    ),
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-AE completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
start = doc.find("### 🟡 6.3")
end = doc.find("### 🟡 6.4", start)
if start < 0 or end < 0:
    fail("bounded roadmap section is missing")
roadmap = doc[start:end]


def roadmap_row(row_title: str) -> str:
    rows = [line for line in roadmap.splitlines() if line.startswith(f"| `{row_title}`")]
    if len(rows) != 1:
        fail(f"expected one roadmap row for {row_title}")
    return rows[0]


row = roadmap_row(title)
if "✅ 完成" not in row or "剩余：无" not in row or row.count(verify_anchor) != 1:
    fail("roadmap row must be complete, remaining-none and bound to the destructive gate")
for authority in authorities:
    if "✅ 完成" not in roadmap_row(authority):
        fail(f"successor authority is not complete: {authority}")

queue = doc[doc.find("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-AE\b", queue):
    fail("completed PR-AE remains in the local queue")
pr_af_complete = "✅ 完成" in roadmap_row("PR-AF Frontend Opportunity View Model Semantics")
pr_af_queued = re.search(r"(?m)^\d+\.\s+\*\*PR-AF\b", queue) is not None
pr_af_is_head = re.search(r"(?m)^1\.\s+\*\*PR-AF\b", queue) is not None
if pr_af_complete and pr_af_queued:
    fail("completed PR-AF remains in the local queue")
if not pr_af_complete and not pr_af_is_head:
    fail("unfinished PR-AF must be the local queue head")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-17 PR-AE Frontend API Client and Contract Runtime Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-AE"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, (artifact, command_anchor) in evidence_contract.items():
    evidence = indexed[kind]
    if evidence["artifact"] != artifact or command_anchor not in evidence["command"]:
        fail(f"evidence anchor drifted: {kind}")
    if not (root / artifact).is_file():
        fail(f"evidence artifact is missing: {artifact}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact, _ in evidence_contract.values():
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

for relative, markers in source_markers.items():
    source = (root / relative).read_text(encoding="utf-8")
    for marker in markers:
        if marker not in source:
            fail(f"source marker missing: {relative}:{marker}")

arbitrage = (root / "frontend/src/api/rest/arbitrage.rs").read_text(encoding="utf-8")
portfolio_paths = (root / "frontend/src/api/rest/portfolio_system/paths.rs").read_text(encoding="utf-8")
trading = (root / "frontend/src/api/rest/trading.rs").read_text(encoding="utf-8")
if "request.opportunity_id\n" in arbitrage or "{opportunity_id}/confirm" in arbitrage:
    fail("arbitrage client reintroduced raw opportunity path interpolation")
if "let venue = encode_query_component(venue);" in portfolio_paths or "let close_run_id = encode_query_component(close_run_id);" in portfolio_paths:
    fail("portfolio client uses query encoding for a path segment")
if trading.count("let id = encode_path_segment(id);") != 2:
    fail("trading identifiers must pass through both path-segment encoders")

for relative, titles in browser_contract.items():
    source = (root / relative).read_text(encoding="utf-8")
    for title in titles:
        escaped = re.escape(title)
        if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', source):
            fail(f"browser anchor is skipped: {title}")
        if not re.search(rf'test\(\s*["\']{escaped}["\']', source):
            fail(f"browser anchor is missing: {title}")

product = json.loads((root / "package.json").read_text(encoding="utf-8"))
release = json.loads((root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8"))
for suffix, path in (
    ("dl", "test/e2e/pr_dl_transport_runtime.spec.ts"),
    ("ft", "test/e2e/pr_ft_load_state.spec.ts"),
    ("cn", "test/e2e/pr_cn_workspace_runtime.spec.ts"),
):
    expected = f"playwright test {path}"
    if product.get("scripts", {}).get(f"test:e2e:pr-{suffix}") != expected:
        fail(f"dedicated PR-{suffix.upper()} browser command drifted")
    for name, package in (("product", product), ("release", release)):
        if package.get("scripts", {}).get("test:e2e:product", "").count(path) != 1:
            fail(f"{name} product suite must include {path} exactly once")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo_gate.count("check_pr_ae_completion.sh") != 2:
    fail("repo gate must execute PR-AE exactly once in docs and all scopes")

print(
    f"OK PR-AE static contract ({len(evidence_contract)} evidence types; "
    f"{sum(map(len, browser_contract.values()))} browser anchors)"
)
PY

if [[ "${PR_AE_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test \
    "$ROOT/test/e2e/pr_dl_transport_runtime.spec.ts" \
    "$ROOT/test/e2e/pr_ft_load_state.spec.ts" \
    "$ROOT/test/e2e/pr_cn_workspace_runtime.spec.ts" \
    --list >/dev/null
fi

if [[ "${PR_AE_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_BY_SKIP_TESTS=1 PR_BY_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_by_completion.sh"
  PR_DL_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dl_completion.sh"
  PR_DT_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dt_completion.sh"
  PR_CN_SKIP_TESTS=1 PR_CN_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_cn_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
  bash "$ROOT/scripts/check_frontend_dto_mirror.sh"
  bash "$ROOT/scripts/check_release_qa_contract.sh"
fi

if [[ "${PR_AE_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    --lib api::rest --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    --lib api::base --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    --lib api::ws_runtime --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    --lib load_state --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dl -- --workers=1
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ft -- --workers=1
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-cn -- --workers=1
fi

printf 'PR-AE frontend API client and contract runtime completion passed\n'
