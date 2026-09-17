#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"

if [ "$MODE" != "check" ] && [ "$MODE" != "--self-test" ]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

python3 - "$ROOT" "$MODE" <<'PY'
from __future__ import annotations

from pathlib import Path
import csv
import io
import re
import sys
import tempfile


PR_TITLE = "PR-FL Watchlist Alerts State, Prewarm Side-Effect & Notification Contract"
BROWSER_PATH = "test/e2e/watchlist_alerts_runtime.spec.ts"
BROWSER_TITLE = "settings exposes bounded watchlist prewarm and truthful toast queue runtime"
REQUIRED_EVIDENCE = {
    "watchlist-validation": (
        "shared-types/src/alerts.rs",
        "blank_symbol_is_rejected",
    ),
    "alert-validation": (
        "shared-types/src/alerts.rs",
        "webhook_channel_is_fail_closed_even_with_https_url",
    ),
    "route-runtime-contract": (
        "crates/api/src/app.rs",
        "gated_routes_register_when_enabled",
    ),
    "volatile-restart-contract-gate": (
        "crates/api/src/routers/watchlist/tests.rs",
        "volatile_watchlist_alert_state_restarts_empty_with_explicit_runtime_meta",
    ),
    "mutation-idempotency-gate": (
        "crates/realtime/src/alerts/tests/cases.rs",
        "watchlist_and_alert_create_replay_without_duplicate_business_side_effects",
    ),
    "runtime-plan-health-gate": (
        "crates/api/src/lifecycle/market_data_tests/watchlist_runtime.rs",
        "watchlist_runtime_plan_is_bounded_visible_and_private_ws_free",
    ),
    "alert-engine-correctness-gate": (
        "crates/realtime/src/alerts/tests/cases.rs",
        "alert_matching_requires_p0_fresh_executable_and_highest_final_score",
    ),
    "notification-channel-truth-gate": (
        "crates/api/src/lifecycle/snapshot/alerts/tests.rs",
        "toast_with_subscriber_queues_typed_user_alert_event",
    ),
    "ws-contract-gate": (
        "crates/api/src/services/ws_replay/tests/watchlist_alerts.rs",
        "watchlist_and_alert_channels_replay_current_volatile_state",
    ),
    "settings-browser-gate": (BROWSER_PATH, BROWSER_TITLE),
}

TEST_ANCHORS = (
    ("shared-types/src/alerts.rs", "blank_symbol_is_rejected"),
    ("shared-types/src/alerts.rs", "webhook_channel_is_fail_closed_even_with_https_url"),
    ("shared-types/src/alerts.rs", "watchlist_alert_stream_timestamps_use_camel_case"),
    ("shared-types/src/alerts.rs", "alert_rule_rejects_cooldown_above_product_limit"),
    ("crates/api/src/app.rs", "gated_routes_register_when_enabled"),
    (
        "crates/realtime/src/alerts/tests/cases.rs",
        "watchlist_and_alert_create_replay_without_duplicate_business_side_effects",
    ),
    (
        "crates/realtime/src/alerts/tests/cases.rs",
        "alert_matching_requires_p0_fresh_executable_and_highest_final_score",
    ),
    (
        "crates/realtime/src/alerts/tests/cases.rs",
        "alert_matching_rejects_stale_market_evidence_and_missing_final_ranking",
    ),
    (
        "crates/realtime/src/alerts/tests/cases.rs",
        "toast_queue_requires_real_subscriber_before_cooldown_and_count",
    ),
    (
        "crates/realtime/src/alerts/tests/cases.rs",
        "unvalidated_extreme_cooldown_cannot_wrap_into_the_past",
    ),
    (
        "crates/api/src/routers/watchlist/tests.rs",
        "watchlist_create_and_delete_replay_without_duplicate_ws_side_effects",
    ),
    (
        "crates/api/src/routers/watchlist/tests.rs",
        "watchlist_delete_cascades_rule_and_cooldown_atomically",
    ),
    (
        "crates/api/src/routers/watchlist/tests.rs",
        "volatile_watchlist_alert_state_restarts_empty_with_explicit_runtime_meta",
    ),
    (
        "crates/api/src/routers/alerts/tests.rs",
        "alert_rule_mutations_replay_without_duplicate_side_effects",
    ),
    (
        "crates/api/src/lifecycle/market_data_tests/watchlist_runtime.rs",
        "watchlist_runtime_plan_is_bounded_visible_and_private_ws_free",
    ),
    (
        "crates/api/src/lifecycle/market_data_tests/watchlist_runtime.rs",
        "watchlist_runtime_surfaces_correlated_prewarm_problem",
    ),
    (
        "crates/api/src/lifecycle/market_data_tests/watchlist_runtime.rs",
        "ticker_plan_prevents_false_full_leg_cap",
    ),
    (
        "crates/api/src/lifecycle/market_data_tests/watchlist_runtime.rs",
        "stale_prewarm_snapshot_cannot_update_recreated_watchlist_id",
    ),
    (
        "crates/api/src/lifecycle/snapshot/alerts/tests.rs",
        "toast_without_subscriber_is_not_reported_as_queued",
    ),
    (
        "crates/api/src/lifecycle/snapshot/alerts/tests.rs",
        "toast_with_subscriber_queues_typed_user_alert_event",
    ),
    (
        "crates/api/src/lifecycle/snapshot/alerts/tests.rs",
        "deleted_rule_generation_is_rejected_before_notification_queue",
    ),
    (
        "crates/api/src/lifecycle/snapshot/alerts/tests.rs",
        "cooldown_registry_only_tracks_future_enabled_delivery_deadlines",
    ),
    (
        "crates/api/src/services/ws_replay/tests/watchlist_alerts.rs",
        "watchlist_and_alert_channels_replay_current_volatile_state",
    ),
    ("crates/api/src/routers/websocket.rs", "watchlist_alert_channels_follow_feature_gate"),
    ("frontend/src/api/ws.rs", "watchlist_alert_stream_contract_is_distinct_from_risk_alerts"),
    (
        "frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts/tests.rs",
        "runtime_labels_distinguish_queued_blocked_and_risk_alert_channels",
    ),
    (
        "frontend/src/panels/modules/settings/data/watchlist_alerts.rs",
        "delayed_rest_response_cannot_replace_newer_ws_snapshot",
    ),
    ("frontend/src/state/watchlist_alerts.rs", "toast_message_uses_backend_final_score_and_execution_legs"),
    (
        "frontend/src/state/watchlist_alerts.rs",
        "alert_event_uses_captured_toast_signal_without_context_lookup",
    ),
)

CLOSURE_PATHS = (
    "scripts/check_pr_fl_completion.sh",
    "scripts/check_product_audit_evidence_index.sh",
    "scripts/verify_repo_gates.sh",
    "scripts/check_route_inventory.sh",
    "crates/api/src/app.rs",
    "shared-types/src/alerts.rs",
    "shared-types/src/lib.rs",
    "crates/realtime/src/alerts.rs",
    "crates/realtime/src/lib.rs",
    "crates/realtime/src/alerts/evaluation.rs",
    "crates/realtime/src/alerts/mutations.rs",
    "crates/realtime/src/alerts/tests.rs",
    "crates/realtime/src/alerts/tests/cases.rs",
    "crates/realtime/src/alerts/tests/fixtures.rs",
    "crates/api/src/lifecycle/market_data.rs",
    "crates/api/src/lifecycle/market_data/watchlist_runtime.rs",
    "crates/api/src/lifecycle/market_data_tests.rs",
    "crates/api/src/lifecycle/market_data_tests/watchlist_runtime.rs",
    "crates/api/src/lifecycle/snapshot.rs",
    "crates/api/src/lifecycle/snapshot/alerts.rs",
    "crates/api/src/lifecycle/snapshot/alerts/tests.rs",
    "crates/api/src/routers/alerts.rs",
    "crates/api/src/routers/alerts/tests.rs",
    "crates/api/src/routers/watchlist.rs",
    "crates/api/src/routers/watchlist/tests.rs",
    "crates/api/src/routers/websocket.rs",
    "crates/api/src/services/runtime_state.rs",
    "crates/api/src/services/ws_replay.rs",
    "crates/api/src/services/ws_replay/tests.rs",
    "crates/api/src/services/ws_replay/tests/watchlist_alerts.rs",
    "frontend/src/api/rest.rs",
    "frontend/src/api/rest/watchlist_alerts.rs",
    "frontend/src/api/ws.rs",
    "frontend/src/app.rs",
    "frontend/src/panels/modules/settings/data.rs",
    "frontend/src/panels/modules/settings/data/watchlist_alerts.rs",
    "frontend/src/panels/modules/settings/tabs/diagnostics.rs",
    "frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts.rs",
    "frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts/tests.rs",
    "frontend/src/state/mod.rs",
    "frontend/src/state/watchlist_alerts.rs",
    "test/e2e/mock_api.mjs",
    BROWSER_PATH,
)

if len(CLOSURE_PATHS) != 43 or len(set(CLOSURE_PATHS)) != 43:
    raise SystemExit("PR-FL completion gate internal error: expected 43 unique closure paths")


def read_tsv(path: Path, expected_header: tuple[str, ...]) -> list[dict[str, str]]:
    if not path.is_file():
        raise ValueError(f"missing {path.name}")
    with path.open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        if tuple(reader.fieldnames or ()) != expected_header:
            raise ValueError(f"{path.name} header drifted")
        return list(reader)


def completion_state(root: Path) -> bool:
    text = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    start = text.find("### 🟡 6.3")
    end = text.find("### 🟡 6.4", start)
    if start < 0 or end < 0:
        raise ValueError("missing bounded 6.3 roadmap section")
    rows = [
        [cell.strip() for cell in line.strip().strip("|").split("|")]
        for line in text[start:end].splitlines()
        if line.startswith("| `PR-FL ")
    ]
    if len(rows) != 1 or len(rows[0]) != 4:
        raise ValueError(f"expected one four-cell PR-FL roadmap row, found {len(rows)}")
    cells = rows[0]
    if cells[0].strip("`") != PR_TITLE:
        raise ValueError("PR-FL roadmap title drifted")
    if cells[1] == "🟡 部分完成":
        return False
    if cells[1] != "✅ 完成":
        raise ValueError(f"unsupported PR-FL status: {cells[1]!r}")
    if "剩余：无。" not in cells[2] or "scripts/check_pr_fl_completion.sh" not in cells[2]:
        raise ValueError("completed PR-FL row must declare no remainder and its completion gate")
    return True


def require_evidence(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        ("pr_id", "evidence_type", "artifact", "command", "notes"),
    )
    selected = [row for row in rows if row["pr_id"].strip() == "PR-FL"]
    by_type: dict[str, dict[str, str]] = {}
    for row in selected:
        evidence_type = row["evidence_type"].strip()
        if evidence_type in by_type:
            raise ValueError(f"duplicate PR-FL evidence type: {evidence_type}")
        by_type[evidence_type] = row
    if set(by_type) != set(REQUIRED_EVIDENCE):
        raise ValueError(
            "PR-FL evidence type drift: "
            f"missing={sorted(set(REQUIRED_EVIDENCE) - set(by_type))}, "
            f"extra={sorted(set(by_type) - set(REQUIRED_EVIDENCE))}"
        )
    for evidence_type, (artifact, anchor) in REQUIRED_EVIDENCE.items():
        row = by_type[evidence_type]
        if row["artifact"].strip() != artifact or anchor not in row["command"]:
            raise ValueError(f"PR-FL {evidence_type} artifact or command anchor drifted")


def strip_comments(text: str) -> str:
    output: list[str] = []
    index = 0
    depth = 0
    quote: str | None = None
    escaped = False
    while index < len(text):
        char = text[index]
        pair = text[index:index + 2]
        if depth:
            if pair == "/*":
                depth += 1
                output.extend("  ")
                index += 2
            elif pair == "*/":
                depth -= 1
                output.extend("  ")
                index += 2
            else:
                output.append("\n" if char == "\n" else " ")
                index += 1
            continue
        if quote:
            output.append(char)
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = None
            index += 1
            continue
        if pair == "//":
            newline = text.find("\n", index + 2)
            if newline < 0:
                output.extend(" " * (len(text) - index))
                break
            output.extend(" " * (newline - index))
            output.append("\n")
            index = newline + 1
        elif pair == "/*":
            depth = 1
            output.extend("  ")
            index += 2
        else:
            output.append(char)
            if char in {'"', "'"}:
                quote = char
            index += 1
    return "".join(output)


def require_test_anchors(root: Path) -> None:
    for relative_path, function_name in TEST_ANCHORS:
        path = root / relative_path
        if not path.is_file():
            raise ValueError(f"missing PR-FL anchor file: {relative_path}")
        text = strip_comments(path.read_text(encoding="utf-8"))
        pattern = re.compile(
            rf"(?m)((?:^\s*#\[[^\n]+\]\s*\n)+)"
            rf"\s*(?:async\s+)?fn\s+{re.escape(function_name)}\s*\("
        )
        match = pattern.search(text)
        if not match or not re.search(r"#\[(?:tokio::)?test", match.group(1)):
            raise ValueError(f"missing runnable PR-FL test anchor {relative_path}::{function_name}")
        if re.search(r"ignore|should_panic", match.group(1)):
            raise ValueError(f"PR-FL test anchor is skipped or should_panic: {relative_path}::{function_name}")

    browser = strip_comments((root / BROWSER_PATH).read_text(encoding="utf-8"))
    runnable = re.search(rf"(?m)^\s*test\(\s*[\"']{re.escape(BROWSER_TITLE)}[\"']", browser)
    skipped = re.search(rf"(?m)^\s*test\.(?:skip|fixme)\(\s*[\"']{re.escape(BROWSER_TITLE)}[\"']", browser)
    if not runnable or skipped:
        raise ValueError("missing runnable PR-FL browser anchor")


def require_exact_coverage(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        ("file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"),
    )
    by_path = {row["file"].strip(): row for row in rows}
    if len(by_path) != len(rows):
        raise ValueError("duplicate coverage path")
    for relative_path in CLOSURE_PATHS:
        if not (root / relative_path).is_file():
            raise ValueError(f"missing designated PR-FL closure path: {relative_path}")
        row = by_path.get(relative_path)
        if row is None or row["coverage_status"].strip() != "exact":
            raise ValueError(f"PR-FL closure path is not exact: {relative_path}")
        if not row["evidence"].strip().startswith("line:"):
            raise ValueError(f"PR-FL closure path lacks line evidence: {relative_path}")


def validate(root: Path) -> bool:
    if not completion_state(root):
        return False
    require_evidence(root)
    require_test_anchors(root)
    require_exact_coverage(root)
    return True


def fixture_tsv(rows: list[list[str]]) -> str:
    output = io.StringIO()
    csv.writer(output, delimiter="\t", lineterminator="\n").writerows(rows)
    return output.getvalue()


def write_fixture(root: Path) -> None:
    docs = root / "docs"
    docs.mkdir(parents=True, exist_ok=True)
    (docs / "PRODUCT_FULL_AUDIT_REFINEMENT.md").write_text(
        "### 🟡 6.3 建议整改顺序\n\n"
        f"| `{PR_TITLE}` | ✅ 完成 | 剩余：无。验证：`scripts/check_pr_fl_completion.sh`。 | static |\n\n"
        "### 🟡 6.4 其它\n",
        encoding="utf-8",
    )
    evidence = [["pr_id", "evidence_type", "artifact", "command", "notes"]]
    for evidence_type, (artifact, anchor) in REQUIRED_EVIDENCE.items():
        evidence.append(["PR-FL", evidence_type, artifact, f"test {anchor}", "self-test"])
    (docs / "PRODUCT_AUDIT_EVIDENCE.tsv").write_text(fixture_tsv(evidence), encoding="utf-8")

    coverage = [["file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"]]
    for index, relative_path in enumerate(CLOSURE_PATHS, 1):
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("", encoding="utf-8")
        coverage.append([relative_path, "exact", "self_test", "P0", "PR-FL", f"line:self:{index}", "exact"])
    (docs / "PRODUCT_AUDIT_COVERAGE.tsv").write_text(fixture_tsv(coverage), encoding="utf-8")

    for relative_path, function_name in TEST_ANCHORS:
        path = root / relative_path
        with path.open("a", encoding="utf-8") as handle:
            handle.write(f"\n#[test]\nfn {function_name}() {{}}\n")
    browser = root / BROWSER_PATH
    browser.write_text(f'test("{BROWSER_TITLE}", async () => {{}});\n', encoding="utf-8")


def expect_failure(root: Path, expected: str) -> None:
    try:
        if not validate(root):
            raise AssertionError("completed fixture was treated as partial")
    except ValueError as error:
        if expected not in str(error):
            raise AssertionError(f"expected {expected!r}, got {error!r}") from error
        return
    raise AssertionError(f"expected validation failure containing {expected!r}")


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="crossline-pr-fl-gate-") as temp:
        root = Path(temp)
        write_fixture(root)
        validate(root)

        doc = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc.write_text(doc.read_text(encoding="utf-8").replace("✅ 完成", "🟡 部分完成"), encoding="utf-8")
        if validate(root):
            raise AssertionError("partial PR-FL row must skip completion enforcement")
        write_fixture(root)

        evidence = root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        lines = evidence.read_text(encoding="utf-8").splitlines()
        evidence.write_text("\n".join(lines[:-1]) + "\n", encoding="utf-8")
        expect_failure(root, "evidence type drift")
        write_fixture(root)

        path = root / TEST_ANCHORS[0][0]
        text = path.read_text(encoding="utf-8")
        path.write_text(text.replace("#[test]\nfn blank_symbol_is_rejected", "/* #[test]\nfn blank_symbol_is_rejected", 1) + "*/\n", encoding="utf-8")
        expect_failure(root, "missing runnable PR-FL test anchor")
        write_fixture(root)

        path = root / TEST_ANCHORS[0][0]
        text = path.read_text(encoding="utf-8")
        path.write_text(text.replace("#[test]\nfn blank_symbol_is_rejected", "#[test]\n#[ignore]\nfn blank_symbol_is_rejected", 1), encoding="utf-8")
        expect_failure(root, "skipped or should_panic")
        write_fixture(root)

        browser = root / BROWSER_PATH
        browser.write_text(f'test.skip("{BROWSER_TITLE}", async () => {{}});\n', encoding="utf-8")
        expect_failure(root, "browser anchor")
        write_fixture(root)

        coverage = root / "docs/PRODUCT_AUDIT_COVERAGE.tsv"
        coverage.write_text(coverage.read_text(encoding="utf-8").replace("\texact\t", "\tbasename\t", 1), encoding="utf-8")
        expect_failure(root, "closure path is not exact")


root = Path(sys.argv[1])
mode = sys.argv[2]
try:
    if mode == "--self-test":
        self_test()
        print("OK PR-FL completion gate self-test")
    elif validate(root):
        print(
            "OK PR-FL completion gate "
            f"({len(REQUIRED_EVIDENCE)} evidence types; {len(TEST_ANCHORS)} test anchors; "
            f"1 browser anchor; {len(CLOSURE_PATHS)} exact paths)"
        )
    else:
        print("SKIP PR-FL completion gate (roadmap row remains partial)")
except (AssertionError, OSError, ValueError) as error:
    raise SystemExit(f"PR-FL completion gate failed: {error}") from error
PY
