#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODE="${1:-check}"

if [ "$MODE" != "check" ] && [ "$MODE" != "--self-test" ]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

python3 - "$ROOT" "$MODE" <<'PY'
from __future__ import annotations

import csv
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


PR_TITLE = "PR-Q Config & Credential Safety"
EVIDENCE = {
    "okx-profile-selection": (
        "crates/api/src/services/trading_credentials.rs",
        "cargo test -p api --bin crypto-arb-api okx_profile --no-fail-fast",
    ),
    "credential-validation": (
        "crates/api/src/services/okx_credential_profile.rs",
        "cargo test -p api --bin crypto-arb-api venue_credentials --no-fail-fast",
    ),
    "credential-health-dto": (
        "shared-types/src/venues/credentials.rs",
        "cargo test -p shared-types credential_field_source_defaults_to_missing_for_legacy_payloads --lib --no-fail-fast",
    ),
    "credential-source-storage": (
        "crates/api/src/services/venue_credentials/storage.rs",
        "cargo test -p api --bin crypto-arb-api migrate_persists_present_fields_and_reports_missing_keys --no-fail-fast",
    ),
    "secret-clear-migrate": (
        "crates/api/src/services/venue_credentials/maintenance.rs",
        "cargo test -p api --bin crypto-arb-api dotenv_path_removal_is_atomic_and_leaves_other_credentials_intact --no-fail-fast",
    ),
    "hyperliquid-agent-relation": (
        "crates/api/src/services/venue_credentials/validation/hyperliquid_probes.rs",
        "cargo test -p api --bin crypto-arb-api hyperliquid_relation_probe --no-fail-fast",
    ),
    "credential-action-run-audit": (
        "crates/api/src/routers/exchanges.rs",
        "cargo test -p api --bin crypto-arb-api clear_credentials_replays_the_same_action_run --no-fail-fast",
    ),
    "durable-audit-summary": (
        "crates/api/src/services/action_runs/audit_summary.rs",
        "cargo test -p api --bin crypto-arb-api credential_maintenance_summary_keeps_safe_operation_evidence --no-fail-fast",
    ),
    "settings-maintenance-ui": (
        "frontend/src/panels/modules/settings/tabs/venue_credentials/maintenance.rs",
        "cargo test --manifest-path frontend/Cargo.toml --lib credential_maintenance --no-fail-fast",
    ),
    "completion-governance": (
        "scripts/check_pr_q_completion.sh",
        "bash scripts/check_pr_q_completion.sh --self-test",
    ),
}

TEST_ANCHORS = (
    (
        "shared-types/src/venues/credentials/tests.rs",
        "credential_field_source_defaults_to_missing_for_legacy_payloads",
    ),
    (
        "crates/api/src/services/venue_credentials/storage/tests.rs",
        "migrate_persists_present_fields_and_reports_missing_keys",
    ),
    (
        "crates/api/src/services/venue_credentials/tests.rs",
        "dotenv_path_removal_is_atomic_and_leaves_other_credentials_intact",
    ),
    (
        "crates/api/src/routers/exchanges.rs",
        "clear_credentials_replays_the_same_action_run",
    ),
    (
        "crates/api/src/services/action_runs/audit_summary.rs",
        "credential_maintenance_summary_keeps_safe_operation_evidence",
    ),
    (
        "crates/api/src/services/venue_credentials/validation/tests.rs",
        "hyperliquid_relation_probe_accepts_approved_agent_for_led_vault",
    ),
    (
        "frontend/src/panels/modules/settings/data/tests/credential_maintenance.rs",
        "credential_maintenance_success_message_keeps_audit_and_health_context",
    ),
    (
        "frontend/src/panels/modules/settings/tabs/venue_credentials/maintenance.rs",
        "clear_requires_the_selected_venue_phrase",
    ),
)

SOURCE_MARKERS = {
    "shared-types/src/venues/credentials.rs": (
        "VenueCredentialFieldSource",
        "VenueCredentialClearRequest",
        "VenueCredentialMigrateRequest",
        "VenueCredentialMaintenanceResponse",
    ),
    "crates/api/src/services/venue_credentials/storage.rs": (
        "CLEARED_SECRETS",
        "pub(super) async fn clear_fields",
        "pub(super) async fn migrate_fields",
        "VenueCredentialFieldSource::Keychain",
    ),
    "crates/api/src/services/venue_credentials/maintenance.rs": (
        "pub(crate) async fn clear",
        "pub(crate) async fn migrate",
        "环境变量来源已在当前进程屏蔽",
    ),
    "crates/api/src/services/venue_credentials/validation/hyperliquid_probes.rs": (
        "hyperliquid_account_relation_probe(common::time::now_ms(), outcome)",
    ),
    "crates/api/src/routers/exchanges.rs": (
        "/api/exchanges/credentials/clear",
        "/api/exchanges/credentials/migrate",
        "ActionRunKind::VenueCredentialsClear",
        "ActionRunKind::VenueCredentialsMigrate",
    ),
    "crates/api/src/services/action_runs/audit_log.rs": (
        '"venue_credentials.clear"',
        '"venue_credentials.migrate"',
    ),
    "frontend/src/panels/modules/settings/data/credential_maintenance.rs": (
        "use_venue_credential_maintenance_action",
        "clear_venue_credentials_with_context",
        "migrate_venue_credentials_with_context",
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials/maintenance.rs": (
        "clear_confirmation_matches",
        "VenueCredentialMaintenance::Clear",
        "VenueCredentialMaintenance::Migrate",
    ),
    "crates/api/src/route_specs.rs": (
        "ActionRunKind::VenueCredentialsClear",
        "ActionRunKind::VenueCredentialsMigrate",
    ),
}

CLOSURE_PATHS = tuple(
    dict.fromkeys(
        [
            "scripts/check_pr_q_completion.sh",
            "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
            "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
            "docs/PRODUCT_AUDIT_COVERAGE.tsv",
            *EVIDENCE.values(),
            *TEST_ANCHORS,
            *SOURCE_MARKERS.keys(),
        ]
    )
)
TSV_HEADER = ("pr_id", "evidence_type", "artifact", "command", "notes")
COVERAGE_HEADER = (
    "file",
    "coverage_status",
    "owner_surface",
    "risk",
    "suggested_pr",
    "evidence",
    "notes",
)


def read_tsv(path: Path, expected_header: tuple[str, ...]) -> list[dict[str, str]]:
    if not path.is_file():
        raise ValueError(f"missing {path}")
    with path.open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        if tuple(reader.fieldnames or ()) != expected_header:
            raise ValueError(
                f"{path.name} header drifted: expected {expected_header}, "
                f"got {tuple(reader.fieldnames or ())}"
            )
        return list(reader)


def require_roadmap(root: Path) -> None:
    path = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
    text = path.read_text(encoding="utf-8")
    start = text.find("### 🟡 6.3")
    end = text.find("### 🟡 6.4", start)
    if start < 0 or end < 0:
        raise ValueError("missing bounded 6.3 roadmap section")
    rows = [
        [cell.strip() for cell in line.strip().strip("|").split("|")]
        for line in text[start:end].splitlines()
        if line.startswith("| `PR-Q ")
    ]
    if len(rows) != 1 or len(rows[0]) != 4:
        raise ValueError("expected one four-cell PR-Q roadmap row")
    cells = rows[0]
    if cells[0].strip("`") != PR_TITLE:
        raise ValueError(f"PR-Q roadmap title drifted: {cells[0]!r}")
    if cells[1] != "✅ 完成":
        raise ValueError(f"PR-Q roadmap status must be complete, got {cells[1]!r}")
    if "剩余：无（本地）" not in cells[2]:
        raise ValueError("PR-Q completion must distinguish no local remaining work")
    if "bash scripts/check_pr_q_completion.sh --self-test" not in cells[3]:
        raise ValueError("PR-Q completion must record its destructive self-test")
    queue_start = text.find("### 🟡 6.5")
    if queue_start < 0:
        raise ValueError("missing 6.5 execution queue")
    queue = text[queue_start:]
    if re.search(r"(?m)^\d+\.\s+\*\*PR-Q\b", queue):
        raise ValueError("completed PR-Q must not remain in the 6.5 queue")


def require_evidence(root: Path) -> None:
    rows = read_tsv(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv", TSV_HEADER)
    by_type: dict[str, dict[str, str]] = {}
    for row in rows:
        if row["pr_id"].strip() != "PR-Q":
            continue
        evidence_type = row["evidence_type"].strip()
        if evidence_type in by_type:
            raise ValueError(f"duplicate PR-Q evidence type: {evidence_type}")
        by_type[evidence_type] = row
    if set(by_type) != set(EVIDENCE):
        raise ValueError(
            "PR-Q evidence type drift: "
            f"missing={sorted(set(EVIDENCE) - set(by_type))}, "
            f"extra={sorted(set(by_type) - set(EVIDENCE))}"
        )
    for evidence_type, (artifact, command) in EVIDENCE.items():
        row = by_type[evidence_type]
        if row["artifact"].strip() != artifact:
            raise ValueError(f"PR-Q {evidence_type} artifact drifted")
        if command not in row["command"]:
            raise ValueError(f"PR-Q {evidence_type} command drifted")
        if not (root / artifact).is_file():
            raise ValueError(f"PR-Q {evidence_type} artifact missing: {artifact}")


def require_runnable_tests(root: Path) -> None:
    for relative_path, function_name in TEST_ANCHORS:
        path = root / relative_path
        text = path.read_text(encoding="utf-8")
        pattern = re.compile(
            r"(?m)^\s*#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*\n"
            r"(?P<attrs>(?:\s*#\[[^\n]+\]\s*\n)*)"
            rf"\s*(?:pub\s+)?(?:async\s+)?fn\s+{re.escape(function_name)}\s*\("
        )
        match = pattern.search(text)
        if match is None:
            raise ValueError(f"missing runnable PR-Q test: {relative_path}::{function_name}")
        if "ignore" in match.group("attrs").lower():
            raise ValueError(f"ignored PR-Q test: {relative_path}::{function_name}")


def require_source_markers(root: Path) -> None:
    for relative_path, markers in SOURCE_MARKERS.items():
        text = (root / relative_path).read_text(encoding="utf-8")
        for marker in markers:
            if marker not in text:
                raise ValueError(f"missing PR-Q marker {relative_path}: {marker}")


def require_coverage(root: Path) -> None:
    rows = read_tsv(root / "docs/PRODUCT_AUDIT_COVERAGE.tsv", COVERAGE_HEADER)
    matches = [
        row
        for row in rows
        if row["file"].strip() == "scripts/check_pr_q_completion.sh"
    ]
    if len(matches) != 1 or matches[0]["coverage_status"].strip() != "exact":
        raise ValueError("PR-Q completion gate requires exact coverage ownership")


def check(root: Path) -> None:
    require_roadmap(root)
    require_evidence(root)
    require_runnable_tests(root)
    require_source_markers(root)
    require_coverage(root)


def copy_fixture(root: Path, fixture: Path) -> None:
    paths: set[str] = {
        "scripts/check_pr_q_completion.sh",
        "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
        "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        *SOURCE_MARKERS.keys(),
    }
    paths.update(artifact for artifact, _ in EVIDENCE.values())
    paths.update(path for path, _ in TEST_ANCHORS)
    for relative_path in paths:
        source = root / relative_path
        if not source.is_file():
            raise ValueError(f"self-test source missing: {relative_path}")
        target = fixture / relative_path
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)


def run_fixture(fixture: Path, expect_pass: bool) -> None:
    result = subprocess.run(
        ["bash", "scripts/check_pr_q_completion.sh"],
        cwd=fixture,
        text=True,
        capture_output=True,
        check=False,
        env=os.environ.copy(),
    )
    if expect_pass != (result.returncode == 0):
        detail = (result.stderr or result.stdout).strip()
        raise ValueError(f"PR-Q destructive self-test expectation failed: {detail}")


def self_test(root: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="crossline-pr-q-completion-") as temp:
        fixture = Path(temp) / "repo"
        copy_fixture(root, fixture)
        run_fixture(fixture, True)

        evidence = fixture / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        baseline = evidence.read_text(encoding="utf-8")
        evidence.write_text(
            "\n".join(
                line
                for line in baseline.splitlines()
                if not line.startswith("PR-Q\tsecret-clear-migrate\t")
            )
            + "\n",
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        evidence.write_text(baseline, encoding="utf-8")

        roadmap = fixture / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        baseline = roadmap.read_text(encoding="utf-8")
        roadmap.write_text(
            baseline.replace(
                "| `PR-Q Config & Credential Safety` | ✅ 完成 |",
                "| `PR-Q Config & Credential Safety` | 🟡 部分完成 |",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        roadmap.write_text(baseline, encoding="utf-8")

        anchor = fixture / "crates/api/src/routers/exchanges.rs"
        baseline = anchor.read_text(encoding="utf-8")
        anchor.write_text(
            baseline.replace(
                "#[tokio::test]\n    async fn clear_credentials_replays_the_same_action_run",
                "#[tokio::test]\n    #[ignore]\n    async fn clear_credentials_replays_the_same_action_run",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        anchor.write_text(baseline, encoding="utf-8")

        coverage = fixture / "docs/PRODUCT_AUDIT_COVERAGE.tsv"
        baseline = coverage.read_text(encoding="utf-8")
        coverage.write_text(
            baseline.replace(
                "scripts/check_pr_q_completion.sh\texact\t",
                "scripts/check_pr_q_completion.sh\tmissing\t",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)

    print("OK PR-Q completion destructive self-test")


root = Path(sys.argv[1])
mode = sys.argv[2]
try:
    if mode == "--self-test":
        self_test(root)
    else:
        check(root)
        print(
            "OK PR-Q completion gate "
            f"({len(EVIDENCE)} evidence types, {len(TEST_ANCHORS)} runnable anchors, "
            f"{len(SOURCE_MARKERS)} source marker files)"
        )
except (OSError, ValueError) as exc:
    raise SystemExit(f"PR-Q completion gate failed: {exc}") from exc
PY
