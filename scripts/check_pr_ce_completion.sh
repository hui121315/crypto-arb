#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
BROWSER_TITLE="PR-EQ Settings keeps builder-scoped Hyperliquid endpoint and wallet/vault evidence non-live"

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

PR_TITLE = "PR-CE Hyperliquid Account Mode & Agent Evidence"
VERIFY_ANCHOR = "`bash scripts/check_pr_ce_completion.sh --self-test`"
HISTORY_HEADING = "## 2026-07-17 PR-CE Hyperliquid Account Mode and Agent Evidence Closure"
OFFICIAL_DOCS = (
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint",
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/nonces-and-api-wallets",
)

EVIDENCE = {
    "successor-pr-eq": ("scripts/check_pr_eq_completion.sh", "check_pr_eq_completion.sh"),
    "successor-pr-ed": ("scripts/check_pr_ed_completion.sh", "check_pr_ed_completion.sh"),
    "successor-pr-eg": ("scripts/check_pr_eg_completion.sh", "check_pr_eg_completion.sh"),
    "successor-pr-dg": ("scripts/check_pr_dg_completion.sh", "check_pr_dg_completion.sh"),
    "successor-pr-da": ("scripts/check_pr_da_completion.sh", "check_pr_da_completion.sh"),
    "account-role-relation": (
        "crates/exchange/src/adapters/hyperliquid_support.rs",
        "hyperliquid_relation_probe",
    ),
    "account-role-policy": (
        "crates/api/src/services/venue_credentials/validation/hyperliquid_probes/probe_support.rs",
        "hyperliquid_relation_probe",
    ),
    "account-abstraction-official-request": (
        "crates/exchange/tests/hyperliquid_test.rs",
        "account_abstraction_reads_official_account_and_dex_states",
    ),
    "save-time-account-mode": (
        "crates/api/src/services/venue_credentials/validation/venues/hyperliquid.rs",
        "hyperliquid_abstraction_and_account_mode_probes_are_fail_closed",
    ),
    "account-mode-probe-matrix": (
        "crates/api/src/services/venue_credentials/validation/tests.rs",
        "hyperliquid_abstraction_and_account_mode_probes_are_fail_closed",
    ),
    "partial-account-read-contract": (
        "crates/exchange/src/live.rs",
        "account_read_keeps_spot_truth_when_perp_margin_fails",
    ),
    "partial-account-read-adapter": (
        "crates/exchange/src/adapters/hyperliquid_account_read.rs",
        "account_read_keeps_spot_truth_when_perp_margin_fails",
    ),
    "spot-survives-perp-failure": (
        "crates/exchange/tests/hyperliquid_test.rs",
        "account_read_keeps_spot_truth_when_perp_margin_fails",
    ),
    "perp-survives-spot-failure": (
        "crates/exchange/tests/hyperliquid_test.rs",
        "account_read_keeps_perp_margin_when_spot_truth_fails",
    ),
    "route-partial-problem": (
        "crates/api/src/trading_service/live_adapters/route_tests/cases.rs",
        "live_router_balances_keep_rows_and_record_inner_source_issue",
    ),
    "route-issue-conversion": (
        "crates/api/src/trading_service/live_adapters/failures.rs",
        "live_router_balances_keep_rows_and_record_inner_source_issue",
    ),
    "settings-account-fields": (
        "crates/api/src/services/venue_credentials/specs.rs",
        "venue_credentials",
    ),
    "settings-supplemental-evidence": (
        "frontend/src/panels/modules/settings/tabs/venue_credentials/validation/supplemental.rs",
        "venue_credentials",
    ),
    "settings-browser": (
        "test/e2e/pr_eq_hyperliquid_contract.spec.ts",
        "test:e2e:pr-eq",
    ),
    "builder-dex-private-read": (
        "crates/exchange/src/adapters/hyperliquid_support.rs",
        "private_state_body_adds_builder_dex_only_for_builder_markets",
    ),
    "protected-ioc": (
        "crates/exchange/src/adapters/hyperliquid_trade_data_tests.rs",
        "market_order_is_protected_ioc_limit",
    ),
    "signer-scoped-nonce": (
        "crates/exchange/src/adapters/hyperliquid_ws_trade.rs",
        "signer_nonce_key_distinguishes_network_and_is_signer_scoped",
    ),
    "signer-session-runtime": (
        "crates/exchange/src/adapters/hyperliquid_ws_trade_tests.rs",
        "signer_session_health_records_scope_nonce_and_last_error",
    ),
    "signer-session-operation-health": (
        "crates/api/src/services/venue_operation_health/snapshot/part_21.rs",
        "signer_session_row_exposes_scope_nonce_and_error",
    ),
    "completion-governance": (
        "scripts/check_pr_ce_completion.sh",
        "check_pr_ce_completion.sh --self-test",
    ),
}

RUNNABLE = (
    ("crates/api/src/services/venue_credentials/validation/tests.rs", "hyperliquid_relation_probe_accepts_direct_subaccount_scope"),
    ("crates/api/src/services/venue_credentials/validation/tests.rs", "hyperliquid_abstraction_and_account_mode_probes_are_fail_closed"),
    ("crates/api/src/services/venue_credentials/validation/tests.rs", "hyperliquid_account_read_probes_preserve_partial_source_failure"),
    ("crates/exchange/tests/hyperliquid_test.rs", "account_abstraction_reads_official_account_and_dex_states"),
    ("crates/exchange/tests/hyperliquid_test.rs", "account_read_keeps_spot_truth_when_perp_margin_fails"),
    ("crates/exchange/tests/hyperliquid_test.rs", "account_read_keeps_perp_margin_when_spot_truth_fails"),
    ("crates/api/src/trading_service/live_adapters/route_tests/cases.rs", "live_router_balances_keep_rows_and_record_inner_source_issue"),
    ("crates/exchange/src/adapters/hyperliquid_ws_trade_tests.rs", "signer_nonce_key_distinguishes_network_and_is_signer_scoped"),
    ("crates/exchange/src/adapters/hyperliquid_ws_trade_tests.rs", "signer_session_health_records_scope_nonce_and_last_error"),
    ("crates/api/src/services/venue_operation_health/snapshot/part_21.rs", "signer_session_row_exposes_scope_nonce_and_error"),
    ("crates/exchange/src/adapters/hyperliquid_tests.rs", "private_state_body_adds_builder_dex_only_for_builder_markets"),
    ("crates/exchange/src/adapters/hyperliquid_trade_data_tests.rs", "market_order_is_protected_ioc_limit"),
)

BROWSER_PATH = "test/e2e/pr_eq_hyperliquid_contract.spec.ts"
BROWSER_TITLE = "PR-EQ Settings keeps builder-scoped Hyperliquid endpoint and wallet/vault evidence non-live"


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


def require_incomplete_queue_head(doc: str, queue: str) -> None:
    matched = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    if not matched:
        fail("local queue must retain a PR roadmap item at its head")
    head = matched.group(1)
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{head} ")), None)
    if row is None or "✅ 完成" in row:
        fail(f"local queue head must be an incomplete roadmap row: {head}")


def check(root: Path) -> None:
    doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{PR_TITLE}`")), None)
    if row is None or "✅ 完成" not in row or "剩余：无。" not in row or row.count(VERIFY_ANCHOR) != 1:
        fail("roadmap row must be complete, remaining-none and bound to the destructive gate")
    queue = doc[doc.index("### 🟡 6.5"):]
    for completed in ("PR-CE", "PR-AA", "PR-AB"):
        if re.search(rf"(?m)^\d+\.\s+\*\*{completed}\b", queue):
            fail(f"completed {completed} remains in the queue")
    require_incomplete_queue_head(doc, queue)
    queue_items = re.findall(r"(?m)^\d+\.\s+\*\*", queue)
    if not queue_items:
        fail("local queue plus external pool must not be empty")

    for successor in ("PR-EQ", "PR-ED", "PR-EG", "PR-DG", "PR-DA"):
        successor_row = next((line for line in doc.splitlines() if line.startswith(f"| `{successor} ")), "")
        if "✅ 完成" not in successor_row or "剩余：无" not in successor_row:
            fail(f"completed successor authority drifted: {successor}")

    history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if HISTORY_HEADING not in history:
        fail("PR-CE closure appendix is missing")
    for url in OFFICIAL_DOCS:
        if url not in history:
            fail(f"official Hyperliquid source is missing from closure history: {url}")

    selected = [item for item in rows(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv") if item["pr_id"] == "PR-CE"]
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

    support = require_marker(
        "crates/exchange/src/adapters/hyperliquid_support.rs",
        "pub async fn credential_relation",
        "main_account_owner",
        '"type": "userAbstraction"',
        '"type": "userDexAbstraction"',
        "pub async fn spot_balance_truth",
        'json!({"type": request_type, "user": user, "dex": dex})',
    )
    if support.count('"type": "userDexAbstraction"') != 1:
        fail("userDexAbstraction request must remain singular and explicit")

    probes = require_marker(
        "crates/api/src/services/venue_credentials/validation/hyperliquid_probes.rs",
        'kind: "account_signer_vault_relation".into()',
        'kind: "account_abstraction".into()',
        '"account_mode_read"',
        '"perp_margin_read"',
        '"spot_truth_read"',
        "each source is retained independently",
    )
    if "merged_probe_status(relation.status, abstraction.status)" not in probes:
        fail("Hyperliquid account mode no longer fails closed across relation and abstraction")
    require_marker(
        "crates/api/src/services/venue_credentials/validation/hyperliquid_probes/probe_support.rs",
        "validate_relation",
        "main_account_owner",
        "derived signer=",
        "vaultDetails leader evidence",
    )

    require_marker(
        "crates/api/src/services/venue_credentials/validation/venues/hyperliquid.rs",
        "optional_hyperliquid_account_relation_probe(adapter.credential_relation())",
        "optional_hyperliquid_abstraction_probe(adapter.account_abstraction_state())",
        'optional_hyperliquid_account_read_probes(adapter.get_account_read(Some("USDC")))',
        "probes.extend(account_read)",
    )
    require_marker(
        "crates/exchange/src/live.rs",
        "pub struct VenueAccountRead",
        "pub issues: Vec<VenueAccountReadIssue>",
        "pub struct VenueAccountReadIssue",
    )
    account_read = require_marker(
        "crates/exchange/src/adapters/hyperliquid_account_read.rs",
        "tokio::join!",
        'const PERP_MARGIN_OPERATION: &str = "perp_margin"',
        'const SPOT_TRUTH_OPERATION: &str = "spot_truth"',
        'const SPOT_TRUTH_VENUE: &str = "hyperliquid:spot"',
    )
    if "tokio::try_join!" in account_read:
        fail("partial Hyperliquid account read regressed to all-or-nothing try_join")
    require_marker(
        "crates/api/src/trading_service/live_adapters/router.rs",
        "RouteFailure::from_account_read_issue",
        "rows.extend(read.balances)",
        "summaries.extend(read.summaries)",
    )
    require_marker(
        "crates/api/src/trading_service/live_adapters/failures.rs",
        "from_account_read_issue(issue: exchange::VenueAccountReadIssue)",
    )

    require_marker(
        "crates/api/src/services/venue_credentials/specs.rs",
        "余额读取地址（主账户 / 子账户）",
        "已授权 API / Agent 钱包私钥",
        "Vault 执行地址（可选）",
    )
    require_marker(
        "frontend/src/panels/modules/settings/tabs/venue_credentials/validation/supplemental.rs",
        "supplemental_validation_row",
        "data-validation-probe",
        '"account_abstraction" => "账户抽象"',
        '"perp_margin_read" => "Perp Margin"',
        '"spot_truth_read" => "Spot Truth"',
    )

    ws = require_marker(
        "crates/exchange/src/adapters/hyperliquid_ws_trade.rs",
        "pub struct HyperliquidSignerSessionHealth",
        "pub fn hyperliquid_signer_session_health",
        "pub(super) fn signer_nonce_key(network: HyperliquidNetwork, private_key: &str)",
        'format!("{SIGNER_OWNERSHIP_BOUNDARY}|{network:?}|{signer}")',
        "record_signer_session_result(cfg, nonce, result.as_ref().err())",
        "last_nonce: nonce",
        "last_error: error.map(ToString::to_string)",
    )
    nonce_fn = re.search(r"pub\(super\) fn signer_nonce_key\([\s\S]*?\n}\n", ws)
    if nonce_fn is None or "vault" in nonce_fn.group(0).lower():
        fail("nonce key must be scoped only by signer and network")

    require_marker(
        "crates/api/src/services/venue_operation_health/snapshot/part_21.rs",
        "hyperliquid_signer_session_health()",
        "OP_PRIVATE_WS_SESSION",
        "last_nonce={}",
        "last_error={}",
        OFFICIAL_DOCS[1],
    )

    invalid_test = re.compile(r"#\[\s*(?:ignore|should_panic)|#\[\s*cfg_attr[^\]]*ignore")
    for relative, test_name in RUNNABLE:
        source = (root / relative).read_text(encoding="utf-8")
        match = re.search(rf"#\[(?:tokio::)?test\][\s\S]{{0,220}}?(?:async\s+)?fn\s+{re.escape(test_name)}\b", source)
        if match is None or invalid_test.search(source[max(0, match.start() - 180):match.end()]):
            fail(f"runnable test is missing or skipped: {relative}:{test_name}")

    browser = require_marker(
        BROWSER_PATH,
        BROWSER_TITLE,
        "derived signer=",
        'data-validation-probe="account_abstraction"',
        'data-validation-probe="perp_margin_read"',
        'data-validation-probe="spot_truth_read"',
        "userAbstraction=default",
        "userDexAbstraction=xyz",
    )
    escaped = re.escape(BROWSER_TITLE)
    if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', browser):
        fail("PR-CE browser proof is skipped")
    if not re.search(rf'test\(\s*["\']{escaped}["\']', browser):
        fail("PR-CE browser proof is missing")

    repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    if repo_gate.count("check_pr_ce_completion.sh") != 2:
        fail("repo gate must execute PR-CE in docs and full scopes")


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


def replace_queue_head(text: str, replacement: str) -> str:
    queue_start = text.index("### 🟡 6.5")
    queue, count = re.subn(
        r"(?m)^1\.\s+\*\*PR-[A-Z]+\b",
        f"1. **{replacement}",
        text[queue_start:],
        count=1,
    )
    return text if count != 1 else text[:queue_start] + queue


def self_test(source_root: Path) -> None:
    paths = {
        "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
        "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        "docs/audit_history/PRODUCT_AUDIT_HISTORY.md",
        "scripts/verify_repo_gates.sh",
        "crates/api/src/services/venue_credentials/validation/hyperliquid_probes.rs",
        "crates/api/src/trading_service/live_adapters/router.rs",
        BROWSER_PATH,
    }
    paths.update(artifact for artifact, _ in EVIDENCE.values())
    paths.update(relative for relative, _ in RUNNABLE)
    with tempfile.TemporaryDirectory(prefix="crossline-pr-ce-completion-") as temp:
        test_root = Path(temp) / "repo"
        for relative in paths:
            target = test_root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_root / relative, target)
        global root
        root = test_root
        check(root)
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace(f"| `{PR_TITLE}` | ✅ 完成 |", f"| `{PR_TITLE}` | 🟡 部分完成 |", 1), "a downgraded roadmap row")
        assert_rejected(root, "docs/PRODUCT_AUDIT_EVIDENCE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("PR-CE\taccount-mode-probe-matrix\t")) + "\n", "an incomplete evidence matrix")
        assert_rejected(root, "crates/exchange/src/adapters/hyperliquid_account_read.rs", lambda text: text.replace("tokio::join!", "tokio::try_join!", 1), "an all-or-nothing account read")
        assert_rejected(root, "crates/exchange/src/adapters/hyperliquid_support.rs", lambda text: text.replace('"type": "userDexAbstraction"', '"type": "removedDexAbstraction"', 1), "a missing dex abstraction request")
        assert_rejected(root, BROWSER_PATH, lambda text: text.replace("derived signer=", "signer="), "missing derived signer evidence")
        assert_rejected(root, "crates/exchange/src/adapters/hyperliquid_ws_trade.rs", lambda text: text.replace('format!("{SIGNER_OWNERSHIP_BOUNDARY}|{network:?}|{signer}")', 'format!("{SIGNER_OWNERSHIP_BOUNDARY}|{network:?}|{signer}|vault")', 1), "a vault-split nonce key")
        assert_rejected(root, "crates/api/src/services/venue_operation_health/snapshot/part_21.rs", lambda text: text.replace("last_nonce={}", "nonce={}"), "missing last nonce health")
        assert_rejected(root, BROWSER_PATH, lambda text: text.replace(f'test("{BROWSER_TITLE}"', f'test.skip("{BROWSER_TITLE}"', 1), "a skipped browser proof")
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-CE stale**", 1), "completed PR-CE returned to the queue")
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: replace_queue_head(text, "PR-AB"), "completed PR-AB returned to the queue head")
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: replace_queue_head(text, "PR-ZZ"), "an untracked local queue head")
        assert_rejected(root, "docs/PRODUCT_AUDIT_COVERAGE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("crates/exchange/src/adapters/hyperliquid_account_read.rs\t")) + "\n", "missing exact coverage")
        assert_rejected(root, "scripts/verify_repo_gates.sh", lambda text: text.replace("check_pr_ce_completion.sh", "removed_pr_ce_completion.sh", 1), "single-scope repo wiring")
    print("PR-CE completion destructive self-test passed")


try:
    if mode == "--self-test":
        self_test(root)
    else:
        check(root)
        print(f"OK PR-CE static contract ({len(EVIDENCE)} evidence types; {len(RUNNABLE)} runnable anchors; 1 browser anchor)")
except (OSError, KeyError, ValueError) as exc:
    raise SystemExit(f"PR-CE completion gate failed: {exc}") from exc
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

if [[ "${PR_CE_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test "$ROOT/test/e2e/pr_eq_hyperliquid_contract.spec.ts" --list >/dev/null
fi

if [[ "${PR_CE_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_pr_eq_completion.sh"
  PR_ED_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ed_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  PR_DG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dg_completion.sh"
  PR_DA_SKIP_TESTS=1 PR_DA_SKIP_UPSTREAM=1 PR_DA_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_da_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_CE_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test hyperliquid_test --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib signer_nonce_key_distinguishes_network_and_is_signer_scoped --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib signer_session_health_records_scope_nonce_and_last_error --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api hyperliquid_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api live_router_balances_keep_rows_and_record_inner_source_issue --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib venue_credentials --no-fail-fast
  CI=1 npx playwright test "$ROOT/test/e2e/pr_eq_hyperliquid_contract.spec.ts" \
    --grep "$BROWSER_TITLE" --workers=1
fi

printf 'PR-CE Hyperliquid account mode and agent evidence completion passed\n'
