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
import shutil
import sys
import tempfile


PR_ID = "PR-FI"
PR_TITLE = "PR-FI API Security, Auth/CORS, WS Auth & High-Risk Audit Contract"
EVIDENCE = {
    "durable-action-audit-ack-shutdown": (
        "crates/api/src/middleware/audit/writer.rs",
        "durable_ack_syncs_each_action_event_and_shutdown_drains_before_exit",
    ),
    "action-run-restart-fail-closed": (
        "crates/api/src/middleware/audit/replay.rs",
        "replay_turns_interrupted_action_run_into_fail_closed_terminal",
    ),
    "action-run-state-recovery": (
        "crates/api/src/state.rs",
        "app_state_recovers_interrupted_action_run_from_durable_audit_snapshot",
    ),
    "trusted-actor-label": (
        "crates/common/src/config.rs",
        "configured_actor_label_must_be_safe_and_static",
    ),
    "terminal-route-replay-contract": (
        "crates/api/src/services/action_runs/tests/terminal_contract.rs",
        "core_high_risk_mutations_preserve_identity_across_terminal_outcomes",
    ),
    "completion-governance-gate": (
        "scripts/check_pr_fi_completion.sh",
        "bash scripts/check_pr_fi_completion.sh --self-test",
    ),
}
SOURCE_MARKERS = {
    "crates/api/src/middleware/audit.rs": (
        "pub(crate) fn record_durable",
        "pub(crate) fn shutdown",
        "actor_label: Option<&str>",
    ),
    "crates/api/src/middleware/audit/writer.rs": (
        "DURABLE_ACK_TIMEOUT",
        "WriterCommand::Shutdown",
        "file.sync_data()",
        "audit_shutdown_drain_timeout",
    ),
    "crates/api/src/middleware/audit/replay.rs": (
        "pub(crate) fn replay_action_runs",
        "ACTION_RUN_REPLAY_UNAVAILABLE",
        "restart_in_flight",
        "replay_keeps_non_idempotent_action_run_for_audit_visibility",
    ),
    "crates/api/src/services/action_runs/audit_log.rs": (
        "audit::record_durable",
        '"actionRun": snapshot',
        "snapshot.result = None",
        "AUDIT_STORAGE_WRITE_FAILED",
    ),
    "crates/api/src/services/action_runs/lifecycle.rs": (
        "Result<ActionRun, AppError>",
        'record_audit(&run, "accepted")?',
    ),
    "crates/api/src/services/action_runs/mutate.rs": (
        "record_audit(&updated, audit_outcome(&updated))?",
        ".insert(run_id.to_owned(), updated.clone())",
    ),
    "crates/api/src/state.rs": (
        "audit::replay_action_runs",
        "action_runs_from_replay",
    ),
    "crates/api/src/main.rs": (
        "lifecycle::drain_runtime(&state, &mut tasks)",
        ".ensure_clean()?",
    ),
    "crates/api/src/lifecycle/drain.rs": (
        "AUDIT_DRAIN_BUDGET",
        "crate::middleware::audit::shutdown",
        "COMPONENT_AUDIT_LOG",
        "DrainStatus::TimedOut",
    ),
    "crates/common/src/config.rs": (
        "auth_actor_label",
        "ensure_actor_label_safe",
        "verified_actor_label",
    ),
    "crates/api/src/middleware/auth.rs": (
        "verified_actor_label",
        "insert_verified_bearer_actor",
    ),
    "crates/api/src/routers/trading/order_replay.rs": (
        "restart_replay_unavailable",
        "ACTION_RUN_REPLAY_UNAVAILABLE",
    ),
}
CLOSURE_PATHS = tuple(
    """
.env.example
crates/common/src/config.rs
crates/api/src/main.rs
crates/api/src/lifecycle/drain.rs
crates/api/src/state.rs
crates/api/src/middleware/audit.rs
crates/api/src/middleware/audit/writer.rs
crates/api/src/middleware/audit/replay.rs
crates/api/src/middleware/auth.rs
crates/api/src/services/action_runs/audit_log.rs
crates/api/src/services/action_runs/lifecycle.rs
crates/api/src/services/action_runs/mutate.rs
crates/api/src/services/action_runs/tests/lifecycle.rs
crates/api/src/services/action_runs/tests/payload.rs
crates/api/src/services/action_runs/tests/terminal_contract.rs
crates/api/src/services/close_runs/auto_compensation.rs
crates/api/src/services/close_runs/tests/cases_b/finality.rs
crates/api/src/services/close_runs/tests/cases_b/manual_terminal.rs
crates/api/src/services/close_runs/tests/cases_b/unwind.rs
crates/api/src/services/hedge_confirm/confirm.rs
crates/api/src/services/hedge_confirm/confirm_replay.rs
crates/api/src/routers/arbitrage/hedge_tests/pricing_confirm.rs
crates/api/src/routers/arbitrage/hedge_tests/pricing_confirm_partial_outcome.rs
crates/api/src/routers/exchanges.rs
crates/api/src/routers/portfolio.rs
crates/api/src/routers/trading/account.rs
crates/api/src/routers/trading/adapters.rs
crates/api/src/routers/trading/kill_switch.rs
crates/api/src/routers/trading/order_replay.rs
crates/api/src/routers/trading/orders.rs
crates/api/src/routers/trading/tests/cases_kill.rs
scripts/check_pr_fi_completion.sh
scripts/verify_repo_gates.sh
docs/PRODUCT_FULL_AUDIT_REFINEMENT.md
docs/audit_history/PRODUCT_AUDIT_HISTORY.md
docs/PRODUCT_AUDIT_EVIDENCE.tsv
docs/PRODUCT_AUDIT_COVERAGE.tsv
""".split()
)
COVERAGE_PATHS = tuple(
    path
    for path in CLOSURE_PATHS
    if path.startswith(("crates/", "scripts/"))
)


def read_text(root: Path, relative: str, failures: list[str]) -> str:
    path = root / relative
    if not path.is_file():
        failures.append(f"PR-FI required path is missing: {relative}")
        return ""
    return path.read_text(encoding="utf-8")


def validate(root: Path) -> list[str]:
    failures: list[str] = []
    for relative, markers in SOURCE_MARKERS.items():
        text = read_text(root, relative, failures)
        for marker in markers:
            if marker not in text:
                failures.append(f"PR-FI source marker missing: {relative}: {marker}")

    doc = read_text(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", failures)
    roadmap_marker = "### 🟡 6.3 建议整改顺序"
    next_section_marker = "### 🟡 6.4"
    roadmap = doc.split(roadmap_marker, 1)[1] if roadmap_marker in doc else ""
    roadmap = roadmap.split(next_section_marker, 1)[0]
    row = next((line for line in roadmap.splitlines() if f"`{PR_TITLE}`" in line), "")
    if not row:
        failures.append("PR-FI roadmap row is missing")
    else:
        if "✅ 完成" not in row:
            failures.append("PR-FI roadmap row must be marked complete")
        if "剩余：无" not in row:
            failures.append("PR-FI roadmap row must state remaining-none")
        if "check_pr_fi_completion.sh" not in row:
            failures.append("PR-FI roadmap row must cite its completion gate")
    queue_marker = "### 🟡 6.5 下一步执行队列"
    queue = doc.split(queue_marker, 1)[1] if queue_marker in doc else ""
    if f"**{PR_ID}**" in queue:
        failures.append("completed PR-FI must be removed from the active queue")

    ledger_path = root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
    if not ledger_path.is_file():
        failures.append("PR-FI evidence ledger is missing")
    else:
        with ledger_path.open(encoding="utf-8", newline="") as handle:
            rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == PR_ID]
        by_type = {row["evidence_type"]: row for row in rows}
        expected = set(EVIDENCE)
        actual = set(by_type)
        if actual != expected:
            failures.append(
                "PR-FI evidence types drifted: "
                f"expected {sorted(expected)}, got {sorted(actual)}"
            )
        for evidence_type, (artifact, command_anchor) in EVIDENCE.items():
            row = by_type.get(evidence_type)
            if row is None:
                continue
            if row["artifact"] != artifact:
                failures.append(
                    f"PR-FI {evidence_type} artifact must be {artifact}, got {row['artifact']}"
                )
            if command_anchor not in row["command"]:
                failures.append(
                    f"PR-FI {evidence_type} command must include {command_anchor}"
                )

    history = read_text(root, "docs/audit_history/PRODUCT_AUDIT_HISTORY.md", failures)
    if "## PR-FI" not in history:
        failures.append("PR-FI history appendix entry is missing")
    for path in CLOSURE_PATHS:
        if path not in history:
            failures.append(f"PR-FI history is missing closure path: {path}")

    coverage_path = root / "docs/PRODUCT_AUDIT_COVERAGE.tsv"
    if not coverage_path.is_file():
        failures.append("PR-FI coverage ledger is missing")
    else:
        with coverage_path.open(encoding="utf-8", newline="") as handle:
            coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
        for path in COVERAGE_PATHS:
            row = coverage.get(path)
            if row is None or row["coverage_status"] != "exact":
                failures.append(f"PR-FI closure path lacks exact coverage: {path}")
    return failures


def seed_fixture(root: Path) -> None:
    for relative, markers in SOURCE_MARKERS.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(markers) + "\n", encoding="utf-8")
    for relative in CLOSURE_PATHS:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists():
            path.write_text("fixture\n", encoding="utf-8")
    (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").write_text(
        "### 🟡 6.3 建议整改顺序\n"
        "| `PR-FI API Security, Auth/CORS, WS Auth & High-Risk Audit Contract` | ✅ 完成 | "
        "剩余：无。 | 验证：`bash scripts/check_pr_fi_completion.sh`。 |\n"
        "### 🟡 6.4 其它\n"
        "### 🟡 6.5 下一步执行队列\n\n1. **外部等待池**\n",
        encoding="utf-8",
    )
    with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, delimiter="\t", lineterminator="\n")
        writer.writerow(("pr_id", "evidence_type", "artifact", "command", "notes"))
        for evidence_type, (artifact, command_anchor) in EVIDENCE.items():
            writer.writerow((PR_ID, evidence_type, artifact, f"verify {command_anchor}", "fixture"))
    (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").write_text(
        "## PR-FI\n" + "\n".join(CLOSURE_PATHS) + "\n",
        encoding="utf-8",
    )
    with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, delimiter="\t", lineterminator="\n")
        writer.writerow(("file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"))
        for relative in COVERAGE_PATHS:
            writer.writerow((relative, "exact", "fixture", "P0", PR_ID, "hist:1", "fixture"))


if sys.argv[2] == "--self-test":
    with tempfile.TemporaryDirectory(prefix="crossline-pr-fi-gate-") as temporary:
        fixture = Path(temporary) / "fixture"
        seed_fixture(fixture)
        baseline = validate(fixture)
        if baseline:
            raise SystemExit("PR-FI completion self-test baseline failed: " + "; ".join(baseline))
        broken = fixture / "crates/api/src/middleware/audit/writer.rs"
        broken.write_text("missing durable marker\n", encoding="utf-8")
        if not validate(fixture):
            raise SystemExit("PR-FI completion self-test negative fixture unexpectedly passed")
        seed_fixture(fixture)
        broken_drain = fixture / "crates/api/src/lifecycle/drain.rs"
        broken_drain.write_text("audit drain disconnected\n", encoding="utf-8")
        if not validate(fixture):
            raise SystemExit("PR-FI audit-drain negative fixture unexpectedly passed")
    print("OK PR-FI completion gate self-test")
    raise SystemExit(0)

failures = validate(Path(sys.argv[1]))
if failures:
    raise SystemExit("PR-FI completion gate failed\n" + "\n".join(f"  {failure}" for failure in failures))

print(f"OK PR-FI completion gate ({len(CLOSURE_PATHS)} closure paths, {len(EVIDENCE)} evidence rows)")
PY
