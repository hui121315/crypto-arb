#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
BROWSER_GREP='PR-DJ top status keeps four runtime scopes distinct and exposes circuit retry|PR-DJ Settings exposes typed HostGate and RateLimiter diagnostics|every always-on bearer route preserves browser auth and typed error boundaries|high-risk action and secret mutation persist correlated redacted audit pairs|opportunities websocket authenticates with ticket before subscribe'

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

python3 - "$ROOT" "$MODE" <<'PY'
from pathlib import Path
import csv
import json
import re
import shutil
import sys
import tempfile

root = Path(sys.argv[1])
mode = sys.argv[2]

PR_TITLE = "PR-AA Verification Gate & Evidence CI"
VERIFY_ANCHOR = "`bash scripts/check_pr_aa_completion.sh --self-test`"
HISTORY_HEADING = "## 2026-07-16 PR-AA Verification Gate and Evidence CI Closure"

EVIDENCE = {
    "successor-pr-bz": ("scripts/check_pr_bz_completion.sh", "check_pr_bz_completion.sh"),
    "successor-pr-fw-endpoint": ("scripts/check_exchange_evidence_debt.sh", "check_exchange_evidence_debt.sh"),
    "successor-pr-fw-operation": ("scripts/check_exchange_operation_evidence_matrix.sh", "check_exchange_operation_evidence_matrix.sh"),
    "successor-pr-dj": ("scripts/check_pr_dj_completion.sh", "check_pr_dj_completion.sh"),
    "successor-pr-dk": ("scripts/check_pr_dk_completion.sh", "check_pr_dk_completion.sh"),
    "successor-pr-ej": ("scripts/check_pr_ej_completion.sh", "check_pr_ej_completion.sh"),
    "canonical-operation-matrix": ("scripts/exchange_operation_evidence_matrix.tsv", "check_exchange_operation_evidence_matrix.sh"),
    "client-order-id-policy": ("crates/exchange/src/client_order_id_policy.rs", "client_order_id_policy"),
    "fill-ledger-context": ("crates/trading/src/ledger.rs", "records_external_fill_context_and_transport_metadata"),
    "workspace-clippy-contract": (".github/workflows/ci.yml", "cargo clippy --workspace --all-targets -- -D warnings"),
    "runtime-required-api-contract": ("scripts/verify_runtime_contracts_with_api.sh", "verify_runtime_contracts_with_api.sh"),
    "runtime-contract-probes": ("scripts/verify_runtime_contracts.sh", "verify_runtime_contracts.sh"),
    "api-security-contract-fixture": ("test/e2e/fixtures/route_runtime_policy.mjs", "route_runtime_policy"),
    "ci-product-gate": (".github/workflows/ci.yml", "browser-smoke"),
    "package-verification-surface": ("package.json", "test:e2e:product"),
    "repo-static-gate": ("scripts/verify_repo_gates.sh", "check_pr_aa_completion.sh"),
    "browser-runtime-smoke": ("test/e2e/pr_dj_runtime_gate.spec.ts", "test:e2e:pr-dj"),
    "browser-rest-security-smoke": ("test/e2e/route_registry.spec.ts", "test:e2e:pr-dk"),
    "browser-ws-auth-smoke": ("test/e2e/data_pipeline.spec.ts", "test:e2e:pr-dk"),
    "completion-governance": ("scripts/check_pr_aa_completion.sh", "check_pr_aa_completion.sh --self-test"),
}

RUNNABLE = (
    ("crates/exchange/src/client_order_id_policy.rs", "binance_policy_accepts_official_regex"),
    ("crates/exchange/src/client_order_id_policy.rs", "okx_policy_rejects_hyphenated_public_id"),
    ("crates/exchange/src/client_order_id_policy.rs", "bybit_policy_derives_unsupported_public_id"),
    ("crates/exchange/src/client_order_id_policy.rs", "bitget_policy_rejects_outside_official_regex"),
    ("crates/exchange/src/client_order_id_policy.rs", "gate_policy_compacts_to_official_text_shape"),
    ("crates/exchange/src/client_order_id_policy.rs", "htx_policy_derives_positive_numeric_client_order_id"),
    ("crates/exchange/src/client_order_id_policy.rs", "kucoin_policy_keeps_trimmed_client_oid_without_unverified_charset"),
    ("crates/exchange/src/client_order_id_policy.rs", "hyperliquid_policy_derives_128_bit_cloid_for_builder_venue"),
    ("crates/exchange/src/client_order_id_policy.rs", "hyperliquid_policy_normalizes_official_cloid_case"),
    ("crates/exchange/src/rest_registry.rs", "operation_matrix_rest_buckets_match_runtime_registry_projection"),
    ("crates/exchange/tests/ws_trading_specs_test.rs", "operation_matrix_boundaries_match_ws_registry_projection"),
    ("crates/exchange/tests/ws_trading_specs_test.rs", "operation_matrix_ws_fixtures_match_runtime_registry_evidence"),
    ("crates/api/src/routers/trading/tests/cases_registry.rs", "transport_registry_route_summary_matches_operation_matrix_tsv"),
    ("crates/api/src/routers/trading/tests/cases_registry.rs", "transport_registry_route_preserves_rest_operation_matrix_buckets"),
    ("crates/trading/src/ledger.rs", "records_external_fill_context_and_transport_metadata"),
)

BROWSERS = {
    "test/e2e/pr_dj_runtime_gate.spec.ts": (
        "PR-DJ top status keeps four runtime scopes distinct and exposes circuit retry",
        "PR-DJ Settings exposes typed HostGate and RateLimiter diagnostics",
    ),
    "test/e2e/route_registry.spec.ts": (
        "every always-on bearer route preserves browser auth and typed error boundaries",
        "high-risk action and secret mutation persist correlated redacted audit pairs",
    ),
    "test/e2e/data_pipeline.spec.ts": (
        "opportunities websocket authenticates with ticket before subscribe",
    ),
}


def fail(message: str) -> None:
    raise ValueError(message)


def read_rows(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def require_markers(relative: str, *markers: str) -> str:
    source = (root / relative).read_text(encoding="utf-8")
    for marker in markers:
        if marker not in source:
            fail(f"missing {relative} marker: {marker}")
    return source


def require_runnable(relative: str, test_name: str) -> None:
    source = (root / relative).read_text(encoding="utf-8")
    pattern = re.compile(
        rf"(?ms)(?P<attrs>(?:^\s*#\[[^\n]+\]\s*\n)+)\s*(?:async\s+)?fn\s+{re.escape(test_name)}\s*\("
    )
    match = pattern.search(source)
    if match is None:
        fail(f"runnable test is missing: {relative}:{test_name}")
    attrs = match.group("attrs")
    if "#[test]" not in attrs and "#[tokio::test" not in attrs:
        fail(f"test attribute is missing: {relative}:{test_name}")
    if re.search(r"#\[(?:ignore|should_panic)|#\[cfg(?:_attr)?\(", attrs):
        fail(f"runnable test is skipped or cfg-disabled: {relative}:{test_name}")


def require_incomplete_queue_head(doc: str, queue: str) -> None:
    matched = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    if not matched:
        fail("local queue must retain a PR roadmap item at its head")
    head = matched.group(1)
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{head} ")), None)
    if row is None or "✅ 完成" in row:
        fail(f"local queue head must be an incomplete roadmap row: {head}")


def check(current_root: Path) -> None:
    global root
    root = current_root
    doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{PR_TITLE}`")), None)
    if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
        fail("roadmap row must be completed with no local remainder")
    if row.count(VERIFY_ANCHOR) != 1:
        fail("roadmap verification must name the destructive completion gate")

    queue = doc[doc.index("### 🟡 6.5"):]
    for completed in ("PR-AA", "PR-AB"):
        if re.search(rf"(?m)^\d+\.\s+\*\*{completed}\b", queue):
            fail(f"completed {completed} remains in the local queue")
    require_incomplete_queue_head(doc, queue)

    successor_titles = (
        "PR-BZ Exchange Official Evidence Registry & Fixture Gate",
        "PR-FW Exchange Official Evidence, Adapter Schema Fixture & Trading Capability Gate",
        "PR-DJ Runtime Health Verification Gate & CI Evidence Contract",
        "PR-DK API Security Surface & High-Risk Audit Contract",
        "PR-EJ Documentation Authority & Verification Gate Contract",
    )
    for title in successor_titles:
        successor = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
        if successor is None or "✅ 完成" not in successor:
            fail(f"required successor authority is not complete: {title}")

    history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if HISTORY_HEADING not in history:
        fail("history closure appendix is missing")

    evidence_rows = [entry for entry in read_rows(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv") if entry["pr_id"] == "PR-AA"]
    indexed = {entry["evidence_type"]: entry for entry in evidence_rows}
    if len(indexed) != len(evidence_rows) or set(indexed) != set(EVIDENCE):
        fail(f"evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for evidence_type, (artifact, _) in EVIDENCE.items():
        evidence = indexed[evidence_type]
        if evidence["artifact"] != artifact or not (root / artifact).is_file():
            fail(f"{evidence_type} artifact drifted or is missing")
        if not evidence["command"].strip() or not evidence["notes"].strip():
            fail(f"{evidence_type} lacks command or notes")

    matrix_rows = read_rows(root / "scripts/exchange_operation_evidence_matrix.tsv")
    expected_venues = {"Binance", "Okx", "Bybit", "Bitget", "Gate", "Htx", "Kucoin", "Hyperliquid"}
    if len(matrix_rows) != 8 or {entry["venue"] for entry in matrix_rows} != expected_venues:
        fail("canonical operation matrix must contain exactly eight venues")
    for entry in matrix_rows:
        for column in (
            "rest_trade_write_order_ack",
            "rest_private_order_status",
            "rest_private_account_balance",
            "rest_private_account_position",
        ):
            if entry[column] != "recorded":
                fail(f"{entry['venue']} operation bucket is not recorded: {column}")
        if entry["finality_boundary"] != "ack_not_final":
            fail(f"{entry['venue']} ACK/finality boundary drifted")

    require_markers(
        "scripts/check_exchange_evidence_debt.sh",
        "parser_test",
        "request_builder_test",
        "schema_hash",
        "OK exchange evidence debt gate",
    )
    require_markers(
        "scripts/check_exchange_operation_evidence_matrix.sh",
        "WS evidence tests locked",
        "OK exchange operation evidence matrix gate",
    )
    runtime_wrapper = require_markers(
        "scripts/verify_runtime_contracts_with_api.sh",
        "ALLOW_RUNTIME_SKIP=0",
        "RUNTIME_DIR=\"$(mktemp -d",
        "trap cleanup EXIT",
        "start_api",
        "verify_runtime_contracts.sh",
    )
    if runtime_wrapper.count("ALLOW_RUNTIME_SKIP=0") != 2:
        fail("runtime wrapper must force no-skip in both execution branches")
    if "ALLOW_RUNTIME_SKIP=1" in runtime_wrapper:
        fail("runtime wrapper contains a skippable execution branch")
    runtime_contract = require_markers(
        "scripts/verify_runtime_contracts.sh",
        'ALLOW_RUNTIME_SKIP="${ALLOW_RUNTIME_SKIP:-0}"',
        'fail "base=$API_URL reason=health_unreachable"',
        "probe_metrics_contract",
        "probe_optional_opportunity_error_contract",
    )
    if "runtime contracts skipped" not in runtime_contract:
        fail("runtime contract explicit skip diagnostic is missing")
    require_markers(
        "test/e2e/fixtures/route_runtime_policy.mjs",
        "export const ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY = loadAlwaysOnBearerRouteRuntimePolicy();",
        "startRouteRuntimeFixture",
        "route runtime fixture inventory header drifted",
    )

    for relative, test_name in RUNNABLE:
        require_runnable(relative, test_name)

    for relative, titles in BROWSERS.items():
        source = (root / relative).read_text(encoding="utf-8")
        for title in titles:
            escaped = re.escape(title)
            if re.search(rf"test\.(?:skip|fixme)\(\s*['\"]{escaped}['\"]", source):
                fail(f"browser proof is skipped: {title}")
            if not re.search(rf"test\(\s*['\"]{escaped}['\"]", source):
                fail(f"browser proof is missing: {title}")

    package = json.loads((root / "package.json").read_text(encoding="utf-8"))
    scripts = package.get("scripts", {})
    product = scripts.get("test:e2e:product", "")
    for relative in BROWSERS:
        if product.count(relative) != 1:
            fail(f"product browser suite must include {relative} exactly once")
    if scripts.get("verify:repo") != "bash scripts/verify_repo_gates.sh":
        fail("package repo verification command drifted")
    if scripts.get("verify:runtime") != "bash scripts/verify_runtime_contracts_with_api.sh":
        fail("package runtime verification command drifted")

    ci = require_markers(
        ".github/workflows/ci.yml",
        "bash scripts/verify_repo_gates.sh",
        "RUNTIME_BUILD_API=0 bash scripts/verify_runtime_contracts_with_api.sh",
        "run: CI=1 npm run test:e2e:product",
        "cargo clippy --workspace --all-targets -- -D warnings",
        "cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings",
        "needs: [fmt, clippy, msrv, dev-msrv, test, runtime-contracts, frontend, browser-smoke, supply-chain]",
    )
    if ci.index("bash scripts/verify_repo_gates.sh") < ci.index("install ripgrep and jq"):
        fail("CI repo gate appears before its ripgrep dependency")

    repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    if repo_gate.count("check_pr_aa_completion.sh") != 2:
        fail("repo gate must execute PR-AA in docs and full scopes")

    coverage = {entry["file"]: entry for entry in read_rows(root / "docs/PRODUCT_AUDIT_COVERAGE.tsv")}
    for artifact, _ in EVIDENCE.values():
        if artifact == "package.json":
            continue
        if coverage.get(artifact, {}).get("coverage_status") != "exact":
            fail(f"exact coverage missing for {artifact}")


def assert_rejected(relative: str, transform, label: str) -> None:
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
        "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
        "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        "docs/audit_history/PRODUCT_AUDIT_HISTORY.md",
        "scripts/verify_repo_gates.sh",
    }
    paths.update(artifact for artifact, _ in EVIDENCE.values())
    paths.update(relative for relative, _ in RUNNABLE)
    paths.update(BROWSERS)
    with tempfile.TemporaryDirectory(prefix="crossline-pr-aa-completion-") as temp:
        test_root = Path(temp) / "repo"
        for relative in paths:
            target = test_root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_root / relative, target)
        check(test_root)
        assert_rejected("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace(f"| `{PR_TITLE}` | ✅ 完成 |", f"| `{PR_TITLE}` | 🟡 部分完成 |", 1), "a downgraded roadmap row")
        assert_rejected("docs/PRODUCT_AUDIT_EVIDENCE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("PR-AA\tclient-order-id-policy\t")) + "\n", "an incomplete evidence matrix")
        assert_rejected("scripts/exchange_operation_evidence_matrix.tsv", lambda text: text.replace("Binance\trecorded\trecorded\trecorded\trecorded\t", "Binance\tmissing\trecorded\trecorded\trecorded\t", 1), "a missing operation bucket")
        assert_rejected("crates/exchange/src/client_order_id_policy.rs", lambda text: text.replace("#[test]\n    fn binance_policy_accepts_official_regex", "#[test]\n    #[ignore]\n    fn binance_policy_accepts_official_regex", 1), "a skipped client-id policy test")
        assert_rejected("crates/trading/src/ledger.rs", lambda text: text.replace("#[test]\n    fn records_external_fill_context_and_transport_metadata", "#[test]\n    #[ignore]\n    fn records_external_fill_context_and_transport_metadata", 1), "a skipped fill-ledger context test")
        assert_rejected("scripts/verify_runtime_contracts_with_api.sh", lambda text: text.replace("ALLOW_RUNTIME_SKIP=0", "ALLOW_RUNTIME_SKIP=1", 1), "a skippable runtime wrapper")
        assert_rejected(".github/workflows/ci.yml", lambda text: text.replace("run: CI=1 npm run test:e2e:product", "run: CI=1 npm run test:e2e:data-pipeline", 1), "a narrowed CI browser suite")
        first_browser, first_titles = next(iter(BROWSERS.items()))
        assert_rejected(first_browser, lambda text: text.replace(f'test("{first_titles[0]}"', f'test.skip("{first_titles[0]}"', 1), "a skipped browser proof")
        assert_rejected("test/e2e/fixtures/route_runtime_policy.mjs", lambda text: text.replace("ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY", "REMOVED_BEARER_ROUTE_RUNTIME_POLICY", 1), "a detached API contract fixture")
        assert_rejected("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-AA stale**", 1), "completed PR-AA returned to the queue")
        assert_rejected("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("1. **PR-AC", "1. **PR-AB", 1), "completed PR-AB returned to the queue head")
        assert_rejected("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("1. **PR-AC", "1. **PR-ZZ", 1), "an untracked local queue head")
        assert_rejected("docs/PRODUCT_AUDIT_COVERAGE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("scripts/check_pr_aa_completion.sh\t")) + "\n", "missing exact coverage")
        assert_rejected("scripts/verify_repo_gates.sh", lambda text: text.replace("check_pr_aa_completion.sh", "removed_pr_aa_completion.sh", 1), "single-scope repo wiring")
    print("PR-AA completion destructive self-test passed")


try:
    if mode == "--self-test":
        self_test(root)
    else:
        check(root)
        print(f"OK PR-AA static contract ({len(EVIDENCE)} evidence types; {len(RUNNABLE)} runnable anchors; 5 browser anchors)")
except (OSError, KeyError, ValueError) as exc:
    raise SystemExit(f"PR-AA completion gate failed: {exc}") from exc
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

if [[ "${PR_AA_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test \
    "$ROOT/test/e2e/pr_dj_runtime_gate.spec.ts" \
    "$ROOT/test/e2e/route_registry.spec.ts" \
    "$ROOT/test/e2e/data_pipeline.spec.ts" \
    --list >/dev/null
fi

if [[ "${PR_AA_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bz_completion.sh"
  bash "$ROOT/scripts/check_exchange_evidence_debt.sh"
  bash "$ROOT/scripts/check_exchange_operation_evidence_matrix.sh"
  PR_DJ_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dj_completion.sh"
  PR_DK_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dk_completion.sh"
  PR_EJ_SKIP_DESTRUCTIVE=1 bash "$ROOT/scripts/check_pr_ej_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_AA_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib client_order_id_policy --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test ws_trading_specs_test operation_matrix_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api transport_registry_route_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p trading records_external_fill_context_and_transport_metadata --no-fail-fast
  bash "$ROOT/scripts/verify_runtime_contracts_with_api.sh"
  CI=1 npx playwright test \
    "$ROOT/test/e2e/pr_dj_runtime_gate.spec.ts" \
    "$ROOT/test/e2e/route_registry.spec.ts" \
    "$ROOT/test/e2e/data_pipeline.spec.ts" \
    --grep "$BROWSER_GREP" --workers=1
fi

printf 'PR-AA verification gate and evidence CI completion passed\n'
