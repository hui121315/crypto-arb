#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

python3 - "$ROOT" "$MODE" <<'PY'
from __future__ import annotations

from pathlib import Path
import csv
import os
import re
import shutil
import subprocess
import sys
import tempfile


PR_TITLE = "PR-CT Gate Official Futures Order Semantics"
VERIFY_ANCHOR = "`bash scripts/check_pr_ct_completion.sh --self-test`"
EVIDENCE = {
    "official-place-registry": (
        "crates/exchange/src/venue_spec.rs",
        "gate_place_order_evidence_uses_recorded_fixture_metadata",
    ),
    "official-status-registry": (
        "crates/exchange/src/venue_spec.rs",
        "gate_get_order_evidence_uses_recorded_fixture_metadata",
    ),
    "operation-evidence-matrix": (
        "scripts/exchange_operation_evidence_matrix.tsv",
        "check_exchange_operation_evidence_matrix.sh",
    ),
    "native-instrument-spec": (
        "crates/exchange/src/adapters/gate_contracts_tests.rs",
        "official_fixture_closes_native_identity_and_contract_spec",
    ),
    "order-compiler": (
        "crates/exchange/src/adapters/gate_trade_data_tests.rs",
        "place_order_converts_base_qty_to_contracts",
    ),
    "limit-tif-contract": (
        "crates/exchange/src/adapters/gate_trade_data_tests.rs",
        "limit_order_uses_requested_official_ioc_or_fok_tif",
    ),
    "post-only-contract": (
        "crates/exchange/src/adapters/gate_trade_data_tests.rs",
        "post_only_uses_poc_tif",
    ),
    "reduce-only-contract": (
        "crates/exchange/src/adapters/gate_trade_data_tests.rs",
        "reduce_only_uses_official_optional_payload_shape",
    ),
    "market-zero-ioc-contract": (
        "crates/exchange/src/adapters/gate_trade_data_tests.rs",
        "market_order_uses_official_zero_price_ioc_shape",
    ),
    "client-order-id-contract": (
        "crates/exchange/src/adapters/gate_trade_data_tests.rs",
        "gate_text_compacts_long_or_invalid_ids_to_official_policy",
    ),
    "ack-identity-contract": (
        "crates/exchange/src/adapters/gate_trade_data_tests.rs",
        "gate_place_order_ack_parses_official_fixture",
    ),
    "order-status-http-boundary": (
        "crates/exchange/src/adapters/gate_private_rest.rs",
        "order_row_returns_none_on_404",
    ),
    "terminal-parser-contract": (
        "crates/exchange/src/adapters/gate_private_data_tests.rs",
        "gate_get_order_ioc_fixture_preserves_partial_fill_as_terminal_cancel",
    ),
    "numeric-finality-query": (
        "crates/exchange/src/adapters/gate_tests.rs",
        "numeric_order_finality_uses_order_id_and_enriches_fill_fees",
    ),
    "missing-order-unknown-finality": (
        "crates/exchange/src/adapters/gate_tests.rs",
        "numeric_order_absence_remains_unknown_without_fill_probe",
    ),
    "run-finality-problem": (
        "crates/api/src/services/run_finality/tests.rs",
        "finality_remote_missing_problem_carries_order_context",
    ),
    "private-ws-terminal-finality": (
        "crates/api/src/trading_service/private_ws_mapper/tests/cases_gate_account.rs",
        "gate_terminal_orders_map_finality_by_exchange_id",
    ),
    "fill-fee-evidence": (
        "crates/exchange/src/adapters/gate_fill_evidence_tests.rs",
        "parses_official_my_trades_fixture_without_combining_fee_units",
    ),
    "venue-capability-matrix": (
        "crates/exchange/src/venue_capability.rs",
        "non_native_market_compilers_remain_explicit",
    ),
    "hedge-ticket-compiler": (
        "crates/api/src/routers/arbitrage/hedge_tests/compile_basic.rs",
        "order_compile_plan_marks_gate_market_as_price_zero_ioc",
    ),
    "settings-capability-copy": (
        "frontend/src/panels/modules/settings/tabs/adapters/venue_capabilities.rs",
        "venue_matrix_copy_keeps_non_native_market_and_finality_semantics_visible",
    ),
    "settings-browser": (
        "test/e2e/pr_es_venue_capability_matrix.spec.ts",
        "test:e2e:pr-es",
    ),
    "successor-authorities": (
        "scripts/check_pr_er_completion.sh",
        "check_pr_er_completion.sh",
    ),
    "repo-gate-wiring": (
        "scripts/verify_repo_gates.sh",
        "check_pr_ct_completion.sh",
    ),
    "completion-governance": (
        "scripts/check_pr_ct_completion.sh",
        "check_pr_ct_completion.sh --self-test",
    ),
}
AUTHORITIES = (
    "PR-ER Gate Futures Order, Account & Private WS Semantics Contract",
    "PR-ES Venue Capability Matrix & Order Compiler Contract",
    "PR-BW ExecutionRun Finality & ActionState Snapshot",
    "PR-BX VenueRuntimeHealth & Settings Diagnostics",
    "PR-BZ Exchange Official Evidence Registry & Fixture Gate",
)
RUST_ANCHORS = (
    ("crates/exchange/src/adapters/gate_trade_data_tests.rs", "place_order_converts_base_qty_to_contracts"),
    ("crates/exchange/src/adapters/gate_trade_data_tests.rs", "limit_order_uses_requested_official_ioc_or_fok_tif"),
    ("crates/exchange/src/adapters/gate_trade_data_tests.rs", "limit_order_rejects_non_official_gtx_tif"),
    ("crates/exchange/src/adapters/gate_trade_data_tests.rs", "post_only_uses_poc_tif"),
    ("crates/exchange/src/adapters/gate_trade_data_tests.rs", "reduce_only_uses_official_optional_payload_shape"),
    ("crates/exchange/src/adapters/gate_trade_data_tests.rs", "market_order_uses_official_zero_price_ioc_shape"),
    ("crates/exchange/src/adapters/gate_trade_data_tests.rs", "gate_text_compacts_long_or_invalid_ids_to_official_policy"),
    ("crates/exchange/src/adapters/gate_trade_data_tests.rs", "gate_place_order_ack_parses_official_fixture"),
    ("crates/exchange/src/adapters/gate_private_rest.rs", "order_row_returns_none_on_404"),
    ("crates/exchange/src/adapters/gate_private_rest.rs", "order_row_surfaces_http_error_on_server_error"),
    ("crates/exchange/src/adapters/gate_private_rest.rs", "order_row_parses_official_success_envelope"),
    ("crates/exchange/src/adapters/gate_private_data_tests.rs", "gate_get_order_ioc_fixture_preserves_partial_fill_as_terminal_cancel"),
    ("crates/exchange/src/adapters/gate_private_data_tests.rs", "parse_finished_order_maps_all_documented_non_fill_reasons_to_canceled"),
    ("crates/exchange/src/adapters/gate_private_data_tests.rs", "parse_open_order_surfaces_client_order_id_and_reduce_only"),
    ("crates/exchange/src/adapters/gate_tests.rs", "numeric_order_finality_uses_order_id_and_enriches_fill_fees"),
    ("crates/exchange/src/adapters/gate_tests.rs", "numeric_order_absence_remains_unknown_without_fill_probe"),
    ("crates/api/src/trading_service/tests/reconcile/finality/part_01.rs", "refresh_order_state_prefers_numeric_exchange_order_id"),
    ("crates/api/src/trading_service/tests/reconcile/finality/part_01.rs", "cancel_keeps_cancel_requested_when_order_query_returns_none"),
    ("crates/api/src/services/run_finality/tests.rs", "finality_remote_missing_problem_carries_order_context"),
    ("crates/api/src/trading_service/private_ws_mapper/tests/cases_gate_account.rs", "gate_terminal_orders_map_finality_by_exchange_id"),
    ("crates/exchange/src/adapters/gate_fill_evidence_tests.rs", "parses_official_my_trades_fixture_without_combining_fee_units"),
    ("crates/exchange/src/venue_capability.rs", "non_native_market_compilers_remain_explicit"),
    ("crates/api/src/routers/arbitrage/hedge_tests/compile_basic.rs", "order_compile_plan_marks_gate_market_as_price_zero_ioc"),
    ("crates/api/src/routers/arbitrage/hedge_tests/compile_basic.rs", "order_compile_plan_blocks_gate_limit_gtx_until_post_only_is_used"),
    ("crates/api/src/routers/arbitrage/hedge_tests/compile_basic.rs", "order_compile_plan_allows_gate_limit_ioc_and_fok_without_blockers"),
    ("crates/api/src/routers/arbitrage/hedge_tests/compile_basic.rs", "order_compile_plan_exposes_gate_limit_time_in_force_options"),
    ("frontend/src/panels/modules/settings/tabs/adapters/venue_capabilities.rs", "venue_matrix_copy_keeps_non_native_market_and_finality_semantics_visible"),
    ("crates/exchange/src/venue_spec.rs", "gate_get_order_evidence_uses_recorded_fixture_metadata"),
    ("crates/exchange/src/venue_spec.rs", "gate_place_order_evidence_uses_recorded_fixture_metadata"),
)
BROWSER_TITLE = "PR-ES Settings renders all venue compiler contracts without credential filtering"
COVERAGE_PATHS = tuple(dict.fromkeys(
    artifact for artifact, _ in EVIDENCE.values()
    if not artifact.startswith("docs/")
))
CLOSURE_PATHS = tuple(dict.fromkeys((
    "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
    "docs/audit_history/PRODUCT_AUDIT_HISTORY.md",
    "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
    "docs/PRODUCT_AUDIT_COVERAGE.tsv",
    "scripts/check_pr_ct_completion.sh",
    "scripts/verify_repo_gates.sh",
    "scripts/check_exchange_operation_evidence_matrix.sh",
    "scripts/exchange_operation_evidence_matrix.tsv",
    "scripts/check_pr_er_completion.sh",
    "scripts/check_pr_es_completion.sh",
    "scripts/check_pr_bw_completion.sh",
    "scripts/check_pr_bx_completion.sh",
    "scripts/check_pr_bz_completion.sh",
    "crates/exchange/src/venue_spec.rs",
    "crates/exchange/src/venue_capability.rs",
    "crates/exchange/src/adapters/gate.rs",
    "crates/exchange/src/adapters/gate_tests.rs",
    "crates/exchange/src/adapters/gate_contracts_tests.rs",
    "crates/exchange/src/adapters/gate_trade_data.rs",
    "crates/exchange/src/adapters/gate_trade_data_tests.rs",
    "crates/exchange/src/adapters/gate_private_rest.rs",
    "crates/exchange/src/adapters/gate_private_data_tests.rs",
    "crates/exchange/src/adapters/gate_fill_evidence_tests.rs",
    "crates/api/src/services/run_finality/tests.rs",
    "crates/api/src/trading_service/tests/reconcile/finality/part_01.rs",
    "crates/api/src/trading_service/private_ws_mapper/tests/cases_gate_account.rs",
    "crates/api/src/routers/arbitrage/hedge_tests/compile_basic.rs",
    "frontend/src/panels/modules/settings/tabs/adapters/venue_capabilities.rs",
    "test/e2e/pr_es_venue_capability_matrix.spec.ts",
)))


def fail(message: str) -> None:
    raise ValueError(message)


def tsv_rows(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def roadmap_row(doc: str, title: str) -> str:
    rows = [line for line in doc.splitlines() if line.startswith(f"| `{title}`")]
    if len(rows) != 1:
        fail(f"expected one roadmap row for {title}")
    return rows[0]


def require_incomplete_queue_head(doc: str) -> None:
    queue = doc[doc.index("### 🟡 6.5"):]
    matched = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    if not matched:
        fail("local queue must retain a PR roadmap item at its head")
    head = matched.group(1)
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{head} ")), None)
    if row is None or "✅ 完成" in row:
        fail(f"local queue head must be an incomplete roadmap row: {head}")


def require_runnable_anchor(root: Path, relative: str, name: str) -> None:
    source = (root / relative).read_text(encoding="utf-8")
    match = re.search(rf"(?m)^\s*(?:async\s+)?fn\s+{re.escape(name)}\s*\(", source)
    if not match:
        fail(f"missing runnable anchor {relative}::{name}")
    prefix = source[max(0, match.start() - 600):match.start()]
    attrs = "\n".join(
        line for line in prefix.splitlines()[-14:] if line.strip().startswith("#[")
    )
    if not re.search(r"#\[\s*(?:tokio::)?test(?:\]|\()", attrs):
        fail(f"anchor is not a test {relative}::{name}")
    if re.search(r"#\[\s*(?:ignore|should_panic)", attrs):
        fail(f"anchor is skipped or conditional {relative}::{name}")


def check(root: Path) -> None:
    doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    row = roadmap_row(doc, PR_TITLE)
    if "✅ 完成" not in row or "剩余：无。" not in row or row.count(VERIFY_ANCHOR) != 1:
        fail("PR-CT roadmap row must be complete, remaining-none and bound to the self-test")
    for authority in AUTHORITIES:
        authority_row = roadmap_row(doc, authority)
        if "✅ 完成" not in authority_row or "剩余：无" not in authority_row:
            fail(f"successor authority is incomplete: {authority}")

    queue = doc[doc.index("### 🟡 6.5"):]
    if re.search(r"(?m)^\d+\.\s+\*\*PR-CT\b", queue):
        fail("completed PR-CT remains in the local queue")
    require_incomplete_queue_head(doc)
    queue_items = re.findall(r"(?m)^\d+\.\s+\*\*", queue)
    if not queue_items:
        fail("local queue plus external pool must not be empty")
    external = queue.split("**外部等待池", 1)
    if len(external) != 2 or "PR-CT" not in external[1] or "live" not in external[1].lower():
        fail("real Gate live capture must remain an explicit external boundary")

    history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if "## 2026-07-16 PR-CT Gate Official Futures Order Semantics Closure" not in history:
        fail("PR-CT history closure appendix is missing")

    selected = [
        item for item in tsv_rows(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv")
        if item["pr_id"] == "PR-CT"
    ]
    indexed = {item["evidence_type"]: item for item in selected}
    if len(indexed) != len(selected) or set(indexed) != set(EVIDENCE):
        fail(f"evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for evidence_type, (artifact, command_anchor) in EVIDENCE.items():
        item = indexed[evidence_type]
        if item["artifact"] != artifact or command_anchor not in item["command"]:
            fail(f"evidence artifact or command drifted: {evidence_type}")
        if not (root / artifact).is_file() or not item["notes"].strip():
            fail(f"evidence artifact or notes missing: {evidence_type}")
    required_notes = {
        "reduce-only-contract": "false_omitted=true",
        "missing-order-unknown-finality": "404_not_final=true",
        "run-finality-problem": "remote_missing_is_problem=true",
        "venue-capability-matrix": "ack_not_final=true",
        "successor-authorities": "live_capture_external=true",
    }
    for evidence_type, marker in required_notes.items():
        if marker not in indexed[evidence_type]["notes"]:
            fail(f"evidence boundary marker missing: {evidence_type}:{marker}")

    for relative, name in RUST_ANCHORS:
        require_runnable_anchor(root, relative, name)

    browser = (root / "test/e2e/pr_es_venue_capability_matrix.spec.ts").read_text(encoding="utf-8")
    if re.search(r"\b(?:test|describe)\.(?:skip|fixme)|FIXME", browser):
        fail("Gate Settings browser evidence contains skip/fixme")
    if f'test("{BROWSER_TITLE}"' not in browser:
        fail("Gate Settings browser anchor is missing")
    for marker in ("Price-zero IOC 市价", "not.toContainText(\"GTX\")", "ACK 非终态"):
        if marker not in browser:
            fail(f"Gate Settings browser assertion missing: {marker}")

    matrix_rows = tsv_rows(root / "scripts/exchange_operation_evidence_matrix.tsv")
    gate_rows = [item for item in matrix_rows if item["venue"] == "Gate"]
    if len(gate_rows) != 1:
        fail("operation evidence matrix must contain exactly one Gate row")
    gate = gate_rows[0]
    expected = {
        "rest_trade_write_order_ack": "recorded",
        "rest_private_order_status": "recorded",
        "ws_live_write_path": "recorded_place_cancel",
        "ws_private_stream_evidence": "recorded",
        "finality_boundary": "ack_not_final",
    }
    for key, value in expected.items():
        if gate[key] != value:
            fail(f"Gate operation evidence matrix drifted: {key}={gate[key]!r}")

    compiler = (root / "crates/exchange/src/adapters/gate_trade_data.rs").read_text(encoding="utf-8")
    for marker in ('tif: "poc"', 'price: "0".into()', 'tif: "ioc"', "intent.reduce_only.then_some(true)"):
        if marker not in compiler:
            fail(f"Gate compiler marker missing: {marker}")
    finality = (root / "crates/exchange/src/venue_capability.rs").read_text(encoding="utf-8")
    for marker in ("ack_is_final: false", '"gate" => vec![TimeInForce::Ioc, TimeInForce::Fok, TimeInForce::Gtc]', "VenueOrderKind::PriceZeroIoc"):
        if marker not in finality:
            fail(f"Gate capability/finality marker missing: {marker}")

    coverage = {item["file"]: item for item in tsv_rows(root / "docs/PRODUCT_AUDIT_COVERAGE.tsv")}
    for relative in COVERAGE_PATHS:
        item = coverage.get(relative)
        if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
            fail(f"exact coverage missing for {relative}")

    repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    if repo_gate.count("check_pr_ct_completion.sh") != 2:
        fail("repo gate must execute PR-CT exactly once in docs and all scopes")


def run_fixture(root: Path, expect_pass: bool) -> None:
    env = os.environ.copy()
    env.update({
        "PR_CT_SKIP_TESTS": "1",
        "PR_CT_SKIP_UPSTREAM": "1",
        "PR_CT_SKIP_BROWSER_LIST": "1",
    })
    result = subprocess.run(
        ["bash", "scripts/check_pr_ct_completion.sh"],
        cwd=root,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )
    if expect_pass != (result.returncode == 0):
        detail = (result.stderr or result.stdout).strip()
        fail(f"destructive self-test expectation failed: {detail}")


def self_test(root: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="crossline-pr-ct-completion-") as temp:
        fixture = Path(temp) / "repo"
        for relative in CLOSURE_PATHS:
            source = root / relative
            target = fixture / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        run_fixture(fixture, True)

        evidence = fixture / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        evidence_baseline = evidence.read_text(encoding="utf-8")
        for evidence_type in EVIDENCE:
            evidence.write_text(
                "\n".join(
                    line for line in evidence_baseline.splitlines()
                    if not line.startswith(f"PR-CT\t{evidence_type}\t")
                ) + "\n",
                encoding="utf-8",
            )
            run_fixture(fixture, False)
        evidence.write_text(evidence_baseline, encoding="utf-8")

        doc = fixture / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc_baseline = doc.read_text(encoding="utf-8")
        doc.write_text(
            doc_baseline.replace(
                f"| `{PR_TITLE}` | ✅ 完成 |",
                f"| `{PR_TITLE}` | 🟡 部分完成 |",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        doc.write_text(
            doc_baseline.replace(
                "### 🟡 6.5 下一步执行队列",
                "### 🟡 6.5 下一步执行队列\n\n1. **PR-CT Gate Official Futures Order Semantics** — stale completed row",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        doc.write_text(doc_baseline, encoding="utf-8")

        reduce_only = fixture / "crates/exchange/src/adapters/gate_trade_data_tests.rs"
        reduce_baseline = reduce_only.read_text(encoding="utf-8")
        reduce_only.write_text(
            reduce_baseline.replace(
                "#[test]\nfn reduce_only_uses_official_optional_payload_shape",
                "#[test]\n#[ignore]\nfn reduce_only_uses_official_optional_payload_shape",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        reduce_only.write_text(reduce_baseline, encoding="utf-8")

        capability = fixture / "crates/exchange/src/venue_capability.rs"
        capability_baseline = capability.read_text(encoding="utf-8")
        capability.write_text(
            capability_baseline.replace("ack_is_final: false", "ack_is_final: true", 1),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        capability.write_text(capability_baseline, encoding="utf-8")

        missing = fixture / "crates/exchange/src/adapters/gate_tests.rs"
        missing_baseline = missing.read_text(encoding="utf-8")
        missing.write_text(
            missing_baseline.replace(
                "numeric_order_absence_remains_unknown_without_fill_probe",
                "numeric_order_absence_is_final",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        missing.write_text(missing_baseline, encoding="utf-8")

        matrix = fixture / "scripts/exchange_operation_evidence_matrix.tsv"
        matrix_baseline = matrix.read_text(encoding="utf-8")
        matrix_lines = matrix_baseline.splitlines()
        gate_index = next(index for index, line in enumerate(matrix_lines) if line.startswith("Gate\t"))
        matrix_lines[gate_index] = matrix_lines[gate_index].rsplit("\t", 1)[0] + "\tack_is_final"
        matrix.write_text("\n".join(matrix_lines) + "\n", encoding="utf-8")
        run_fixture(fixture, False)
        matrix.write_text(matrix_baseline, encoding="utf-8")

        browser = fixture / "test/e2e/pr_es_venue_capability_matrix.spec.ts"
        browser_baseline = browser.read_text(encoding="utf-8")
        browser.write_text(
            browser_baseline.replace(f'test("{BROWSER_TITLE}"', f'test.skip("{BROWSER_TITLE}"', 1),
            encoding="utf-8",
        )
        run_fixture(fixture, False)

    print("OK PR-CT completion destructive self-test")


try:
    root = Path(sys.argv[1])
    if sys.argv[2] == "--self-test":
        self_test(root)
    else:
        check(root)
        print(
            f"OK PR-CT static contract ({len(EVIDENCE)} evidence types; "
            f"{len(RUST_ANCHORS)} Rust/Wasm anchors; one non-skipping browser anchor)"
        )
except (OSError, ValueError) as error:
    raise SystemExit(f"PR-CT completion gate failed: {error}") from error
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

if [[ "${PR_CT_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test "$ROOT/test/e2e/pr_es_venue_capability_matrix.spec.ts" --list >/dev/null
fi

if [[ "${PR_CT_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_ES_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_es_completion.sh"
  PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bw_completion.sh"
  bash "$ROOT/scripts/check_pr_bx_completion.sh"
  PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bz_completion.sh"
  bash "$ROOT/scripts/check_exchange_operation_evidence_matrix.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_CT_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib gate_trade_data --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib numeric_order_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib gate_get_order --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api \
    refresh_order_state_prefers_numeric_exchange_order_id --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api \
    finality_remote_missing_problem_carries_order_context --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib \
    venue_matrix_copy_keeps_non_native_market_and_finality_semantics_visible --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-es -- --workers=1
fi

printf 'PR-CT Gate official futures order semantics completion passed\n'
