#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
TRANSPORT="$ROOT/frontend/src/api/rest/transport.rs"
RECOVERY="$ROOT/frontend/src/panels/modules/settings/data/action_recovery.rs"
BROWSER="$ROOT/test/e2e/pr_ck_action_evidence.spec.ts"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-CK completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  backup_dir="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-ck.XXXXXX")"
  cp "$TRANSPORT" "$backup_dir/transport.rs"
  cp "$RECOVERY" "$backup_dir/action_recovery.rs"
  cp "$BROWSER" "$backup_dir/pr_ck_action_evidence.spec.ts"
  restore() {
    cp "$backup_dir/transport.rs" "$TRANSPORT"
    cp "$backup_dir/action_recovery.rs" "$RECOVERY"
    cp "$backup_dir/pr_ck_action_evidence.spec.ts" "$BROWSER"
    rm -rf "$backup_dir"
  }
  trap restore EXIT

  PR_CK_SKIP_TESTS=1 PR_CK_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$TRANSPORT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = ".request_headers(Request::post(&url), request_id);"
if source.count(marker) != 1:
    raise SystemExit("PR-CK self-test transport marker drifted")
path.write_text(source.replace(marker, '.request_headers(Request::post(&url), "detached");'), encoding="utf-8")
PY
  if PR_CK_SKIP_TESTS=1 PR_CK_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a mutation detached from its displayed request id"
  fi
  cp "$backup_dir/transport.rs" "$TRANSPORT"

  python3 - "$RECOVERY" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "latest_action_run(rows, &kinds)"
if source.count(marker) != 1:
    raise SystemExit("PR-CK self-test recovery marker drifted")
path.write_text(source.replace(marker, "rows.first()"), encoding="utf-8")
PY
  if PR_CK_SKIP_TESTS=1 PR_CK_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted unscoped ActionRun recovery"
  fi
  cp "$backup_dir/action_recovery.rs" "$RECOVERY"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'test("PR-CK execution pending state exposes request idempotency and client order ids"'
if source.count(marker) != 1:
    raise SystemExit("PR-CK self-test browser marker drifted")
path.write_text(source.replace(marker, 'test.skip("PR-CK execution pending state exposes request idempotency and client order ids"'), encoding="utf-8")
PY
  if PR_CK_SKIP_TESTS=1 PR_CK_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped product evidence scenario"
  fi

  printf 'PR-CK completion self-test passed\n'
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
title = "PR-CK Frontend High-Risk ActionState Boundary"
verify_anchor = "`bash scripts/check_pr_ck_completion.sh --self-test`"
evidence = {
    "shared-action-evidence": "shared-types/src/actions/evidence.rs",
    "shared-action-state": "shared-types/src/actions/state.rs",
    "exact-request-context": "frontend/src/api/rest/transport.rs",
    "hedge-request-context": "frontend/src/api/rest/arbitrage.rs",
    "trading-request-context": "frontend/src/api/rest/trading.rs",
    "portfolio-request-context": "frontend/src/api/rest/portfolio_system.rs",
    "settings-request-context": "frontend/src/api/rest/exchanges.rs",
    "snapshot-recovery": "frontend/src/state/action_state.rs",
    "execution-action-boundary": "frontend/src/panels/modules/execution/data/actions.rs",
    "execution-outcome-evidence": "frontend/src/panels/modules/execution/data/actions/outcome.rs",
    "execution-run-context": "frontend/src/panels/modules/execution/data/run/context.rs",
    "execution-remedy-boundary": "frontend/src/panels/modules/execution/data/remedy.rs",
    "positions-action-boundary": "frontend/src/panels/modules/positions/data/actions.rs",
    "positions-compensation-boundary": "frontend/src/panels/modules/positions/data/actions/compensation.rs",
    "positions-close-run-recovery": "frontend/src/panels/modules/positions/data/runs.rs",
    "positions-snapshot-recovery": "frontend/src/panels/modules/positions/data/runs/recovery.rs",
    "settings-action-boundary": "frontend/src/panels/modules/settings/data/actions.rs",
    "settings-mutation-transport": "frontend/src/panels/modules/settings/data/actions/transport.rs",
    "settings-replay-boundary": "frontend/src/panels/modules/settings/data/actions/replay.rs",
    "settings-maintenance-boundary": "frontend/src/panels/modules/settings/data/credential_maintenance.rs",
    "settings-action-run-recovery": "frontend/src/panels/modules/settings/data/action_recovery.rs",
    "settings-selection-boundary": "frontend/src/panels/modules/settings/tabs/venue_credentials.rs",
    "legacy-readiness-boundary": "frontend/src/panels/modules/settings/view.rs",
    "non-skipping-product-proof": "test/e2e/pr_ck_action_evidence.spec.ts",
    "product-suite-contract": "package.json",
    "release-suite-contract": "scripts/fixtures/release_qa_contract/package.json",
    "completion-governance": "scripts/check_pr_ck_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-CK completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-CK\b", queue):
    fail("completed PR-CK remains in the local queue")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-15 PR-CK Frontend High-Risk ActionState Boundary Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-CK"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence):
    fail(f"evidence type drift: expected={sorted(evidence)}, actual={sorted(indexed)}")
for kind, artifact in evidence.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/actions/evidence.rs": (
        "pub struct ActionEvidence",
        "pub request_id: Option<String>",
        "pub action_run_id: Option<String>",
        "pub idempotency_key: Option<String>",
        "pub client_order_ids: Vec<String>",
    ),
    "shared-types/src/actions/evidence/conversions.rs": (
        "pub fn from_action_run",
        "pub fn from_close_run",
        "pub fn from_execution_run",
        "pub fn from_order_record",
    ),
    "shared-types/src/actions/state.rs": (
        "evidence: ActionEvidence",
        "pub fn with_evidence",
        "pub const fn evidence(&self)",
        "pub fn message(&self, default: &str)",
        "legacy_state_without_evidence_still_decodes",
    ),
    "frontend/src/api/rest/transport.rs": (
        "pub struct MutationRequestContext",
        "let request_id = context.request_id();",
        ".request_headers(Request::post(&url), request_id);",
        ".request_headers(Request::patch(&url), request_id);",
        "request.header(HEADER_IDEMPOTENCY_KEY, idempotency_key)",
    ),
    "frontend/src/api/rest/arbitrage.rs": ("confirm_hedge_with_context", "post_json_with_context"),
    "frontend/src/api/rest/trading.rs": ("cancel_order_with_context", "post_json_with_context"),
    "frontend/src/api/rest/portfolio_system.rs": (
        "close_portfolio_position_with_context",
        "close_all_portfolio_positions_with_context",
        "post_json_with_context",
    ),
    "frontend/src/api/rest/exchanges.rs": (
        "save_venue_credentials_with_context",
        "clear_venue_credentials_with_context",
        "migrate_venue_credentials_with_context",
        "post_json_with_context",
    ),
    "frontend/src/state/action_state.rs": (
        "action_state_from_action_run",
        "latest_action_run",
        "action_state_from_execution_run",
        "action_state_from_order_record",
        "merge_order_evidence",
    ),
    "frontend/src/panels/modules/execution/data/actions.rs": (
        "MutationRequestContext::with_idempotency_key",
        ".with_client_order_ids(request.client_order_ids)",
        "confirm_hedge_with_context",
        "restored_execution_run_evidence",
    ),
    "frontend/src/panels/modules/execution/data/actions/outcome.rs": (
        "confirm_response_evidence",
        "ActionEvidence::from_execution_run",
        "ActionEvidence::from_order_record",
        "confirm_response_problem",
    ),
    "frontend/src/panels/modules/execution/data/run/context.rs": (
        "restored_execution_run_evidence",
        "ActionEvidence::from_execution_run(run).with_idempotency_key",
    ),
    "frontend/src/panels/modules/execution/data/remedy.rs": (
        "MutationRequestContext::new_idempotent_attempt",
        "ActionEvidence::from_execution_run",
    ),
    "frontend/src/panels/modules/positions/data/actions.rs": (
        "close_request_context",
        "MutationRequestContext::with_idempotency_key",
        "recover_position_close_state",
    ),
    "frontend/src/panels/modules/positions/data/actions/compensation.rs": (
        "ActionEvidence::from_close_run",
        "submit_close_run_compensation_task",
        "cancel_close_run_compensation_task",
        "submit_close_run_manual_terminal_task",
    ),
    "frontend/src/panels/modules/positions/data/runs.rs": (
        "close_run_action_state",
        "ActionEvidence::from_close_run",
        "ActionEvidence::from_order_record",
    ),
    "frontend/src/panels/modules/positions/data/runs/recovery.rs": (
        "recover_close_run_state",
        ".recent_close_runs",
        "CloseRunScope::Single | CloseRunScope::Pair",
    ),
    "frontend/src/panels/modules/settings/data/actions.rs": (
        "use_action_run_recovery",
        "MutationRequestContext::with_idempotency_key",
        "credential_response_evidence",
    ),
    "frontend/src/panels/modules/settings/data/actions/transport.rs": (
        "save_venue_credentials_with_context",
        "update_trading_risk_config_with_context",
        "set_kill_switch_with_context",
    ),
    "frontend/src/panels/modules/settings/data/actions/replay.rs": (
        "credential_save_fingerprint",
        "risk_config_replay_slot",
        "should_reuse_credential_replay_key",
    ),
    "frontend/src/panels/modules/settings/data/credential_maintenance.rs": (
        "use_action_run_recovery",
        "clear_venue_credentials_with_context",
        "migrate_venue_credentials_with_context",
    ),
    "frontend/src/panels/modules/settings/data/action_recovery.rs": (
        "latest_action_run(rows, &kinds)",
        "action_state_from_action_run(run)",
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials.rs": (
        "previous_selected",
        "credential_selection_changed",
        "use_action_runs(credentials_refresh_nonce)",
    ),
    "frontend/src/panels/modules/settings/view.rs": (
        'SettingsTab::from_slug("readiness"), None',
        "SettingsTab::Credentials",
        "SettingsTab::Diagnostics",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

transport_tests = (root / "frontend/src/api/rest/transport/tests.rs").read_text(encoding="utf-8")
transport_match = re.search(
    r"(?P<attrs>(?:\s*#\[[^\n]+\]\s*)+)fn\s+mutation_context_reuses_exact_request_identity_in_evidence\b",
    transport_tests,
)
if transport_match is None or "test" not in transport_match.group("attrs"):
    fail("exact request-context fixture is missing from the transport test module")
if any(marker in transport_match.group("attrs") for marker in ("ignore", "should_panic", "cfg(")):
    fail("exact request-context fixture must remain non-skipping")

for legacy_path in (
    "frontend/src/panels/modules/settings/tabs/readiness.rs",
    "crates/api/src/services/live_readiness.rs",
):
    if (root / legacy_path).exists():
        fail(f"legacy readiness file returned: {legacy_path}")

browser_path = evidence["non-skipping-product-proof"]
browser = (root / browser_path).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
browser_markers = (
    "PR-CK pending evidence uses the exact mutation request headers",
    "PR-CK restores structured ActionRun evidence after settings unmount",
    "PR-CK execution pending state exposes request idempotency and client order ids",
    'headers["x-request-id"]',
    'headers["idempotency-key"]',
    "request_id req-pr-ck-restored",
    "action_run_id action-pr-ck-restored",
    "client_order_id",
)
for marker in browser_markers:
    if marker not in browser:
        fail(f"browser fixture missing marker: {marker}")
if browser.count('test("PR-CK ') != 3:
    fail("browser fixture must keep all three non-skipping scenarios")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
dedicated = "playwright test test/e2e/pr_ck_action_evidence.spec.ts"
if scripts.get("test:e2e:pr-ck") != dedicated:
    fail("dedicated browser command is missing")
if scripts.get("test:e2e:product", "").count(browser_path) != 1:
    fail("product suite must include the PR-CK fixture exactly once")
release = json.loads(
    (root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8")
)
if release.get("scripts", {}).get("test:e2e:product", "").count(browser_path) != 1:
    fail("release QA fixture must include the PR-CK fixture exactly once")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
coverage_exceptions = {"package.json", "scripts/fixtures/release_qa_contract/package.json"}
for artifact in evidence.values():
    if artifact in coverage_exceptions:
        continue
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

print(f"OK PR-CK static completion contract ({len(evidence)} evidence types)")
PY

if rg -n 'use_global\(\)\.(client|settings_client)|settings_client\(|spawn_local' \
  "$ROOT/frontend/src/panels/modules" \
  --glob '*.rs' --glob '!**/data.rs' --glob '!**/data/*.rs' --glob '!**/data/**/*.rs' >/dev/null; then
  fail "component/view/tab bypassed a data hook"
fi
if rg -n 'client\.(submit|cancel|close|set_|update_|save_|confirm|select|kill|reconcile|execute|place|delete|create)' \
  "$ROOT/frontend/src/panels/modules" \
  --glob '*.rs' --glob '!**/data.rs' --glob '!**/data/*.rs' --glob '!**/data/**/*.rs' >/dev/null; then
  fail "component/view/tab called a mutation client directly"
fi

if [[ "${PR_CK_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DT_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dt_completion.sh"
  PR_DU_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_du_completion.sh"
  PR_DF_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_df_completion.sh"
  PR_DZ_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dz_completion.sh"
fi

if [[ "${PR_CK_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test -p shared-types actions --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    mutation_context_reuses_exact_request_identity_in_evidence --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    action_run_recovery_keeps_structured_context --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    confirm_response_merges_pending_run_and_order_identity_evidence --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    close_run_message_includes_idempotency_evidence --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    initial_venue_selection_preserves_recovered_action_state --lib --no-fail-fast
  (cd "$ROOT" && CI=1 npm run test:e2e:pr-ck -- --workers=1)
fi

printf 'PR-CK completion gate passed\n'
