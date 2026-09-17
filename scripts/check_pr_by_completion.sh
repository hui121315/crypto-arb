#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
DTO_FILE="$ROOT/frontend/src/api/rest/dto.rs"
REST_ERROR="$ROOT/frontend/src/api/rest/error.rs"
LOAD_STATE="$ROOT/frontend/src/state/load_state.rs"
WS_STATE="$ROOT/frontend/src/api/ws.rs"
BROWSER="$ROOT/test/e2e/pr_ft_load_state.spec.ts"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BY completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  temp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-by.XXXXXX")"
  cp "$DTO_FILE" "$temp/dto.rs"
  cp "$REST_ERROR" "$temp/error.rs"
  cp "$LOAD_STATE" "$temp/load-state.rs"
  cp "$WS_STATE" "$temp/ws.rs"
  cp "$BROWSER" "$temp/browser.ts"
  cp "$DOC" "$temp/audit.md"
  restore() {
    cp "$temp/dto.rs" "$DTO_FILE"
    cp "$temp/error.rs" "$REST_ERROR"
    cp "$temp/load-state.rs" "$LOAD_STATE"
    cp "$temp/ws.rs" "$WS_STATE"
    cp "$temp/browser.ts" "$BROWSER"
    cp "$temp/audit.md" "$DOC"
    rm -rf "$temp"
  }
  trap restore EXIT

  PR_BY_SKIP_TESTS=1 PR_BY_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  printf '\npub struct LocalMirroredOrder;\n' >>"$DTO_FILE"
  if PR_BY_SKIP_TESTS=1 PR_BY_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a frontend-local REST DTO"
  fi
  cp "$temp/dto.rs" "$DTO_FILE"

  python3 - "$REST_ERROR" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "serde_json::from_str::<ApiProblemEnvelope>(body).map(|envelope| envelope.error)"
if source.count(marker) != 1:
    raise SystemExit("PR-BY self-test setup failed: REST parser marker drifted")
path.write_text(source.replace(marker, marker.replace(".map", ".ok().map"), 1), encoding="utf-8")
PY
  if PR_BY_SKIP_TESTS=1 PR_BY_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted erased REST problem parse failures"
  fi
  cp "$temp/error.rs" "$REST_ERROR"

  python3 - "$LOAD_STATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "Stale { value: T, problem: ApiProblem },"
if source.count(marker) != 1:
    raise SystemExit("PR-BY self-test setup failed: LoadState marker drifted")
path.write_text(source.replace(marker, "Stale { value: T },", 1), encoding="utf-8")
PY
  if PR_BY_SKIP_TESTS=1 PR_BY_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted stale state without a typed problem"
  fi
  cp "$temp/load-state.rs" "$LOAD_STATE"

  python3 - "$WS_STATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "pub problem_count: u64,"
if source.count(marker) != 1:
    raise SystemExit("PR-BY self-test setup failed: WS problem counter marker drifted")
path.write_text(source.replace(marker, "pub hidden_problem_count: u64,", 1), encoding="utf-8")
PY
  if PR_BY_SKIP_TESTS=1 PR_BY_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted hidden WS problem evidence"
  fi
  cp "$temp/ws.rs" "$WS_STATE"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'test("PR-FT opportunities cold error remains typed'
if source.count(marker) != 1:
    raise SystemExit("PR-BY self-test setup failed: browser fixture marker drifted")
path.write_text(source.replace(marker, 'test.skip("PR-FT opportunities cold error remains typed', 1), encoding="utf-8")
PY
  if PR_BY_SKIP_TESTS=1 PR_BY_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped product error-state fixture"
  fi
  cp "$temp/browser.ts" "$BROWSER"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "### 🟡 6.5 下一步执行队列"
if source.count(marker) != 1:
    raise SystemExit("PR-BY self-test setup failed: queue heading drifted")
stale = "\n\n1. **PR-BY Frontend ApiProblem & LoadState Contract** — stale completed row"
path.write_text(
    source.replace(marker, marker + stale, 1),
    encoding="utf-8",
)
PY
  if PR_BY_SKIP_TESTS=1 PR_BY_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a completed PR-BY in the local queue"
  fi

  printf 'PR-BY completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
pr_id = "PR-BY"
title = "PR-BY Frontend ApiProblem & LoadState Contract"
verify_anchor = "`bash scripts/check_pr_by_completion.sh --self-test`"
evidence_contract = {
    "shared-api-problem": "shared-types/src/problem.rs",
    "shared-action-state": "shared-types/src/actions/state.rs",
    "frontend-load-state": "frontend/src/state/load_state.rs",
    "resource-envelope-polling": "frontend/src/state/polling.rs",
    "rest-problem-parser": "frontend/src/api/rest/error.rs",
    "rest-request-correlation": "frontend/src/api/rest/transport.rs",
    "ws-channel-state": "frontend/src/api/ws.rs",
    "ws-runtime-problems": "frontend/src/api/ws_runtime.rs",
    "opportunity-loadstate-source": "frontend/src/state/arbitrage_stream.rs",
    "review-loadstate-consumer": "frontend/src/panels/modules/review/data.rs",
    "settings-loadstate-consumer": "frontend/src/panels/modules/settings/data/resources.rs",
    "positions-transport-consumer": "frontend/src/panels/modules/positions/components/snapshot_transport.rs",
    "execution-preview-consumer": "frontend/src/panels/modules/execution/data/preview.rs",
    "opportunity-detail-consumer": "frontend/src/panels/modules/opportunities/components/detail_panel.rs",
    "dto-single-source-governance": "scripts/check_frontend_dto_mirror.sh",
    "async-state-governance": "scripts/verify_repo_gates.sh",
    "product-error-state-browser": "test/e2e/pr_ft_load_state.spec.ts",
    "ws-loadstate-authority": "scripts/check_pr_eh_completion.sh",
    "transport-authority": "scripts/check_pr_dl_completion.sh",
    "request-correlation-authority": "scripts/check_pr_dt_completion.sh",
    "non-p0-dto-authority": "scripts/check_pr_dq_completion.sh",
    "completion-governance": "scripts/check_pr_by_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-BY completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
fact_lines = history.split("## 附录 I：", 1)[0].splitlines() + doc.splitlines()
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be complete with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate once")

queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-BY\b", queue):
    fail("completed PR-BY remains in the local queue")
successor_title = "PR-BZ Exchange Official Evidence Registry & Fixture Gate"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
    None,
)
if successor_row is None:
    fail("PR-BZ successor roadmap row is missing")
successor_complete = "✅ 完成" in successor_row
successor_queued = re.search(r"(?m)^\d+\.\s+\*\*PR-BZ\b", queue) is not None
successor_is_head = re.search(r"(?m)^1\.\s+\*\*PR-BZ\b", queue) is not None
if successor_complete and successor_queued:
    fail("completed PR-BZ successor remains in the local queue")
if not successor_complete and not successor_is_head:
    fail("unfinished PR-BZ successor must remain the local queue head")

for line in doc.splitlines():
    if "PR-BY" not in line or not line.startswith("|"):
        continue
    cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
    if any(marker in cell for cell in cells[:2] for marker in ("🟡", "⏳", "❌")):
        fail(f"unfinished audit row still delegates work to PR-BY: {cells[0]}")

for audit_id in ("AUD-131", "AUD-136", "AUD-138", "AUD-139", "AUD-153"):
    audit = next((line for line in reversed(fact_lines) if f"`{audit_id}`" in line), None)
    if audit is None or "✅ 完成" not in audit:
        fail(f"absorbed audit row remains incomplete: {audit_id}")

absorbed_findings = (
    "持仓/风控前端仍用 `Option` 吞错",
    "OKX `get_order` parse error 退成 `None`",
    "Bybit realtime-only 查单不足以证明最终态",
    "KuCoin `get_order` clientOid 查询入口已修",
    "机会搜索/详情错误被吞成空数据",
    "执行 Preview 错误被吞成 pending",
    "Settings resource 失败被吞成读取中",
    "通用前端 polling 继续吞错",
    "Options positions 返回空组合和 0 Greeks",
    "前端保留未挂主 UI 的 chat/options/simulation client 与 DTO",
    "portfolio/system snapshot 失败只 warn 不广播 degraded",
    "polling/action 继续吞掉 typed errors",
    "机会搜索/详情/预览错误被空态吞没",
    "Settings diagnostics 没有 per-venue runtime health",
    "强平/funding/margin 缺失被空值或估算掩盖",
    "持仓/余额前端错误被空态吞掉",
    "通用资源 `.ok()` 吞错",
    "前端 `ApiError` 丢失 status/code/request_id/retry_after",
    "通用 polling 把 REST 失败 `.ok()` 成 `None`",
    "对冲 preview / 机会详情 / symbol search 吞错",
    "主线 REST DTO 前后端重复定义",
    "request_id 只在 header/log，不进入 handler/audit/error body",
    "平仓 / 下单前端错误仍缺完整 action request_id 链",
    "设置页 resource 把 API 失败显示成读取中",
    "Portfolio snapshot 是整包成功/失败",
    "Balance API 失败与保证金不足同形",
)
for finding_title in absorbed_findings:
    finding = next((line for line in reversed(fact_lines) if finding_title in line), None)
    if finding is None or "✅ 完成" not in finding:
        fail(f"absorbed finding remains incomplete: {finding_title}")

for successor in (
    "PR-BA", "PR-CJ", "PR-CK", "PR-DE", "PR-DF", "PR-DG", "PR-DL",
    "PR-DQ", "PR-DR", "PR-DS", "PR-DT", "PR-DZ", "PR-EH", "PR-EX", "PR-FT",
):
    successor_row = next(
        (line for line in doc.splitlines() if line.startswith(f"| `{successor} ")),
        None,
    )
    if successor_row is None or "✅ 完成" not in successor_row:
        fail(f"required successor authority is not complete: {successor}")

if "## 2026-07-15 PR-BY Frontend ApiProblem and LoadState Closure" not in history:
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
    "shared-types/src/problem.rs": (
        "pub struct ApiProblem",
        "pub request_id: Option<String>",
        "pub retry_after_ms: Option<u64>",
        "pub source: Option<String>",
    ),
    "shared-types/src/actions/state.rs": (
        "pub enum ActionState",
        "Pending {",
        "Failed {",
        "problem: ApiProblem",
    ),
    "frontend/src/state/load_state.rs": (
        "pub enum LoadState<T>",
        "Stale { value: T, problem: ApiProblem }",
        "Error(ApiProblem)",
        "error_after_ready_preserves_stale_value",
    ),
    "frontend/src/state/polling.rs": (
        "pub fn use_resource_envelope",
        "pub fn apply_resource_envelope_state",
        "resource_envelope_preserves_stale_value_and_typed_problem",
        "polling_retry_deadline_uses_typed_retry_after",
    ),
    "frontend/src/api/rest/error.rs": (
        "serde_json::from_str::<ApiProblemEnvelope>(body).map",
        '"problemParseError"',
        '"bodyExcerpt"',
        "falls_back_when_body_is_not_problem_json",
    ),
    "frontend/src/api/rest/transport.rs": (
        "let request_id = next_request_id();",
        "HEADER_REQUEST_ID",
        "MutationRequestContext",
    ),
    "frontend/src/api/ws.rs": (
        "pub struct WsChannelState",
        "pub message_count: u64",
        "pub problem_count: u64",
        "pub last_problem_at_ms: Option<u64>",
    ),
    "frontend/src/api/ws_runtime.rs": (
        "runtime_payload_problem_persists_for_late_subscriber",
        "runtime_successful_payload_is_counted_once",
        "ws_auth_ticket_problem",
        "WS_WRITE_ERROR",
    ),
    "frontend/src/state/arbitrage_stream.rs": (
        "preserved_problem_after_event",
        "LoadState::Stale",
        "arbitrage_stream_retry_after_uses_event_and_problem_max",
    ),
    "frontend/src/panels/modules/review/data.rs": (
        ".await.map_err(api_problem)",
        "review_retry_deadline_for_result",
        "state.apply_result(result)",
    ),
    "frontend/src/panels/modules/settings/data/resources.rs": (
        "type SettingsResource<T> = RwSignal<LoadState<T>>",
        "apply_settings_result",
        "state.apply_result(result.map_err",
    ),
    "frontend/src/panels/modules/positions/components/snapshot_transport.rs": (
        "struct SnapshotTransport",
        "retry_after_falls_back_to_last_error",
    ),
    "frontend/src/panels/modules/execution/data/preview.rs": (
        "RwSignal<LoadState<ExecutionPreview>>",
        "LoadState::Stale { value, problem }",
        "LoadState::Error(problem)",
    ),
    "frontend/src/panels/modules/opportunities/components/detail_panel.rs": (
        "LoadState::Error(problem)",
        "stale_empty_detail_state_keeps_problem_visible",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

rest_error = (root / "frontend/src/api/rest/error.rs").read_text(encoding="utf-8")
if ".ok()" in rest_error:
    fail("REST ApiProblem parser erases parse failures with .ok()")
browser = (root / "test/e2e/pr_ft_load_state.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("product error-state browser fixture must not be skipped")
for marker in (
    "PR-FT opportunities cold error remains typed",
    "PR-FT review cold error remains typed",
    "PR-FT settings credentials cold error remains typed",
    "PR-FT execution preview cold error remains typed",
):
    if marker not in browser:
        fail(f"browser fixture is missing: {marker}")

verify_source = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if verify_source.count("check_pr_by_completion.sh") != 2:
    fail("repo docs/full gate wiring drifted")
predecessor = (root / "scripts/check_pr_bw_completion.sh").read_text(encoding="utf-8")
for marker in (
    'successor_title = "PR-BY Frontend ApiProblem & LoadState Contract"',
    "successor_complete",
    "successor_is_head",
):
    if marker not in predecessor:
        fail(f"PR-BW predecessor is not successor-aware: {marker}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence_contract.values():
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

print(
    f"OK PR-BY static contract ({len(evidence_contract)} evidence rows; "
    "typed REST/WS state, stale preservation, DTO single-source and Chromium errors)"
)
PY

bash "$ROOT/scripts/check_frontend_dto_mirror.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_BY_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_EH_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eh_completion.sh"
  PR_DL_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dl_completion.sh"
  PR_DT_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dt_completion.sh"
fi

if [[ "${PR_BY_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types problem --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib api::rest::error --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib load_state --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib polling --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib ws_runtime --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib arbitrage_stream --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib review --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib settings --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib snapshot_transport --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib detail_state --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib preview --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ft -- --workers=1
fi

printf 'PR-BY Frontend ApiProblem and LoadState completion gate passed\n'
