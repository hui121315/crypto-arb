#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

python3 - "$ROOT" "$MODE" <<'PY'
from pathlib import Path
import csv
import re
import shutil
import sys
import tempfile

root = Path(sys.argv[1])
mode = sys.argv[2]

PR_TITLE = "PR-DB PaperLive Runtime Mode & Legacy Readiness Removal Contract"
VERIFY_ANCHOR = "`bash scripts/check_pr_db_completion.sh --self-test`"

EVIDENCE = {
    "successor-pr-fu": ("scripts/check_pr_fu_completion.sh", "check_pr_fu_completion.sh"),
    "successor-pr-ew": ("scripts/check_pr_ew_completion.sh", "check_pr_ew_completion.sh"),
    "successor-pr-eg": ("scripts/check_pr_eg_completion.sh", "check_pr_eg_completion.sh"),
    "successor-pr-dv": ("scripts/check_pr_dv_completion.sh", "check_pr_dv_completion.sh"),
    "successor-pr-ea": ("scripts/check_pr_ea_completion.sh", "check_pr_ea_completion.sh"),
    "shared-environment-projection": ("shared-types/src/live_trading.rs", "execution_modes_project_to_only_two_product_environments"),
    "backend-runtime-source": ("crates/api/src/routers/trading.rs", "routers::trading"),
    "settings-runtime-health": ("frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs", "runtime_health_title_keeps_latency_problem_and_request_context"),
    "ticket-scoped-preflight": ("frontend/src/panels/modules/execution/components/risk_preview/preflight.rs", "ticket_venue_availability_is_scoped_to_live_ticket_preflight"),
    "order-list-environment": ("frontend/src/panels/shared/orders_list/labels.rs", "order_mode_labels_use_the_shared_paper_live_projection"),
    "review-environment": ("frontend/src/panels/modules/review/components/executed_evidence.rs", "evidence_summary_projects_legacy_modes_and_rejects_mixed_environments"),
    "confirm-context-environment": ("frontend/src/panels/modules/execution/data/outcome.rs", "confirm_context_detail_keeps_runtime_identity_and_leg_problems"),
    "risk-config-environment": ("frontend/src/panels/modules/settings/tabs/risk_config/form/tests.rs", "environment_label_is_explicitly_backend_and_read_only"),
    "legacy-route-404": ("crates/api/src/app.rs", "trading_alias_routes_are_registered"),
    "product-copy-gate": ("scripts/product_copy_gate.sh", "product_copy_gate.sh"),
    "settings-browser": ("test/e2e/data_pipeline.spec.ts", "settings credentials keep static adapter copy separate from runtime readiness"),
    "runtime-health-browser": ("test/e2e/pr_eg_runtime_health.spec.ts", "test:e2e:pr-eg"),
    "scoped-preflight-browser": ("test/e2e/pr_dv_scoped_preflight.spec.ts", "test:e2e:pr-dv"),
    "execution-finality-browser": ("test/e2e/pr_ea_execution_finality.spec.ts", "test:e2e:pr-ea"),
    "completion-governance": ("scripts/check_pr_db_completion.sh", "check_pr_db_completion.sh --self-test"),
}

RUNNABLE = (
    ("shared-types/src/live_trading.rs", "execution_modes_project_to_only_two_product_environments"),
    ("frontend/src/panels/shared/execution_environment.rs", "execution_mode_and_environment_labels_hide_compatibility_variants"),
    ("frontend/src/panels/shared/orders_list/labels.rs", "order_mode_labels_use_the_shared_paper_live_projection"),
    ("frontend/src/panels/modules/review/components/executed_evidence.rs", "evidence_summary_projects_legacy_modes_and_rejects_mixed_environments"),
    ("frontend/src/panels/modules/execution/data/outcome/tests.rs", "confirm_context_detail_keeps_runtime_identity_and_leg_problems"),
    ("frontend/src/panels/modules/settings/tabs/risk_config/form/tests.rs", "environment_label_is_explicitly_backend_and_read_only"),
    ("frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs", "runtime_health_title_keeps_latency_problem_and_request_context"),
    ("frontend/src/panels/modules/execution/components/risk_preview/tests.rs", "ticket_venue_availability_is_scoped_to_live_ticket_preflight"),
    ("crates/api/src/app.rs", "trading_alias_routes_are_registered"),
)

BROWSERS = {
    "test/e2e/data_pipeline.spec.ts": "settings credentials keep static adapter copy separate from runtime readiness",
    "test/e2e/pr_eg_runtime_health.spec.ts": "PR-EG Settings consumes typed venue runtime health and fails closed without evidence",
    "test/e2e/pr_dv_scoped_preflight.spec.ts": "PR-DV renders ticket-scoped capability, account, finality, and request evidence",
    "test/e2e/pr_ea_execution_finality.spec.ts": "PR-EA renders ticket evidence and replayable finality timeline",
}


def fail(message: str) -> None:
    raise ValueError(message)


def rows(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def require_marker(relative: str, *markers: str) -> str:
    source = (root / relative).read_text(encoding="utf-8")
    for marker in markers:
        if marker not in source:
            fail(f"missing {relative} marker: {marker}")
    return source


def check_product_mode_strings() -> None:
    forbidden = re.compile(r'"[^"\n]*\b(?:Paper|Live|Dry-run|Testnet)\b[^"\n]*"')
    for path in (root / "frontend/src").rglob("*.rs"):
        match = forbidden.search(path.read_text(encoding="utf-8"))
        if match:
            fail(f"raw product execution mode string returned: {path.relative_to(root)}:{match.group(0)}")


def check_legacy_route_boundary() -> None:
    roots = [root / "shared-types/src", root / "crates/api/src", root / "frontend/src"]
    legacy_hits: list[tuple[str, str]] = []
    forbidden = re.compile(r"LiveReadiness|APP_LIVE_READY|ADR-0008")
    for base in roots:
        for path in base.rglob("*"):
            if not path.is_file():
                continue
            source = path.read_text(encoding="utf-8", errors="ignore")
            if forbidden.search(source):
                fail(f"legacy readiness identifier returned: {path.relative_to(root)}")
            legacy_hits.extend(
                (path.relative_to(root).as_posix(), line)
                for line in source.splitlines()
                if "live-readiness" in line
            )
    env = root / ".env.example"
    if env.exists() and (forbidden.search(env.read_text(encoding="utf-8")) or "live-readiness" in env.read_text(encoding="utf-8")):
        fail("legacy readiness returned to .env.example")
    if len(legacy_hits) != 1 or legacy_hits[0][0] != "crates/api/src/app.rs":
        fail(f"legacy route must exist only as one API 404 regression assertion: {legacy_hits}")
    app = (root / "crates/api/src/app.rs").read_text(encoding="utf-8")
    if 'route_status(router, "/api/trading/live-readiness")' not in app or "StatusCode::NOT_FOUND" not in app:
        fail("legacy route 404 assertion is detached")


def check(root: Path) -> None:
    doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{PR_TITLE}`")), None)
    if row is None or "✅ 完成" not in row or "剩余：无。" not in row or row.count(VERIFY_ANCHOR) != 1:
        fail("roadmap row must be complete, remaining-none and bound to the destructive gate")
    queue = doc[doc.index("### 🟡 6.5"):]
    if re.search(r"(?m)^\d+\.\s+\*\*PR-DB\b", queue):
        fail("completed PR-DB remains in the queue")
    successor_row = next(
        (line for line in doc.splitlines() if line.startswith("| `PR-DI Portfolio AccountState & CloseRun Contract`")),
        None,
    )
    if successor_row is None or "✅ 完成" not in successor_row or "剩余：无。" not in successor_row:
        fail("successor PR-DI completion drifted")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-DI\b", queue):
        fail("completed successor PR-DI remains in the queue")
    pr_cf_row = next(
        (line for line in doc.splitlines() if line.startswith("| `PR-CF CEX Credential Validation & Account Mode Evidence Matrix`")),
        None,
    )
    if pr_cf_row is None or "✅ 完成" not in pr_cf_row or "剩余：无。" not in pr_cf_row:
        fail("successor PR-CF completion drifted")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-CF\b", queue):
        fail("completed successor PR-CF remains in the queue")
    queue_items = re.findall(r"(?m)^\d+\.\s+\*\*", queue)
    if not queue_items:
        fail("local queue plus external pool must not be empty")

    history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if "## 2026-07-17 PR-DB PaperLive Runtime and Legacy Readiness Closure" not in history:
        fail("PR-DB closure appendix is missing")

    selected = [item for item in rows(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv") if item["pr_id"] == "PR-DB"]
    indexed = {item["evidence_type"]: item for item in selected}
    if len(indexed) != len(selected) or set(indexed) != set(EVIDENCE):
        fail(f"evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for evidence_type, (artifact, anchor) in EVIDENCE.items():
        item = indexed[evidence_type]
        if item["artifact"] != artifact or anchor not in item["command"] or not item["notes"].strip():
            fail(f"evidence anchor drifted: {evidence_type}")
        if not (root / artifact).is_file():
            fail(f"evidence artifact missing: {artifact}")

    coverage = {item["file"]: item for item in rows(root / "docs/PRODUCT_AUDIT_COVERAGE.tsv")}
    for artifact, _ in EVIDENCE.values():
        if artifact.startswith("docs/"):
            continue
        item = coverage.get(artifact)
        if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
            fail(f"exact coverage missing for {artifact}")

    for successor in ("PR-FU", "PR-EW", "PR-EG", "PR-DV", "PR-EA"):
        successor_row = next((line for line in doc.splitlines() if line.startswith(f"| `{successor} ")), "")
        if "✅ 完成" not in successor_row or "剩余：无" not in successor_row:
            fail(f"completed successor authority drifted: {successor}")

    shared = require_marker(
        "shared-types/src/live_trading.rs",
        "pub enum ExecutionEnvironment",
        "Self::DryRun | Self::Testnet => ExecutionEnvironment::Paper",
    )
    if shared.count("pub enum ExecutionEnvironment") != 1:
        fail("ExecutionEnvironment must remain a single shared contract")
    require_marker(
        "crates/api/src/routers/trading.rs",
        "environment: execution_environment(risk)",
        "ExecutionEnvironment::Paper",
        "ExecutionEnvironment::Live",
    )
    require_marker(
        "frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs",
        "VenueRuntimeHealthSnapshot",
        "runtime_health_title_keeps_latency_problem_and_request_context",
    )
    require_marker(
        "frontend/src/panels/modules/execution/components/risk_preview/preflight.rs",
        "ticket_venue_availability_summary",
        "live_operation_health",
    )
    require_marker(
        "frontend/src/panels/shared/orders_list/labels.rs",
        "execution_mode_label(mode)",
        "order_mode_labels_use_the_shared_paper_live_projection",
    )
    require_marker(
        "frontend/src/panels/modules/review/components/executed_evidence.rs",
        "execution_environment_summary(row)",
        "order.intent.mode.environment()",
        '"执行环境缺证据"',
        '"执行环境冲突"',
    )
    require_marker(
        "frontend/src/panels/modules/execution/data/outcome.rs",
        "execution_environment_label(environment)",
    )
    require_marker(
        "frontend/src/panels/modules/settings/tabs/risk_config/form.rs",
        '"实盘写入启用"',
        '"实盘写入停用"',
    )
    product_gate = require_marker("scripts/product_copy_gate.sh", "Paper|Live", "Dry-run|Testnet")
    if "product execution environments must use the shared 模拟/实盘 labels" not in product_gate:
        fail("product copy gate does not explain the shared environment boundary")

    invalid_test = re.compile(r"#\[\s*(?:ignore|should_panic)|#\[\s*cfg_attr[^\]]*ignore")
    for relative, test_name in RUNNABLE:
        source = (root / relative).read_text(encoding="utf-8")
        match = re.search(rf"#\[(?:tokio::)?test\][\s\S]{{0,180}}?(?:async\s+)?fn\s+{re.escape(test_name)}\b", source)
        if match is None or invalid_test.search(source[max(0, match.start() - 160):match.end()]):
            fail(f"runnable test is missing or skipped: {relative}:{test_name}")

    for relative, title in BROWSERS.items():
        source = (root / relative).read_text(encoding="utf-8")
        escaped = re.escape(title)
        if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', source):
            fail(f"browser proof is skipped: {title}")
        if not re.search(rf'test\(\s*["\']{escaped}["\']', source):
            fail(f"browser proof is missing: {title}")

    check_product_mode_strings()
    check_legacy_route_boundary()
    repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    if repo_gate.count("check_pr_db_completion.sh") != 2:
        fail("repo gate must execute PR-DB in docs and full scopes")


def assert_rejected(root: Path, relative: str, transform, label: str) -> None:
    path = root / relative
    baseline = path.read_text(encoding="utf-8")
    changed = transform(baseline)
    if changed == baseline:
        fail(f"self-test setup drifted: {label}")
    path.write_text(changed, encoding="utf-8")
    try:
        check(root)
    except ValueError:
        pass
    else:
        fail(f"self-test accepted {label}")
    finally:
        path.write_text(baseline, encoding="utf-8")


def self_test(source_root: Path) -> None:
    paths = {
        ".env.example",
        "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
        "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        "docs/audit_history/PRODUCT_AUDIT_HISTORY.md",
        "scripts/verify_repo_gates.sh",
        "frontend/src/panels/shared/execution_environment.rs",
        "frontend/src/panels/modules/execution/data/outcome/tests.rs",
        "frontend/src/panels/modules/execution/components/risk_preview/tests.rs",
    }
    paths.update(artifact for artifact, _ in EVIDENCE.values())
    paths.update(relative for relative, _ in RUNNABLE)
    paths.update(BROWSERS)
    with tempfile.TemporaryDirectory(prefix="crossline-pr-db-completion-") as temp:
        test_root = Path(temp) / "repo"
        for relative in paths:
            target = test_root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_root / relative, target)
        global root
        root = test_root
        check(root)
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace(f"| `{PR_TITLE}` | ✅ 完成 |", f"| `{PR_TITLE}` | 🟡 部分完成 |", 1), "a downgraded roadmap row")
        assert_rejected(root, "docs/PRODUCT_AUDIT_EVIDENCE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("PR-DB\treview-environment\t")) + "\n", "an incomplete evidence matrix")
        assert_rejected(root, "frontend/src/panels/shared/execution_environment.rs", lambda text: text.replace('ExecutionEnvironment::Live => "实盘"', 'ExecutionEnvironment::Live => "Live"', 1), "raw Live product copy")
        assert_rejected(root, "crates/api/src/app.rs", lambda text: text.replace("/api/trading/live-readiness", "/api/trading/status", 1), "a missing legacy-route 404 assertion")
        assert_rejected(root, "frontend/src/panels/shared/orders_list/labels.rs", lambda text: text.replace("execution_mode_label(mode)", 'match mode { _ => "模拟" }', 1), "an order-list mode fork")
        assert_rejected(root, "frontend/src/panels/modules/review/components/executed_evidence.rs", lambda text: text.replace("execution_environment_summary(row)", '"执行环境未知".to_owned()', 1), "detached review environment evidence")
        assert_rejected(root, "frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs", lambda text: text.replace("VenueRuntimeHealthSnapshot", "RemovedRuntimeHealthSnapshot"), "detached Settings runtime health")
        assert_rejected(root, "test/e2e/pr_dv_scoped_preflight.spec.ts", lambda text: text.replace('test("PR-DV renders ticket-scoped', 'test.skip("PR-DV renders ticket-scoped', 1), "a skipped scoped-preflight browser proof")
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-DB stale**\n2. **PR-DI stale successor**", 1), "completed PR-DB or PR-DI returned to the queue")
        assert_rejected(root, "scripts/verify_repo_gates.sh", lambda text: text.replace("check_pr_db_completion.sh", "removed_pr_db_completion.sh", 1), "single-scope repo wiring")
    print("PR-DB completion destructive self-test passed")


try:
    if mode == "--self-test":
        self_test(root)
    else:
        check(root)
        print(f"OK PR-DB static contract ({len(EVIDENCE)} evidence types; {len(RUNNABLE)} runnable anchors; {len(BROWSERS)} browser anchors)")
except (OSError, KeyError, ValueError) as exc:
    raise SystemExit(f"PR-DB completion gate failed: {exc}") from exc
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

if [[ "${PR_DB_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test \
    "$ROOT/test/e2e/data_pipeline.spec.ts" \
    "$ROOT/test/e2e/pr_eg_runtime_health.spec.ts" \
    "$ROOT/test/e2e/pr_dv_scoped_preflight.spec.ts" \
    "$ROOT/test/e2e/pr_ea_execution_finality.spec.ts" \
    --list >/dev/null
fi

if [[ "${PR_DB_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_FU_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_fu_completion.sh"
  PR_EW_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ew_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  PR_DV_SKIP_TESTS=1 PR_DV_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dv_completion.sh"
  PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_DB_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types --lib execution_modes_project_to_only_two_product_environments --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib paper_live_projection --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib evidence_summary_projects_legacy_modes_and_rejects_mixed_environments --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib confirm_context_detail_keeps_runtime_identity_and_leg_problems --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib environment_label_is_explicitly_backend_and_read_only --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib runtime_health_title_keeps_latency_problem_and_request_context --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib ticket_venue_availability_is_scoped_to_live_ticket_preflight --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api trading_alias_routes_are_registered --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api live_operation_health_guard_scopes_all_evidence_to_ticket_venues --no-fail-fast
  bash "$ROOT/scripts/product_copy_gate.sh"
  CI=1 npx playwright test \
    "$ROOT/test/e2e/data_pipeline.spec.ts" \
    "$ROOT/test/e2e/pr_eg_runtime_health.spec.ts" \
    "$ROOT/test/e2e/pr_dv_scoped_preflight.spec.ts" \
    "$ROOT/test/e2e/pr_ea_execution_finality.spec.ts" \
    --grep 'settings credentials keep static adapter copy separate from runtime readiness|PR-EG Settings consumes typed venue runtime health and fails closed without evidence|PR-DV renders ticket-scoped capability, account, finality, and request evidence|PR-EA renders ticket evidence and replayable finality timeline' \
    --workers=1
fi

printf 'PR-DB PaperLive runtime and legacy readiness completion passed\n'
