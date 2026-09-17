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


PR_TITLE = "PR-CU OKX Official Trade Order Semantics"
SUCCESSOR_TITLE = "PR-CV Binance Official USD-M Order Semantics"
VERIFY_ANCHOR = "`bash scripts/check_pr_cu_completion.sh --self-test`"
EVIDENCE = {
    "shared-order-identity-plan": (
        "crates/api/src/services/hedge_preview/guards/identity.rs",
        "okx_identity_constraints_produce_execution_ready_plan",
    ),
    "identity-finality-fail-closed": (
        "crates/api/src/services/hedge_preview/guards/identity.rs",
        "okx_identity_plan_fails_closed_without_private_finality_evidence",
    ),
    "client-order-id-policy": (
        "crates/exchange/src/client_order_id_policy.rs",
        "okx_policy_rejects_hyphenated_public_id",
    ),
    "hedge-ticket-policy-binding": (
        "crates/api/src/routers/arbitrage/hedge_tests/compile_basic.rs",
        "order_compile_plan_exposes_client_order_id_policy",
    ),
    "optional-futures-payload-scope": (
        "crates/exchange/src/adapters/okx_trade_data_tests.rs",
        "futures_payload_omits_unscoped_optional_currency_and_tag_fields",
    ),
    "order-compiler-account-mode": (
        "crates/exchange/src/adapters/okx_trade_data_tests.rs",
        "long_short_mode_maps_open_and_close_pos_side",
    ),
    "limit-tif-contract": (
        "crates/exchange/src/adapters/okx_trade_data_tests.rs",
        "limit_ioc_and_fok_use_official_ord_type_with_px",
    ),
    "native-instrument-spec": (
        "crates/exchange/src/adapters/okx_instruments_tests.rs",
        "okx_instrument_rule_parses_official_swap_fixture",
    ),
    "rest-ack-contract": (
        "crates/exchange/src/adapters/okx_trade_data_ack_tests.rs",
        "okx_place_order_ack_parses_official_fixture",
    ),
    "rest-order-registry": (
        "crates/exchange/src/venue_spec.rs",
        "okx_place_order_evidence_uses_recorded_fixture_metadata",
    ),
    "private-ws-server-ack": (
        "crates/api/src/lifecycle/private_ws/plain_venues_tests.rs",
        "okx_private_ws_waits_for_all_server_acknowledgements",
    ),
    "private-ws-fill-fee": (
        "crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_03.rs",
        "okx_partial_fill_fixture_preserves_identity_fee_and_deduplicates",
    ),
    "private-ws-cancel-finality": (
        "crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_03.rs",
        "okx_cancel_fixture_is_final_only_after_private_order_event",
    ),
    "execution-run-authority": (
        "scripts/check_pr_ea_completion.sh",
        "check_pr_ea_completion.sh",
    ),
    "instrument-registry-authority": (
        "scripts/check_pr_eb_completion.sh",
        "check_pr_eb_completion.sh",
    ),
    "okx-venue-authority": (
        "scripts/check_pr_ek_completion.sh",
        "check_pr_ek_completion.sh",
    ),
    "operation-registry-authority": (
        "scripts/check_pr_bz_completion.sh",
        "check_pr_bz_completion.sh",
    ),
    "product-browser": (
        "test/e2e/data_pipeline.spec.ts",
        "test:e2e:pr-ek",
    ),
    "repo-gate-wiring": (
        "scripts/verify_repo_gates.sh",
        "verify_repo_gates.sh",
    ),
    "completion-governance": (
        "scripts/check_pr_cu_completion.sh",
        "check_pr_cu_completion.sh --self-test",
    ),
}
AUTHORITIES = (
    "PR-EA ExecutionRun Finality & ActionState Contract",
    "PR-EB Venue InstrumentSpec & Order Sizing Contract",
    "PR-EK OKX V5 Order, Account & Private WS Semantics Contract",
    "PR-BZ Exchange Official Evidence Registry & Fixture Gate",
)
RUST_ANCHORS = (
    ("crates/api/src/services/hedge_preview/guards/identity.rs", "okx_identity_constraints_produce_execution_ready_plan"),
    ("crates/api/src/services/hedge_preview/guards/identity.rs", "okx_identity_plan_fails_closed_without_private_finality_evidence"),
    ("crates/exchange/src/adapters/okx_trade_data_tests.rs", "futures_payload_omits_unscoped_optional_currency_and_tag_fields"),
    ("crates/exchange/src/adapters/okx_trade_data_tests.rs", "limit_ioc_and_fok_use_official_ord_type_with_px"),
    ("crates/exchange/src/adapters/okx_trade_data_tests.rs", "limit_gtx_uses_official_post_only_ord_type"),
    ("crates/exchange/src/adapters/okx_trade_data_tests.rs", "net_mode_sends_net_pos_side_and_reduce_only"),
    ("crates/exchange/src/adapters/okx_trade_data_tests.rs", "long_short_mode_maps_open_and_close_pos_side"),
    ("crates/exchange/src/adapters/okx_trade_data_tests.rs", "place_order_arg_validates_official_cl_ord_id_rule"),
    ("crates/exchange/src/adapters/okx_live_tests.rs", "okx_account_config_parses_official_fixture_position_mode"),
    ("crates/exchange/src/adapters/okx_instruments_tests.rs", "okx_instrument_rule_parses_official_swap_fixture"),
    ("crates/exchange/src/adapters/okx_trade_data_ack_tests.rs", "okx_place_order_ack_parses_official_fixture"),
    ("crates/exchange/src/adapters/okx_trade_data_ack_tests.rs", "okx_cancel_order_ack_parses_official_fixture"),
    ("crates/exchange/tests/okx_test.rs", "live_place_order_uses_official_instrument_contract_sizing"),
    ("crates/exchange/tests/okx_test.rs", "live_get_order_parse_error_is_not_none"),
    ("crates/api/src/lifecycle/private_ws/plain_venues_tests.rs", "okx_private_ws_waits_for_all_server_acknowledgements"),
    ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_03.rs", "okx_partial_fill_fixture_preserves_identity_fee_and_deduplicates"),
    ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_03.rs", "okx_cancel_fixture_is_final_only_after_private_order_event"),
    ("crates/api/src/routers/arbitrage/hedge_tests/compile_basic.rs", "order_compile_plan_exposes_client_order_id_policy"),
)
BROWSER_TITLE = "settings credentials surface local capture-shaped trading runtime 4/4 ok gate"
COVERAGE_PATHS = tuple(dict.fromkeys(
    artifact for artifact, _ in EVIDENCE.values() if not artifact.startswith("docs/")
))
CLOSURE_PATHS = tuple(dict.fromkeys((
    "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
    "docs/audit_history/PRODUCT_AUDIT_HISTORY.md",
    "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
    "docs/PRODUCT_AUDIT_COVERAGE.tsv",
    "scripts/check_pr_cu_completion.sh",
    *COVERAGE_PATHS,
    *(artifact for artifact, _ in RUST_ANCHORS),
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
        fail("PR-CU roadmap row must be complete, remaining-none and bound to the self-test")
    for authority in AUTHORITIES:
        authority_row = roadmap_row(doc, authority)
        if "✅ 完成" not in authority_row or "剩余：无" not in authority_row:
            fail(f"successor authority is incomplete: {authority}")

    queue = doc[doc.index("### 🟡 6.5"):]
    if re.search(r"(?m)^\d+\.\s+\*\*PR-CU\b", queue):
        fail("completed PR-CU remains in the local queue")
    require_incomplete_queue_head(doc)
    successor_row = roadmap_row(doc, SUCCESSOR_TITLE)
    successor_complete = "✅ 完成" in successor_row and "剩余：无。" in successor_row
    successor_queued = re.search(r"(?m)^1\.\s+\*\*PR-CV\b", queue) is not None
    if successor_complete and successor_queued:
        fail("completed PR-CV remains at the local queue head")
    if not successor_complete and not successor_queued:
        fail("incomplete PR-CV must be the next local queue head")
    external = queue.split("**外部等待池", 1)
    if len(external) != 2 or "PR-CU" not in external[1] or "live" not in external[1].lower():
        fail("real OKX live capture must remain an explicit external boundary")

    history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if "## 2026-07-16 PR-CU OKX Official Trade Order Semantics Closure" not in history:
        fail("PR-CU history closure appendix is missing")

    selected = [
        item for item in tsv_rows(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv")
        if item["pr_id"] == "PR-CU"
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
        "identity-finality-fail-closed": "identity_fail_closed=true",
        "optional-futures-payload-scope": "tgtCcy_spot_only=true",
        "private-ws-cancel-finality": "ack_not_final=true",
        "okx-venue-authority": "live_capture_external=true",
    }
    for evidence_type, marker in required_notes.items():
        if marker not in indexed[evidence_type]["notes"]:
            fail(f"evidence boundary marker missing: {evidence_type}:{marker}")

    for relative, name in RUST_ANCHORS:
        require_runnable_anchor(root, relative, name)

    identity = (root / "crates/api/src/services/hedge_preview/guards/identity.rs").read_text(encoding="utf-8")
    accepted = re.search(r"if\s+!matches!\(\s*venue,\s*([^)]*)\)", identity, re.DOTALL)
    if accepted is None:
        fail("shared identity venue dispatcher is missing")
    for venue in ("binance", "bitget", "bybit", "okx"):
        if f'"{venue}"' not in accepted.group(1):
            fail(f"shared identity venue dispatcher is missing: {venue}")
    for marker in (
        '"okx" => okx_identity_constraints',
        "okx-v5-public-instruments",
        "okx-v5-private-orders-stream",
        "okx-v5-private-orders-finality",
        "okx-v5-private-orders-fill-fee",
    ):
        if marker not in identity:
            fail(f"OKX shared identity marker missing: {marker}")

    browser = (root / "test/e2e/data_pipeline.spec.ts").read_text(encoding="utf-8")
    if re.search(r"\b(?:test|describe)\.(?:skip|fixme)|FIXME", browser):
        fail("OKX product browser evidence contains skip/fixme")
    if f'test("{BROWSER_TITLE}"' not in browser:
        fail("OKX product browser anchor is missing")

    coverage = {item["file"]: item for item in tsv_rows(root / "docs/PRODUCT_AUDIT_COVERAGE.tsv")}
    for relative in COVERAGE_PATHS:
        item = coverage.get(relative)
        if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
            fail(f"exact coverage missing for {relative}")

    repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    if repo_gate.count("check_pr_cu_completion.sh") != 2:
        fail("repo gate must execute PR-CU exactly once in docs and all scopes")


def run_fixture(root: Path, expect_pass: bool) -> None:
    env = os.environ.copy()
    env.update({
        "PR_CU_SKIP_TESTS": "1",
        "PR_CU_SKIP_UPSTREAM": "1",
        "PR_CU_SKIP_BROWSER_LIST": "1",
    })
    result = subprocess.run(
        ["bash", "scripts/check_pr_cu_completion.sh"],
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
    with tempfile.TemporaryDirectory(prefix="crossline-pr-cu-completion-") as temp:
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
                    if not line.startswith(f"PR-CU\t{evidence_type}\t")
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
                "### 🟡 6.5 下一步执行队列\n\n1. **PR-CU OKX Official Trade Order Semantics** — stale completed row",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        doc.write_text(doc_baseline, encoding="utf-8")

        doc.write_text(
            doc_baseline.replace(
                f"| `{SUCCESSOR_TITLE}` | ✅ 完成 |",
                f"| `{SUCCESSOR_TITLE}` | 🟡 部分完成 |",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        doc.write_text(doc_baseline, encoding="utf-8")

        identity = fixture / "crates/api/src/services/hedge_preview/guards/identity.rs"
        identity_baseline = identity.read_text(encoding="utf-8")
        identity.write_text(
            identity_baseline.replace('"okx"', '"removed-okx"', 1),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        identity.write_text(identity_baseline, encoding="utf-8")

        payload = fixture / "crates/exchange/src/adapters/okx_trade_data_tests.rs"
        payload_baseline = payload.read_text(encoding="utf-8")
        payload.write_text(
            payload_baseline.replace(
                "#[test]\nfn futures_payload_omits_unscoped_optional_currency_and_tag_fields",
                "#[test]\n#[ignore]\nfn futures_payload_omits_unscoped_optional_currency_and_tag_fields",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        payload.write_text(payload_baseline, encoding="utf-8")

        browser = fixture / "test/e2e/data_pipeline.spec.ts"
        browser_baseline = browser.read_text(encoding="utf-8")
        browser.write_text(
            browser_baseline.replace(f'test("{BROWSER_TITLE}"', f'test.skip("{BROWSER_TITLE}"', 1),
            encoding="utf-8",
        )
        run_fixture(fixture, False)

    print("OK PR-CU completion destructive self-test")


try:
    root = Path(sys.argv[1])
    if sys.argv[2] == "--self-test":
        self_test(root)
    else:
        check(root)
        print(
            f"OK PR-CU static contract ({len(EVIDENCE)} evidence types; "
            f"{len(RUST_ANCHORS)} runnable Rust anchors; one non-skipping browser anchor)"
        )
except (OSError, ValueError) as error:
    raise SystemExit(f"PR-CU completion gate failed: {error}") from error
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

if [[ "${PR_CU_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test "$ROOT/test/e2e/data_pipeline.spec.ts" --list >/dev/null
fi

if [[ "${PR_CU_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
  PR_EB_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eb_completion.sh"
  PR_EK_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ek_completion.sh"
  PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bz_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_CU_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api okx_identity_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api \
    order_compile_plan_exposes_client_order_id_policy --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib okx_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test okx_test --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api okx_ --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ek -- --workers=1
fi

printf 'PR-CU OKX official trade order semantics completion passed\n'
