#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
INVENTORY="$ROOT/docs/API_ROUTE_INVENTORY.tsv"
AUDIT_ACTIONS="$ROOT/crates/api/src/services/action_runs/audit_log.rs"
TERMINAL_TESTS="$ROOT/crates/api/src/services/action_runs/tests/terminal_contract.rs"
WS_BROWSER="$ROOT/test/e2e/data_pipeline.spec.ts"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-CA completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  temp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-ca.XXXXXX")"
  cp "$DOC" "$temp/audit.md"
  cp "$INVENTORY" "$temp/inventory.tsv"
  cp "$AUDIT_ACTIONS" "$temp/audit-actions.rs"
  cp "$TERMINAL_TESTS" "$temp/terminal-contract.rs"
  cp "$WS_BROWSER" "$temp/data-pipeline.spec.ts"
  restore() {
    cp "$temp/audit.md" "$DOC"
    cp "$temp/inventory.tsv" "$INVENTORY"
    cp "$temp/audit-actions.rs" "$AUDIT_ACTIONS"
    cp "$temp/terminal-contract.rs" "$TERMINAL_TESTS"
    cp "$temp/data-pipeline.spec.ts" "$WS_BROWSER"
    rm -rf "$temp"
  }
  trap restore EXIT

  PR_CA_SKIP_TESTS=1 PR_CA_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$INVENTORY" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "/api/trading/orders\tPOST\ttrading\tmain_p0\talways\thigh\tbearer\taction_run\t"
if source.count(marker) != 1:
    raise SystemExit("PR-CA self-test setup failed: order mutation inventory marker drifted")
path.write_text(source.replace(marker, marker.replace("action_run", "read"), 1), encoding="utf-8")
PY
  if PR_CA_SKIP_TESTS=1 PR_CA_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a high-risk route without ActionRun audit policy"
  fi
  cp "$temp/inventory.tsv" "$INVENTORY"

  python3 - "$AUDIT_ACTIONS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'ActionRunKind::TradingOrderSubmit => "trading.order.submit"'
if source.count(marker) != 1:
    raise SystemExit("PR-CA self-test setup failed: audit action marker drifted")
path.write_text(source.replace(marker, marker.replace("trading.order.submit", "trading.order.unknown"), 1), encoding="utf-8")
PY
  if PR_CA_SKIP_TESTS=1 PR_CA_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted an ActionRun-to-audit-action drift"
  fi
  cp "$temp/audit-actions.rs" "$AUDIT_ACTIONS"

  python3 - "$TERMINAL_TESTS" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
name = "core_high_risk_mutations_preserve_identity_across_terminal_outcomes"
pattern = re.compile(rf"(?m)^(\s*)(?:async\s+)?fn\s+{name}\s*\(")
match = pattern.search(source)
if match is None:
    raise SystemExit("PR-CA self-test setup failed: terminal matrix test drifted")
path.write_text(source[:match.start()] + match.group(1) + "#[ignore]\n" + source[match.start():], encoding="utf-8")
PY
  if PR_CA_SKIP_TESTS=1 PR_CA_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted an ignored ActionRun terminal matrix"
  fi
  cp "$temp/terminal-contract.rs" "$TERMINAL_TESTS"

  python3 - "$WS_BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
title = "opportunities websocket authenticates with ticket before subscribe"
marker = f'test("{title}"'
if source.count(marker) != 1:
    raise SystemExit("PR-CA self-test setup failed: WS browser anchor drifted")
path.write_text(source.replace(marker, f'test.skip("{title}"', 1), encoding="utf-8")
PY
  if PR_CA_SKIP_TESTS=1 PR_CA_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped browser WS security fixture"
  fi
  cp "$temp/data-pipeline.spec.ts" "$WS_BROWSER"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
heading = "### 🟡 6.5 下一步执行队列"
if source.count(heading) != 1:
    raise SystemExit("PR-CA self-test setup failed: queue heading drifted")
stale = "\n\n1. **PR-CA Route Inventory & High-Risk Audit Gate** — stale completed row"
path.write_text(source.replace(heading, heading + stale, 1), encoding="utf-8")
PY
  if PR_CA_SKIP_TESTS=1 PR_CA_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted completed PR-CA in the local queue"
  fi
  cp "$temp/audit.md" "$DOC"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-FJ Security Verification, CI Gate & Runtime Smoke Contract` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-CA self-test setup failed: PR-FJ authority marker drifted")
path.write_text(source.replace(marker, marker.replace("✅ 完成", "🟡 部分完成"), 1), encoding="utf-8")
PY
  if PR_CA_SKIP_TESTS=1 PR_CA_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted an unfinished security verification authority"
  fi

  printf 'PR-CA completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

from collections import Counter
from pathlib import Path
import csv
import re
import sys

root = Path(sys.argv[1])
pr_id = "PR-CA"
title = "PR-CA Route Inventory & High-Risk Audit Gate"
verify_anchor = "`bash scripts/check_pr_ca_completion.sh --self-test`"
evidence_contract = {
    "route-inventory-authority": ("docs/API_ROUTE_INVENTORY.tsv", "bash scripts/check_route_inventory.sh"),
    "route-registry-runtime-parity": ("crates/api/src/route_specs.rs", "route_registry_"),
    "route-static-client-parity": ("scripts/check_route_inventory.sh", "bash scripts/check_route_inventory.sh"),
    "route-runtime-policy": ("scripts/check_route_runtime_policy.sh", "bash scripts/check_route_runtime_policy.sh"),
    "mutation-audit-matrix": ("scripts/check_mutation_audit_contract.sh", "bash scripts/check_mutation_audit_contract.sh"),
    "action-run-terminal-correlation": ("crates/api/src/services/action_runs/tests/terminal_contract.rs", "core_high_risk_mutations_preserve_identity_across_terminal_outcomes"),
    "real-api-security-audit": ("scripts/verify_api_security_runtime_smoke.sh", "bash scripts/verify_api_security_runtime_smoke.sh"),
    "durable-audit-restart-authority": ("scripts/check_pr_fi_completion.sh", "bash scripts/check_pr_fi_completion.sh --self-test"),
    "route-surface-authority": ("scripts/check_pr_do_completion.sh", "bash scripts/check_pr_do_completion.sh --self-test"),
    "security-surface-authority": ("scripts/check_pr_dk_completion.sh", "bash scripts/check_pr_dk_completion.sh --self-test"),
    "security-verification-authority": ("scripts/check_pr_fj_completion.sh", "bash scripts/check_pr_fj_completion.sh --self-test"),
    "ws-channel-boundary": ("crates/api/src/routers/websocket.rs", "routers::websocket::tests"),
    "browser-rest-security": ("test/e2e/route_registry.spec.ts", "test:e2e:pr-dk"),
    "browser-ws-auth": ("test/e2e/data_pipeline.spec.ts", "test:e2e:pr-dk"),
    "frontend-action-authority": ("scripts/check_pr_ck_completion.sh", "bash scripts/check_pr_ck_completion.sh --self-test"),
    "api-problem-authority": ("scripts/check_pr_eh_completion.sh", "bash scripts/check_pr_eh_completion.sh"),
    "completion-governance": ("scripts/check_pr_ca_completion.sh", "bash scripts/check_pr_ca_completion.sh --self-test"),
}
mutations = (
    ("POST", "/api/arbitrage/opportunities/:id/confirm", "HedgeConfirm", "hedge.confirm"),
    ("PATCH", "/api/automation/config", "AutomationConfigUpdate", "automation.config.update"),
    ("POST", "/api/automation/control", "AutomationControl", "automation.control"),
    ("POST", "/api/exchanges/credentials", "VenueCredentialsUpdate", "venue_credentials.update"),
    ("POST", "/api/exchanges/credentials/clear", "VenueCredentialsClear", "venue_credentials.clear"),
    ("POST", "/api/exchanges/credentials/migrate", "VenueCredentialsMigrate", "venue_credentials.migrate"),
    ("POST", "/api/trading/adapters/select", "TradingAdapterSelect", "trading.adapter.select"),
    ("POST", "/api/trading/fee-snapshots", "TradingFeeSnapshotUpsert", "trading.fee_snapshot.upsert"),
    ("POST", "/api/trading/kill-switch", "TradingKillSwitch", "trading.kill_switch.set"),
    ("POST", "/api/trading/orders", "TradingOrderSubmit", "trading.order.submit"),
    ("POST", "/api/trading/orders/:id/cancel", "TradingOrderCancel", "trading.order.cancel"),
    ("POST", "/api/trading/orders/reconcile", "TradingOrderReconcile", "trading.order.reconcile"),
    ("POST", "/api/trading/portfolio/close-all", "PortfolioCloseAll", "portfolio.positions.close_all"),
    ("POST", "/api/trading/portfolio/close-runs/:close_run_id/compensation-orders", "PortfolioCloseCompensation", "portfolio.close_run.compensate"),
    ("POST", "/api/trading/portfolio/close-runs/:close_run_id/manual-terminal", "PortfolioCloseManualTerminal", "portfolio.close_run.manual_terminal"),
    ("POST", "/api/trading/portfolio/positions/:venue/:symbol/close", "PortfolioClosePosition", "portfolio.position.close"),
    ("POST", "/api/trading/portfolio/positions/:venue/:symbol/close-pair", "PortfolioClosePair", "portfolio.position.close_pair"),
    ("PATCH", "/api/trading/risk-config", "TradingRiskConfigUpdate", "trading.risk_config.update"),
    ("PATCH", "/api/webhook/config", "WebhookConfigUpdate", "webhook.config.update"),
)


def fail(message: str) -> None:
    raise SystemExit(f"PR-CA completion gate failed: {message}")


def require_incomplete_queue_head(doc: str, queue: str) -> None:
    matched = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    if not matched:
        fail("local queue must retain a PR roadmap item at its head")
    head = matched.group(1)
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{head} ")), None)
    if row is None or "✅ 完成" in row:
        fail(f"local queue head must be an incomplete roadmap row: {head}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
fact_lines = history.split("## 附录 I：", 1)[0].splitlines() + doc.splitlines()
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be complete with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must bind the destructive completion gate")

queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-CA\b", queue):
    fail("completed PR-CA remains in the local queue")
successor_title = "PR-CB Settings Credential Health & ActionState Contract"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
    None,
)
if successor_row is None:
    fail("PR-CB successor roadmap row is missing")
successor_complete = "✅ 完成" in successor_row and "剩余：无。" in successor_row
successor_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CB\b", queue))
following_title = "PR-CD Portfolio AccountState Evidence & Risk LoadState Contract"
following_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{following_title}`")),
    None,
)
if following_row is None:
    fail("PR-CD following-successor roadmap row is missing")
following_complete = "✅ 完成" in following_row and "剩余：无。" in following_row
following_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CD\b", queue))
next_title = "PR-CM Venue API Status Center & Runtime Probe Contract"
next_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{next_title}`")),
    None,
)
if next_row is None:
    fail("PR-CM next-successor roadmap row is missing")
next_complete = "✅ 完成" in next_row and "剩余：无" in next_row
next_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CM\b", queue))
final_title = "PR-CN Frontend Workstation State & Navigation Runtime Contract"
final_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{final_title}`")),
    None,
)
if final_row is None:
    fail("PR-CN final-successor roadmap row is missing")
final_complete = "✅ 完成" in final_row and "剩余：无" in final_row
final_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CN\b", queue))
after_title = "PR-CQ Local Runtime & Operator QA Contract"
after_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{after_title}`")),
    None,
)
if after_row is None:
    fail("PR-CQ after-successor roadmap row is missing")
after_complete = "✅ 完成" in after_row and "剩余：无" in after_row
after_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CQ\b", queue))
if successor_complete:
    if successor_is_head:
        fail("completed PR-CB successor remains at the local queue head")
    if following_complete:
        if following_is_head:
            fail("completed PR-CD following-successor remains at the local queue head")
        if next_complete:
            if next_is_head:
                fail("completed PR-CM next-successor remains at the local queue head")
            if final_complete:
                if final_is_head:
                    fail("completed PR-CN final-successor remains at the local queue head")
                if after_complete:
                    if after_is_head:
                        fail("completed PR-CQ after-successor remains at the local queue head")
                    require_incomplete_queue_head(doc, queue)
                elif not after_is_head:
                    fail("PR-CQ must become the local queue head after PR-CN completion")
            elif not final_is_head:
                fail("PR-CN must become the local queue head after PR-CM completion")
        elif not next_is_head:
            fail("PR-CM must become the local queue head after PR-CD completion")
    elif not following_is_head:
        fail("PR-CD must become the local queue head after PR-CB completion")
elif not successor_is_head:
    fail("PR-CB must remain the local queue head until its completion contract closes")
queue_items = re.findall(r"(?m)^\d+\.\s+\*\*", queue)
if not queue_items:
    fail("local queue plus external pool must not be empty")

delegated_rows = [
    line
    for line in fact_lines
    if line.startswith("|") and "PR-CA" in line and "✅ 完成" in " ".join(line.split("|")[1:3])
]
if len(delegated_rows) < 8:
    fail(f"expected at least eight PR-CA audit rows, found {len(delegated_rows)}")
for delegated in delegated_rows:
    cells = [cell.strip() for cell in delegated.strip().strip("|").split("|")]
    if "✅ 完成" not in " ".join(cells[:2]) or any(
        marker in " ".join(cells[:2]) for marker in ("🟡", "⏳", "❌")
    ):
        fail(f"unfinished audit row still delegates work to PR-CA: {cells[0]}")

for successor in ("PR-BR", "PR-CK", "PR-DK", "PR-DO", "PR-EH", "PR-FH", "PR-FI", "PR-FJ"):
    successor_row = next(
        (line for line in doc.splitlines() if line.startswith(f"| `{successor} ")),
        None,
    )
    if successor_row is None or "✅ 完成" not in successor_row:
        fail(f"required successor authority is not complete: {successor}")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [entry for entry in csv.DictReader(handle, delimiter="\t") if entry["pr_id"] == pr_id]
indexed = {entry["evidence_type"]: entry for entry in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, (artifact, command_anchor) in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{evidence_type} artifact drifted or is missing")
    if command_anchor not in evidence["command"] or not evidence["notes"].strip():
        fail(f"{evidence_type} command or notes drifted")

with (root / "docs/API_ROUTE_INVENTORY.tsv").open(encoding="utf-8", newline="") as handle:
    inventory = list(csv.DictReader(handle, delimiter="\t"))
endpoint_keys = [
    (method, entry["path"])
    for entry in inventory
    for method in entry["methods"].split(",")
]
if len(endpoint_keys) != 95 or len(set(endpoint_keys)) != 95:
    fail(f"route inventory must remain exactly 95 unique endpoints, got {len(endpoint_keys)}")
exposure = Counter(
    entry["default_exposure"] for entry in inventory for _ in entry["methods"].split(",")
)
if exposure != Counter({"always": 78, "default_off": 17}):
    fail(f"route exposure matrix drifted: {dict(exposure)}")
protected = {
    (method, entry["path"])
    for entry in inventory
    if entry["class"] == "main_p0"
    and entry["risk"] == "high"
    and entry["auth_policy"] == "bearer"
    and entry["audit_policy"] in {"action_run", "secret_mutation"}
    for method in entry["methods"].split(",")
}
expected_mutations = {(method, path) for method, path, _, _ in mutations}
if protected != expected_mutations:
    fail(f"20-route mutation matrix drifted: missing={sorted(expected_mutations - protected)}, extra={sorted(protected - expected_mutations)}")

audit_source = (root / "crates/api/src/services/action_runs/audit_log.rs").read_text(encoding="utf-8")
for _, _, kind, action in mutations:
    if f'ActionRunKind::{kind} => "{action}"' not in audit_source:
        fail(f"ActionRunKind::{kind} audit action drifted")


def require_runnable_test(relative_path: str, name: str) -> None:
    source = (root / relative_path).read_text(encoding="utf-8")
    pattern = re.compile(
        rf"(?m)((?:^\s*#\[[^\n]+\]\s*\n)+)\s*(?:async\s+)?fn\s+{re.escape(name)}\s*\("
    )
    match = pattern.search(source)
    if match is None or not re.search(r"#\[(?:tokio::)?test", match.group(1)):
        fail(f"missing runnable test anchor: {relative_path}::{name}")
    if re.search(r"\b(?:ignore|should_panic)\b", match.group(1)):
        fail(f"test anchor is ignored or should_panic: {relative_path}::{name}")


for relative_path, name in (
    ("crates/api/src/app.rs", "route_inventory_action_run_policies_match_typed_runtime_registry"),
    ("crates/api/src/services/action_runs/tests/terminal_contract.rs", "core_high_risk_mutations_preserve_identity_across_terminal_outcomes"),
    ("crates/api/src/routers/arbitrage/hedge_tests/pricing_confirm_context.rs", "confirm_missing_preview_terminalizes_action_run_and_preserves_identity"),
    ("crates/api/src/routers/websocket.rs", "subscribe_requires_auth_when_security_enabled"),
    ("crates/api/src/routers/websocket.rs", "unknown_channel_is_rejected"),
    ("crates/api/src/routers/websocket.rs", "cap_blocks_new_subscriptions"),
):
    require_runnable_test(relative_path, name)

browser_anchors = {
    "test/e2e/route_registry.spec.ts": (
        "every always-on bearer route preserves browser auth and typed error boundaries",
        "high-risk action and secret mutation persist correlated redacted audit pairs",
    ),
    "test/e2e/data_pipeline.spec.ts": (
        "opportunities websocket authenticates with ticket before subscribe",
    ),
}
for relative_path, titles in browser_anchors.items():
    source = (root / relative_path).read_text(encoding="utf-8")
    for test_title in titles:
        escaped = re.escape(test_title)
        if re.search(rf"(?m)^\s*test\.(?:skip|fixme)\(\s*['\"]{escaped}['\"]", source):
            fail(f"browser anchor must not be skipped: {test_title}")
        if not re.search(rf"(?m)^\s*test\(\s*['\"]{escaped}['\"]", source):
            fail(f"missing non-skipping browser anchor: {test_title}")

markers = {
    "scripts/verify_api_security_runtime_smoke.sh": (
        "expected_audit_lines_before_replay=$((AUTH_DENIAL_EVENT_COUNT + 18))",
        "assert_auth_denial_event",
        "idempotent replay appended audit events",
        "cross-route accepted/terminal audit pairs",
        "audit log did not append correlated evidence across restart",
        "restarted audit log leaked a credential sentinel",
    ),
    "crates/api/src/routers/websocket.rs": (
        "const MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 32;",
        "websocket auth ticket required before subscribe",
    ),
    "scripts/check_pr_bz_completion.sh": (
        'successor_title = "PR-CA Route Inventory & High-Risk Audit Gate"',
        "successor_complete",
        "successor_is_head",
    ),
}
for relative_path, required in markers.items():
    source = (root / relative_path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {relative_path} marker: {marker}")

verify_source = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if verify_source.count("check_pr_ca_completion.sh") != 2:
    fail("repo docs/full gate wiring drifted")

if "## 2026-07-15 PR-CA Route Inventory and High-Risk Audit Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {entry["file"]: entry for entry in csv.DictReader(handle, delimiter="\t")}
for artifact, _ in evidence_contract.values():
    if artifact.startswith("docs/"):
        continue
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

print(
    f"OK PR-CA static contract ({len(evidence_contract)} evidence rows; "
    "95 routes, 17 default-off surfaces, 20 audited mutations, REST/WS security closure)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_route_inventory.sh"
bash "$ROOT/scripts/check_route_endpoint_metadata.sh"
bash "$ROOT/scripts/check_route_runtime_policy.sh"
bash "$ROOT/scripts/check_mutation_audit_contract.sh"

if [[ "${PR_CA_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DO_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_do_completion.sh"
  PR_DK_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dk_completion.sh"
  bash "$ROOT/scripts/check_pr_fj_completion.sh"
  bash "$ROOT/scripts/check_pr_fi_completion.sh"
  PR_CK_SKIP_TESTS=1 PR_CK_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ck_completion.sh"
  bash "$ROOT/scripts/check_pr_eh_completion.sh"
fi

if [[ "${PR_CA_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api route_inventory_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api route_registry_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api core_high_risk_mutations_preserve_identity_across_terminal_outcomes --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api confirm_missing_preview_terminalizes_action_run_and_preserves_identity --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api routers::websocket::tests --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" bash "$ROOT/scripts/verify_security_contract.sh"
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dk -- --workers=1
fi

printf 'PR-CA Route Inventory and High-Risk Audit Gate completion passed\n'
