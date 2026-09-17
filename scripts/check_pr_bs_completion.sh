#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
EMPTY_POLICY="$ROOT/frontend/src/panels/modules/opportunity_counts/empty_label.rs"
CREDENTIAL_INPUTS="$ROOT/frontend/src/panels/modules/settings/tabs/venue_credentials/inputs.rs"
BROWSER="$ROOT/test/e2e/pr_ft_load_state.spec.ts"
COPY_GATE="$ROOT/scripts/product_copy_gate.sh"
AUDIT_DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BS completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  empty_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bs-empty.XXXXXX")"
  credential_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bs-credential.XXXXXX")"
  browser_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bs-browser.XXXXXX")"
  copy_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bs-copy.XXXXXX")"
  audit_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bs-audit.XXXXXX")"
  cp "$EMPTY_POLICY" "$empty_backup"
  cp "$CREDENTIAL_INPUTS" "$credential_backup"
  cp "$BROWSER" "$browser_backup"
  cp "$COPY_GATE" "$copy_backup"
  cp "$AUDIT_DOC" "$audit_backup"
  restore() {
    cp "$empty_backup" "$EMPTY_POLICY"
    cp "$credential_backup" "$CREDENTIAL_INPUTS"
    cp "$browser_backup" "$BROWSER"
    cp "$copy_backup" "$COPY_GATE"
    cp "$audit_backup" "$AUDIT_DOC"
    rm -f "$empty_backup" "$credential_backup" "$browser_backup" "$copy_backup" "$audit_backup"
  }
  trap restore EXIT

  PR_BS_SKIP_TESTS=1 PR_BS_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$EMPTY_POLICY" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'return format!("机会快照错误 · {}", stream_problem_label(problem));'
if source.count(marker) != 1:
    raise SystemExit("PR-BS self-test setup failed: cold-error marker drifted")
path.write_text(source.replace(marker, 'return format!("当前范围暂无{}", input.noun);', 1), encoding="utf-8")
PY
  if PR_BS_SKIP_TESTS=1 PR_BS_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a cold error rewritten as empty success"
  fi
  cp "$empty_backup" "$EMPTY_POLICY"

  python3 - "$CREDENTIAL_INPUTS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'data-credential-spec-state="error"'
if source.count(marker) != 1:
    raise SystemExit("PR-BS self-test setup failed: credential error marker drifted")
path.write_text(source.replace(marker, 'data-credential-spec-state="unknown"', 1), encoding="utf-8")
PY
  if PR_BS_SKIP_TESTS=1 PR_BS_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a credential error without typed editor state"
  fi
  cp "$credential_backup" "$CREDENTIAL_INPUTS"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'test("PR-FT settings credentials cold error remains typed and hides static success panels"'
if source.count(marker) != 1:
    raise SystemExit("PR-BS self-test setup failed: browser marker drifted")
path.write_text(source.replace(marker, 'test.skip("PR-FT settings credentials cold error remains typed and hides static success panels"', 1), encoding="utf-8")
PY
  if PR_BS_SKIP_TESTS=1 PR_BS_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped product-semantics fixture"
  fi
  cp "$browser_backup" "$BROWSER"

  python3 - "$COPY_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "if rg -n '等待交易所凭证规格'"
if source.count(marker) != 1:
    raise SystemExit("PR-BS self-test setup failed: copy marker drifted")
path.write_text(source.replace(marker, "if false && rg -n '等待交易所凭证规格'", 1), encoding="utf-8")
PY
  if PR_BS_SKIP_TESTS=1 PR_BS_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a detached credential copy regression rule"
  fi
  cp "$copy_backup" "$COPY_GATE"

  python3 - "$AUDIT_DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
updated, replacements = __import__("re").subn(
    r"(?m)^1\. \*\*PR-[A-Z]+",
    "1. **PR-BV",
    source,
    count=1,
)
if replacements != 1:
    raise SystemExit("PR-BS self-test setup failed: current queue head is missing")
path.write_text(updated, encoding="utf-8")
PY
  if PR_BS_SKIP_TESTS=1 PR_BS_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a completed PR-BV successor in the local queue"
  fi
  cp "$audit_backup" "$AUDIT_DOC"

  python3 - "$AUDIT_DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = path.read_text(encoding="utf-8").splitlines()
matches = [
    index
    for index, line in enumerate(lines)
    if "机会搜索/详情/预览错误被空态吞没" in line
]
if len(matches) != 1:
    raise SystemExit("PR-BS self-test setup failed: preview finding drifted")
index = matches[0]
if "✅ 完成" not in lines[index] or "PR-BY" not in lines[index]:
    raise SystemExit("PR-BS self-test setup failed: PR-BY preview closure is missing")
lines[index] = lines[index].replace("✅ 完成", "🟡 部分完成", 1)
path.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
  if PR_BS_SKIP_TESTS=1 PR_BS_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted preview debt after the completed PR-BY successor"
  fi
  cp "$audit_backup" "$AUDIT_DOC"

  python3 - "$AUDIT_DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = path.read_text(encoding="utf-8").splitlines()
matches = [
    index
    for index, line in enumerate(lines)
    if "Hyperliquid account address 容易与 agent address 混淆" in line
]
if len(matches) != 1:
    raise SystemExit("PR-BS self-test setup failed: Hyperliquid identity finding drifted")
index = matches[0]
if "✅ 完成" not in lines[index] or "PR-CB" not in lines[index]:
    raise SystemExit("PR-BS self-test setup failed: PR-CB identity closure is missing")
lines[index] = lines[index].replace("✅ 完成", "🟡 部分完成", 1)
path.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
  if PR_BS_SKIP_TESTS=1 PR_BS_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted Hyperliquid identity debt after the completed PR-CB successor"
  fi

  printf 'PR-BS completion self-test passed\n'
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
pr_id = "PR-BS"
title = "PR-BS Frontend Product Semantics & Copy Gate"
verify_anchor = "`bash scripts/check_pr_bs_completion.sh --self-test`"
evidence_contract = {
    "opportunity-empty-policy": "frontend/src/panels/modules/opportunity_counts/empty_label.rs",
    "cost-semantics": "frontend/src/panels/modules/cost_profile.rs",
    "strategy-exposure": "shared-types/src/strategy.rs",
    "main-strategy-route": "crates/api/src/services/strategy.rs",
    "credential-spec-state": "frontend/src/panels/modules/settings/tabs/venue_credentials/inputs.rs",
    "credential-save-lock": "frontend/src/panels/modules/settings/tabs/venue_credentials.rs",
    "credential-copy-semantics": "frontend/src/panels/modules/settings/tabs/venue_credentials/format/status.rs",
    "runtime-health-semantics": "frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs",
    "okx-ticket-scope-copy": "crates/exchange/src/ws/trading.rs",
    "copy-regression-gate": "scripts/product_copy_gate.sh",
    "product-browser": "test/e2e/pr_ft_load_state.spec.ts",
    "completion-governance": "scripts/check_pr_bs_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-BS completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
fact_lines = history.split("## 附录 I：", 1)[0].splitlines() + doc.splitlines()
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be complete with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive gate once")

queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-BS\b", queue):
    fail("completed PR-BS remains in the local queue")
successor_title = "PR-BV Ranking Contract & Score Explanation"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
    None,
)
if successor_row is None:
    fail("PR-BV successor roadmap row is missing")
successor_complete = "✅ 完成" in successor_row
successor_queued = re.search(r"(?m)^\d+\.\s+\*\*PR-BV\b", queue) is not None
successor_is_head = re.search(r"(?m)^1\.\s+\*\*PR-BV\b", queue) is not None
if successor_complete and successor_queued:
    fail("completed PR-BV successor remains in the local queue")
if not successor_complete and not successor_is_head:
    fail("unfinished PR-BV successor must remain the local queue head")

completed_titles = (
    "期货/机会表空态仍会掩盖错误；旧列语义回流已关闭",
    "RWA/链上/DEX 产品边界缺 copy gate",
    "空态仍用“等待/等待数据”隐藏错误来源",
    "实盘 adapter 能力是静态聚合，凭证文案会显示“全部已验证”",
    "OKX WS 能力 note 残留旧 readiness gate 语义",
    "旧 readiness / spot v1 / strategy kinds 被测试固定为成功 route",
)
for finding_title in completed_titles:
    finding = next((line for line in reversed(fact_lines) if finding_title in line), None)
    if finding is None or "✅" not in finding:
        fail(f"absorbed product-semantics finding remains incomplete: {finding_title}")
for audit_id in ("AUD-111", "AUD-235"):
    audit_row = next((line for line in reversed(fact_lines) if f"`{audit_id}`" in line), None)
    if audit_row is None or "✅ 完成" not in audit_row:
        fail(f"audit row remains incomplete: {audit_id}")
preview_finding = next(
    (line for line in reversed(fact_lines) if "机会搜索/详情/预览错误被空态吞没" in line),
    None,
)
problem_successor_title = "PR-BY Frontend ApiProblem & LoadState Contract"
problem_successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{problem_successor_title}`")),
    None,
)
problem_successor_complete = (
    problem_successor_row is not None and "✅ 完成" in problem_successor_row
)
if problem_successor_complete:
    if preview_finding is None or "✅ 完成" not in preview_finding or "PR-BY" not in preview_finding:
        fail("completed PR-BY successor must own the closed preview finding")
elif preview_finding is None or "🟡 部分完成" not in preview_finding or "PR-BS" in preview_finding:
    fail("remaining preview owner was not narrowed after PR-BS")

address_finding = next(
    (
        line
        for line in reversed(fact_lines)
        if "Hyperliquid account address 容易与 agent address 混淆" in line
    ),
    None,
)
identity_successor_title = "PR-CB Settings Credential Health & ActionState Contract"
identity_successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{identity_successor_title}`")),
    None,
)
identity_successor_complete = (
    identity_successor_row is not None and "✅ 完成" in identity_successor_row
)
if identity_successor_complete:
    if address_finding is None or "✅ 完成" not in address_finding or "PR-CB" not in address_finding:
        fail("completed PR-CB successor must own the closed Hyperliquid identity finding")
elif address_finding is None or "🟡 部分完成" not in address_finding or "PR-BS" in address_finding:
    fail("remaining Hyperliquid address owner was not narrowed after PR-BS")

if "## 2026-07-15 PR-BS Frontend Product Semantics and Copy Gate Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == pr_id]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{evidence_type} artifact drifted or is missing")
    if not evidence["command"].strip() or not evidence["notes"].strip():
        fail(f"{evidence_type} lacks command or notes")

markers = {
    "frontend/src/panels/modules/opportunity_counts/empty_label.rs": (
        'return format!("机会快照错误 · {}", stream_problem_label(problem));',
        'return format!("品种搜索失败 · {}", stream_problem_label(problem));',
        'return format!("当前筛选无匹配{}", input.noun);',
    ),
    "frontend/src/panels/modules/cost_profile.rs": (
        "if has_explicit_one_cycle(cost)",
        "cost.gross_edge_bps - cost.total_cost_bps",
    ),
    "shared-types/src/strategy.rs": (
        "pub enum StrategyExposure",
        "pub const fn is_main_p0_executable",
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials/inputs.rs": (
        "pub(super) enum CredentialSpecState",
        'data-credential-spec-state="error"',
        'problem_message("读取凭证规格失败", &problem)',
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials.rs": (
        "!credential_spec_ready.get()",
        "CredentialSpecState::MissingSpec { venue }",
    ),
    "frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs": (
        "snapshot.currently_usable_count",
        'LoadState::Error(problem) => return problem_cell("读取交易运行状态失败", &problem)',
    ),
    "crates/exchange/src/ws/trading.rs": (
        "HedgeTicket 执行前校验双腿凭证与能力",
    ),
    "scripts/product_copy_gate.sh": (
        "if rg -n '等待交易所凭证规格'",
        "credential specs must expose typed loading/error/missing states",
        "main product strategy exposure must stay explicit",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

inventory = (root / "docs/API_ROUTE_INVENTORY.tsv").read_text(encoding="utf-8")
if "/api/strategy/main-kinds\tGET\tstrategy\tmain_p0\talways" not in inventory:
    fail("main strategy route is not always-on")
if "/api/v1/strategy/kinds\tGET\tstrategy_v1\tdiagnostic\tdefault_off" not in inventory:
    fail("diagnostic strategy route is not default-off")

browser = (root / "test/e2e/pr_ft_load_state.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("product-semantics browser fixture must not be skipped")
for marker in (
    'data-credential-spec-state="error"',
    'getByRole("button", { name: "保存字段" })).toBeDisabled()',
    'not.toContainText("等待交易所凭证规格")',
    "机会快照错误 · ${context}",
):
    if marker not in browser:
        fail(f"product browser marker drifted: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
expected_script = "playwright test test/e2e/pr_ft_load_state.spec.ts"
if package.get("scripts", {}).get("test:e2e:pr-bs") != expected_script:
    fail("package test:e2e:pr-bs script drifted")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence_contract.values():
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

print(
    f"OK PR-BS static contract ({len(evidence_contract)} evidence rows; "
    "typed empty states, credential truth, runtime semantics and product copy)"
)
PY

bash "$ROOT/scripts/product_copy_gate.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_BS_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_BO_SKIP_TESTS=1 PR_BO_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bo_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  PR_DU_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_du_completion.sh"
  bash "$ROOT/scripts/check_route_inventory.sh"
fi

if [[ "${PR_BS_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib empty_label --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib credential_spec_state --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api main_strategy --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib ws_operation_registry_keeps_schema_pending_writes_non_ready --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-bs -- --workers=1
fi

printf 'PR-BS Frontend Product Semantics & Copy Gate completion gate passed\n'
