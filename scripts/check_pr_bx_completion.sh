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
import re
import shutil
import sys
import tempfile


PR_TITLE = "PR-BX VenueRuntimeHealth & Settings Diagnostics"
VERIFY_ANCHOR = "`bash scripts/check_pr_bx_completion.sh --self-test`"
REQUIRED_EVIDENCE = {
    "venue-runtime-health-contract-gate": (
        "shared-types/src/venues/tests_runtime_health.rs",
        "snapshot_groups_normalized_venues_into_all_runtime_slots",
    ),
    "venue-runtime-health-api-gate": (
        "crates/api/src/routers/system.rs",
        "api_projects_existing_operation_snapshot_without_external_probes",
    ),
    "scoped-two-leg-preflight-gate": (
        "crates/api/src/services/hedge_preflight/tests/cases_live_b.rs",
        "live_operation_health_guard_scopes_all_evidence_to_ticket_venues",
    ),
    "system-health-usability-gate": (
        "crates/api/src/services/system_health/tests/health.rs",
        "api_health_requires_ok_rows_to_be_configured_and_supported",
    ),
    "settings-runtime-separation-gate": (
        "frontend/src/panels/modules/settings/tabs/diagnostics/tests/search.rs",
        "operation_health_capability_is_distinct_and_searchable",
    ),
    "selected-venue-usability-gate": (
        "frontend/src/panels/modules/settings/tabs/venue_credentials/tests_runtime/health.rs",
        "ok_but_unconfigured_runtime_link_is_not_currently_usable",
    ),
    "four-runtime-slots-browser-gate": (
        "test/e2e/pr_bx_runtime.spec.ts",
        "PR-BX status bar keeps market, trading, private WS, and app WS sources distinct",
    ),
    "completion-governance-gate": (
        "scripts/check_pr_bx_completion.sh",
        "bash scripts/check_pr_bx_completion.sh --self-test",
    ),
}
TEST_ANCHORS = (
    ("shared-types/src/venues/tests_runtime_health.rs", "snapshot_groups_normalized_venues_into_all_runtime_slots"),
    ("shared-types/src/venues/tests_runtime_health.rs", "snapshot_ties_are_evidence_first_and_input_order_independent"),
    ("crates/api/src/routers/system.rs", "api_projects_existing_operation_snapshot_without_external_probes"),
    ("crates/api/src/services/hedge_preflight/tests/cases_live_b.rs", "live_operation_health_guard_scopes_all_evidence_to_ticket_venues"),
    ("crates/api/src/services/hedge_preflight/tests/cases_live_b.rs", "live_operation_health_guard_blocks_missing_orderbook_evidence"),
    ("crates/api/src/services/system_health/tests/health.rs", "api_health_requires_ok_rows_to_be_configured_and_supported"),
    ("frontend/src/panels/modules/settings/tabs/diagnostics/tests/search.rs", "operation_health_capability_is_distinct_and_searchable"),
    ("frontend/src/panels/modules/settings/tabs/venue_credentials/tests_runtime/health.rs", "ok_but_unconfigured_runtime_link_is_not_currently_usable"),
    ("frontend/src/panels/status_bar/slots/tests/cases_ws.rs", "app_ws_uses_channel_state_not_subscriber_count"),
)
BROWSER_ANCHORS = (
    "PR-BX status bar keeps market, trading, private WS, and app WS sources distinct",
    "PR-BX Settings separates configuration, capability, and current usability",
)
CLOSURE_PATHS = tuple(
    """
shared-types/src/lib.rs
shared-types/src/venues.rs
shared-types/src/venues/runtime_health.rs
shared-types/src/venues/runtime_health_snapshot.rs
shared-types/src/venues/tests_runtime_health.rs
crates/api/src/route_specs.rs
crates/api/src/routers/system.rs
crates/api/src/services/hedge_preflight.rs
crates/api/src/services/hedge_preflight/live_health/credentials/part_01.rs
crates/api/src/services/hedge_preflight/tests/cases_live_a/cases/part_01.rs
crates/api/src/services/hedge_preflight/tests/cases_live_b.rs
crates/api/src/services/hedge_preflight/tests/fixtures.rs
crates/api/src/services/system_health/slots.rs
crates/api/src/services/system_health/tests/health.rs
frontend/src/panels/modules/settings/tabs/diagnostics/filter.rs
frontend/src/panels/modules/settings/tabs/diagnostics/operation.rs
frontend/src/panels/modules/settings/tabs/diagnostics/tests/message.rs
frontend/src/panels/modules/settings/tabs/diagnostics/tests/search.rs
frontend/src/panels/modules/settings/tabs/diagnostics/tests/ws_rtt.rs
frontend/src/panels/modules/settings/tabs/diagnostics/ws_rtt.rs
frontend/src/panels/modules/settings/tabs/venue_credentials/derive.rs
frontend/src/panels/modules/settings/tabs/venue_credentials/format/status.rs
frontend/src/panels/modules/settings/tabs/venue_credentials/tests_runtime/health.rs
frontend/src/panels/status_bar/data.rs
frontend/src/panels/status_bar/slots.rs
frontend/src/panels/status_bar/slots/api.rs
frontend/src/panels/status_bar/slots/app_ws.rs
frontend/src/panels/status_bar/slots/market_data.rs
frontend/src/panels/status_bar/slots/operation.rs
frontend/src/panels/status_bar/slots/scan.rs
frontend/src/panels/status_bar/slots/tests/cases_api.rs
frontend/src/panels/status_bar/slots/tests/cases_api2.rs
frontend/src/panels/status_bar/slots/tests/cases_misc.rs
frontend/src/panels/status_bar/slots/tests/cases_ws.rs
frontend/src/panels/status_bar/slots/tests/fixtures.rs
frontend/src/panels/status_bar/slots/ws.rs
frontend/src/panels/status_bar/view.rs
scripts/check_pr_bx_completion.sh
scripts/check_product_audit_evidence_index.sh
scripts/check_release_qa_contract.sh
scripts/verify_repo_gates.sh
test/e2e/mock_api.mjs
test/e2e/pr_bx_runtime.spec.ts
""".split()
)

if len(CLOSURE_PATHS) != 43 or len(set(CLOSURE_PATHS)) != 43:
    raise SystemExit("PR-BX completion gate internal error: expected 43 unique closure paths")


def read_tsv(path: Path, header: tuple[str, ...]) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        if tuple(reader.fieldnames or ()) != header:
            raise ValueError(f"{path.name} header drifted")
        return list(reader)


def bounded_section(text: str, number: str) -> str:
    match = re.search(rf"(?m)^### .*\b{re.escape(number)}\b.*$", text)
    if not match:
        raise ValueError(f"missing bounded {number} section")
    following = re.search(r"(?m)^### ", text[match.end():])
    end = match.end() + following.start() if following else len(text)
    return text[match.start():end]


def require_completed_row(root: Path) -> None:
    text = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    rows = [
        [cell.strip() for cell in line.strip().strip("|").split("|")]
        for line in bounded_section(text, "6.3").splitlines()
        if line.startswith("| `PR-BX ")
    ]
    if len(rows) != 1 or len(rows[0]) != 4:
        raise ValueError("expected one four-cell PR-BX roadmap row")
    title, status, remainder, verification = rows[0]
    if title.strip("`") != PR_TITLE or status != "✅ 完成":
        raise ValueError("PR-BX roadmap row is not completed")
    if "剩余：无。" not in remainder or verification.count(VERIFY_ANCHOR) != 1:
        raise ValueError("PR-BX completed row lacks remaining-none or exact self-test anchor")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-BX\*\*", bounded_section(text, "6.5")):
        raise ValueError("completed PR-BX remains in the 6.5 queue")


def require_evidence(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        ("pr_id", "evidence_type", "artifact", "command", "notes"),
    )
    bx = {row["evidence_type"]: row for row in rows if row["pr_id"] == "PR-BX"}
    if set(bx) != set(REQUIRED_EVIDENCE):
        raise ValueError("PR-BX evidence type set drifted")
    for evidence_type, (artifact, anchor) in REQUIRED_EVIDENCE.items():
        row = bx[evidence_type]
        if row["artifact"] != artifact or anchor not in row["command"]:
            raise ValueError(f"PR-BX {evidence_type} artifact/command drifted")
    command = bx["completion-governance-gate"]["command"]
    for anchor in ("bash -n scripts/check_pr_bx_completion.sh", "--self-test", "bash scripts/check_pr_bx_completion.sh"):
        if anchor not in command:
            raise ValueError(f"completion governance command lacks {anchor}")


def require_anchors(root: Path) -> None:
    skip_pattern = re.compile(r"#\s*\[\s*(?:ignore|should_panic)|(?:test|describe)\.skip|\.skip\(|FIXME", re.I)
    for artifact, anchor in TEST_ANCHORS:
        text = (root / artifact).read_text(encoding="utf-8")
        if anchor not in text:
            raise ValueError(f"missing runnable anchor {anchor} in {artifact}")
        if skip_pattern.search(text):
            raise ValueError(f"skip marker found in PR-BX anchor artifact {artifact}")
    browser = (root / "test/e2e/pr_bx_runtime.spec.ts").read_text(encoding="utf-8")
    for anchor in BROWSER_ANCHORS:
        if browser.count(anchor) != 1:
            raise ValueError(f"browser anchor drifted: {anchor}")
    if skip_pattern.search(browser):
        raise ValueError("PR-BX browser evidence contains skip marker")


def require_coverage(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        ("file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"),
    )
    by_file = {row["file"]: row for row in rows}
    for path in CLOSURE_PATHS:
        row = by_file.get(path)
        if row is None or row["coverage_status"] != "exact":
            raise ValueError(f"PR-BX closure path is not exact: {path}")


def validate(root: Path) -> None:
    require_completed_row(root)
    require_evidence(root)
    require_anchors(root)
    require_coverage(root)


def copy_fixture(root: Path, fixture: Path) -> None:
    paths = {
        "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
        "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        *CLOSURE_PATHS,
        *(artifact for artifact, _ in TEST_ANCHORS),
        *(artifact for artifact, _ in REQUIRED_EVIDENCE.values()),
    }
    for relative in paths:
        source = root / relative
        target = fixture / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)


def expect_failure(fixture: Path, mutation) -> None:
    mutation()
    try:
        validate(fixture)
    except (ValueError, FileNotFoundError):
        return
    raise ValueError("PR-BX destructive self-test unexpectedly passed")


def self_test(root: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="crossline-pr-bx-") as temp:
        base = Path(temp) / "base"
        copy_fixture(root, base)
        validate(base)

        evidence = Path(temp) / "evidence"
        shutil.copytree(base, evidence)
        path = evidence / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        expect_failure(evidence, lambda: path.write_text(path.read_text().replace(
            "snapshot_groups_normalized_venues_into_all_runtime_slots", "drifted-anchor", 1
        ), encoding="utf-8"))

        coverage = Path(temp) / "coverage"
        shutil.copytree(base, coverage)
        path = coverage / "docs/PRODUCT_AUDIT_COVERAGE.tsv"
        expect_failure(coverage, lambda: path.write_text(path.read_text().replace(
            "shared-types/src/venues/runtime_health.rs\texact\t",
            "shared-types/src/venues/runtime_health.rs\tmissing\t",
            1,
        ), encoding="utf-8"))

        queue = Path(temp) / "queue"
        shutil.copytree(base, queue)
        path = queue / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        expect_failure(queue, lambda: path.write_text(path.read_text().replace(
            "1. **PR-ER**", "1. **PR-BX**", 1
        ), encoding="utf-8"))

        skipped = Path(temp) / "skipped"
        shutil.copytree(base, skipped)
        path = skipped / "test/e2e/pr_bx_runtime.spec.ts"
        expect_failure(skipped, lambda: path.write_text(
            "test.skip('disabled', () => {});\n" + path.read_text(), encoding="utf-8"
        ))


root = Path(sys.argv[1])
try:
    if sys.argv[2] == "--self-test":
        self_test(root)
        print("OK PR-BX completion gate self-test")
    else:
        validate(root)
        print(
            f"OK PR-BX completion gate ({len(REQUIRED_EVIDENCE)} evidence types; "
            f"{len(TEST_ANCHORS)} Rust/frontend anchors; {len(BROWSER_ANCHORS)} browser anchors; "
            f"{len(CLOSURE_PATHS)} exact paths)"
        )
except (ValueError, FileNotFoundError) as error:
    raise SystemExit(f"PR-BX completion gate failed: {error}")
PY
