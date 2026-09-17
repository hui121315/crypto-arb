#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
RISK_TAB="$ROOT/frontend/src/panels/modules/settings/tabs/risk_config.rs"
RISK_FORM="$ROOT/frontend/src/panels/modules/settings/tabs/risk_config/form.rs"
RISK_CHECKS="$ROOT/crates/trading/src/risk/checks.rs"
BROWSER="$ROOT/test/e2e/pr_du_settings_control_plane.spec.ts"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BN completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  tab_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bn-tab.XXXXXX")"
  form_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bn-form.XXXXXX")"
  checks_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bn-checks.XXXXXX")"
  browser_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bn-browser.XXXXXX")"
  cp "$RISK_TAB" "$tab_backup"
  cp "$RISK_FORM" "$form_backup"
  cp "$RISK_CHECKS" "$checks_backup"
  cp "$BROWSER" "$browser_backup"
  restore() {
    cp "$tab_backup" "$RISK_TAB"
    cp "$form_backup" "$RISK_FORM"
    cp "$checks_backup" "$RISK_CHECKS"
    cp "$browser_backup" "$BROWSER"
    rm -f "$tab_backup" "$form_backup" "$checks_backup" "$browser_backup"
  }
  trap restore EXIT

  PR_BN_SKIP_TESTS=1 PR_BN_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$RISK_TAB" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'data-settings-risk-scope="runtime-readonly"'
if source.count(marker) != 1:
    raise SystemExit("PR-BN self-test setup failed: runtime scope marker drifted")
path.write_text(source.replace(marker, 'data-settings-risk-scope="runtime-editable"', 1), encoding="utf-8")
PY
  if PR_BN_SKIP_TESTS=1 PR_BN_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted editable runtime facts"
  fi
  cp "$tab_backup" "$RISK_TAB"

  python3 - "$RISK_FORM" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "实盘写入启用"
if source.count(marker) != 1:
    raise SystemExit("PR-BN self-test setup failed: product environment copy drifted")
path.write_text(source.replace(marker, "Live 写入启用", 1), encoding="utf-8")
PY
  if PR_BN_SKIP_TESTS=1 PR_BN_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted legacy Live product copy"
  fi
  cp "$form_backup" "$RISK_FORM"

  python3 - "$RISK_CHECKS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "config.kill_switch_active && !intent.reduce_only"
if source.count(marker) != 1:
    raise SystemExit("PR-BN self-test setup failed: reduce-only guard marker drifted")
path.write_text(source.replace(marker, "config.kill_switch_active", 1), encoding="utf-8")
PY
  if PR_BN_SKIP_TESTS=1 PR_BN_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a kill switch that blocks reduce-only closes"
  fi
  cp "$checks_backup" "$RISK_CHECKS"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'test("PR-BN separates runtime facts, editable risk, and kill-switch policy"'
if source.count(marker) != 1:
    raise SystemExit("PR-BN self-test setup failed: browser test marker drifted")
path.write_text(source.replace(marker, 'test.skip("PR-BN separates runtime facts, editable risk, and kill-switch policy"', 1), encoding="utf-8")
PY
  if PR_BN_SKIP_TESTS=1 PR_BN_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped product fixture"
  fi

  printf 'PR-BN completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
pr_id = "PR-BN"
title = "PR-BN Risk Mode & Guard Evidence Contract"
verify_anchor = "`bash scripts/check_pr_bn_completion.sh --self-test`"
evidence_contract = {
    "risk-block-evidence": "shared-types/src/live_trading.rs",
    "ticket-preflight-contract": "shared-types/src/hedge.rs",
    "action-evidence": "shared-types/src/actions/evidence.rs",
    "risk-engine-policy": "crates/trading/src/risk/checks.rs",
    "risk-engine-tests": "crates/trading/src/risk/tests/gating.rs",
    "funding-direction": "crates/portfolio/src/risk.rs",
    "risk-config-boundary": "crates/api/src/services/risk_config.rs",
    "adapter-preflight": "crates/api/src/services/hedge_preflight/capability.rs",
    "margin-preflight": "crates/api/src/services/hedge_margin.rs",
    "confirm-guard-aggregation": "crates/api/src/services/hedge_confirm/confirm_validate.rs",
    "kill-switch-action-run": "crates/api/src/routers/trading/kill_switch.rs",
    "cancel-action-run": "crates/api/src/routers/trading/orders.rs",
    "terminal-action-matrix": "crates/api/src/services/action_runs/tests/terminal_contract.rs",
    "runtime-risk-ui": "frontend/src/panels/modules/settings/tabs/risk_config.rs",
    "runtime-risk-facts": "frontend/src/panels/modules/settings/tabs/risk_config/form.rs",
    "shared-risk-policy": "frontend/src/panels/shared/risk_policy.rs",
    "positions-risk-policy": "frontend/src/panels/modules/positions/components/kill_switch_bar.rs",
    "frontend-action-state": "frontend/src/panels/modules/positions/data/actions.rs",
    "frontend-preflight": "frontend/src/panels/modules/execution/components/risk_preview/preflight.rs",
    "partial-outcome": "frontend/src/panels/modules/execution/data/outcome.rs",
    "product-browser": "test/e2e/pr_du_settings_control_plane.spec.ts",
    "product-style": "frontend/styles/src/skin/settings.css",
    "mutation-audit-gate": "scripts/check_mutation_audit_contract.sh",
    "scoped-preflight-authority": "scripts/check_pr_dv_completion.sh",
    "action-state-authority": "scripts/check_pr_ck_completion.sh",
    "durable-audit-authority": "scripts/check_pr_fi_completion.sh",
    "partial-outcome-authority": "scripts/check_pr_df_completion.sh",
    "portfolio-close-authority": "scripts/check_pr_dz_completion.sh",
    "completion-governance": "scripts/check_pr_bn_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-BN completion gate failed: {message}")


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
if re.search(r"(?m)^\d+\.\s+\*\*PR-BN\b", queue):
    fail("completed PR-BN remains in the local queue")

finding_titles = (
    "顶部 Live/Dry-run 是本地状态",
    "RiskConfig snapshot / patch / adapter select 读写边界不清",
    "RiskDecision 阻断缺 actual/limit/source evidence",
    "Kill Switch 与 CloseAll 共用组件局部 busy",
    "设置页执行模式仍用前端本地 `risk_live` 和 Dry-run 文案",
    "风险 funding outflow 把收款腿也按绝对值计入",
)
for finding_title in finding_titles:
    finding = next((line for line in reversed(fact_lines) if finding_title in line), None)
    if finding is None or "✅" not in finding:
        fail(f"absorbed finding remains incomplete: {finding_title}")

upstream_titles = (
    "PR-DV HedgeTicket Scoped Preflight & Confirm Contract",
    "PR-CK Frontend High-Risk ActionState Boundary",
    "PR-FI API Security, Auth/CORS, WS Auth & High-Risk Audit Contract",
    "PR-DF Execution Preview ActionState & ApiProblem Contract",
    "PR-DZ Portfolio AccountState & CloseRun Contract",
    "PR-ED AccountState Evidence & Unified Margin Contract",
    "PR-ES Venue Capability Matrix & Order Compiler Contract",
)
for upstream_title in upstream_titles:
    upstream = next(
        (line for line in doc.splitlines() if line.startswith(f"| `{upstream_title}`")),
        None,
    )
    if upstream is None or "✅ 完成" not in upstream or "剩余：无。" not in upstream:
        fail(f"completed upstream authority drifted: {upstream_title}")

if "## 2026-07-15 PR-BN Risk Mode and Guard Evidence Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == pr_id]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(
        "evidence type drift: "
        f"expected={sorted(evidence_contract)}, actual={sorted(indexed)}"
    )
for evidence_type, artifact in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{evidence_type} artifact drifted or is missing")
    if not evidence["command"].strip() or not evidence["notes"].strip():
        fail(f"{evidence_type} lacks command or notes")

markers = {
    "shared-types/src/live_trading.rs": (
        "pub struct RiskBlockEvidence",
        "pub actual: Option<serde_json::Value>",
        "pub limit: Option<serde_json::Value>",
        "pub source: String",
    ),
    "shared-types/src/hedge.rs": (
        "pub preflight_outcome: Option<MarginPreflightOutcome>",
        "pub operations: Vec<HedgePreflightOperation>",
    ),
    "shared-types/src/actions/evidence.rs": ("pub struct ActionEvidence", "pub request_id: Option<String>"),
    "crates/trading/src/risk/checks.rs": ("config.kill_switch_active && !intent.reduce_only",),
    "crates/trading/src/risk/tests/gating.rs": ("kill_switch_allows_reduce_only_market_close",),
    "crates/portfolio/src/risk.rs": ("funding_payment_usd(row).max(0.0)",),
    "crates/api/src/services/hedge_preflight/capability.rs": ("preflight_outcome: Some(MarginPreflightOutcome",),
    "crates/api/src/services/hedge_margin.rs": ("blocked_margin_evidence_guard", "preflight_outcome: Some(preflight)"),
    "crates/api/src/services/hedge_confirm/confirm_validate.rs": ('with_details(serde_json::json!({ "guards": blocked }))',),
    "crates/api/src/routers/trading/kill_switch.rs": ("ActionRunKind::TradingKillSwitch", "set_kill_switch(payload.active)"),
    "crates/api/src/routers/trading/orders.rs": ("ActionRunKind::TradingOrderCancel", "replay_cancel_order"),
    "frontend/src/panels/modules/settings/tabs/risk_config.rs": (
        'data-settings-risk-scope="runtime-readonly"',
        'data-settings-risk-scope="editable-thresholds"',
        'data-settings-risk-scope="kill-switch-action"',
    ),
    "frontend/src/panels/modules/settings/tabs/risk_config/form.rs": ("实盘写入启用", "KILL_SWITCH_POLICY_LABEL"),
    "frontend/src/panels/modules/positions/components/kill_switch_bar.rs": (
        'data-risk-policy="kill-switch"',
        "let kill_busy = Memo::new",
        "let close_all_busy = Memo::new",
    ),
    "frontend/src/panels/modules/execution/components/risk_preview/preflight.rs": ("preflight_outcome.as_ref()",),
    "frontend/src/panels/modules/execution/data/outcome.rs": (
        "let Some(outcome) = response.partial_outcome.as_ref() else",
        "basic_confirm_outcome_detail(response)",
    ),
    "frontend/src/panels/shared/risk_policy.rs": ("不会自动撤销现有挂单",),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

positions_bar = (root / "frontend/src/panels/modules/positions/components/kill_switch_bar.rs").read_text(encoding="utf-8")
if "any_busy" in positions_bar:
    fail("Kill Switch and CloseAll actions must not share a local busy lock")

browser = (root / "test/e2e/pr_du_settings_control_plane.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("Settings control-plane browser fixture must not be skipped")
for marker in (
    "PR-BN separates runtime facts, editable risk, and kill-switch policy",
    'route.request().headers()["x-request-id"]',
    'route.request().headers()["idempotency-key"]',
    'expectedActive: false',
    "不会自动撤销现有挂单",
):
    if marker not in browser:
        fail(f"product browser marker drifted: {marker}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence_contract.values():
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

print(
    f"OK PR-BN static contract ({len(evidence_contract)} evidence rows; "
    "risk policy, scoped guards, ActionRun audit and non-skipping product evidence)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_BN_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DV_SKIP_TESTS=1 PR_DV_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dv_completion.sh"
  PR_CK_SKIP_TESTS=1 PR_CK_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ck_completion.sh"
  bash "$ROOT/scripts/check_pr_fi_completion.sh"
  PR_DF_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_df_completion.sh"
  PR_DZ_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dz_completion.sh"
  bash "$ROOT/scripts/check_mutation_audit_contract.sh"
fi

if [[ "${PR_BN_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types risk_decision --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p trading risk --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p portfolio funding_cluster --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api confirm_scoped_preflight_returns_every_blocked_guard --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api kill_switch --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib risk --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-du -- --workers=1
fi

printf 'PR-BN Risk Mode and Guard evidence completion gate passed\n'
