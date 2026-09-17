#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
EVENT="$ROOT/crates/api/src/middleware/audit/event.rs"
ACTION_AUDIT="$ROOT/crates/api/src/services/action_runs/audit_log.rs"
WRITER="$ROOT/crates/api/src/middleware/audit/writer.rs"
SUBMIT="$ROOT/crates/api/src/trading_service/submit.rs"
WEBSOCKET="$ROOT/crates/api/src/routers/websocket.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-CJ completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  backup_dir="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-cj.XXXXXX")"
  files=("$EVENT" "$ACTION_AUDIT" "$WRITER" "$SUBMIT" "$WEBSOCKET")
  for file in "${files[@]}"; do
    cp "$file" "$backup_dir/$(basename "$file")"
  done
  restore() {
    for file in "${files[@]}"; do
      cp "$backup_dir/$(basename "$file")" "$file"
    done
    rm -rf "$backup_dir"
  }
  restore_file() {
    cp "$backup_dir/$(basename "$1")" "$1"
  }
  trap restore EXIT

  PR_CJ_SKIP_TESTS=1 PR_CJ_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$EVENT" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    pub(crate) problem_code: Option<String>,"
if source.count(marker) != 1:
    raise SystemExit("PR-CJ self-test event marker drifted")
path.write_text(source.replace(marker, "    pub(crate) problem_tag: Option<String>,", 1), encoding="utf-8")
PY
  if PR_CJ_SKIP_TESTS=1 PR_CJ_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted removal of typed problemCode"
  fi
  restore_file "$EVENT"

  python3 - "$ACTION_AUDIT" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = ".with_context(action_event_context(run))"
if source.count(marker) != 1:
    raise SystemExit("PR-CJ self-test ActionRun context marker drifted")
path.write_text(source.replace(marker, "", 1), encoding="utf-8")
PY
  if PR_CJ_SKIP_TESTS=1 PR_CJ_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted ActionRun audit without typed context"
  fi
  restore_file "$ACTION_AUDIT"

  python3 - "$WRITER" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "mpsc::sync_channel(AUDIT_WRITER_QUEUE_CAPACITY)"
if source.count(marker) != 1:
    raise SystemExit("PR-CJ self-test bounded writer marker drifted")
path.write_text(source.replace(marker, "mpsc::sync_channel(1)", 1), encoding="utf-8")
PY
  if PR_CJ_SKIP_TESTS=1 PR_CJ_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted writer detached from the bounded capacity contract"
  fi
  restore_file "$WRITER"

  python3 - "$SUBMIT" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "        ensure_live_order_mutation_audit_trail(intent.mode)?;"
if source.count(marker) != 2:
    raise SystemExit("PR-CJ self-test live submit marker drifted")
path.write_text(source.replace(marker, "        // audit gate severed by self-test", 1), encoding="utf-8")
PY
  if PR_CJ_SKIP_TESTS=1 PR_CJ_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a live submit path without the audit gate"
  fi
  restore_file "$SUBMIT"

  python3 - "$WEBSOCKET" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    Ping,\n}"
if source.count(marker) != 1:
    raise SystemExit("PR-CJ self-test websocket enum marker drifted")
path.write_text(source.replace(marker, "    SubmitOrder,\n    Ping,\n}", 1), encoding="utf-8")
PY
  if PR_CJ_SKIP_TESTS=1 PR_CJ_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a mutation command on the product websocket"
  fi

  printf 'PR-CJ completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-CJ High-Risk Audit Trail & Request Correlation Contract"
verify_anchor = "`bash scripts/check_pr_cj_completion.sh --self-test`"
evidence = {
    "typed-event-schema": "crates/api/src/middleware/audit/event.rs",
    "action-context-projection": "crates/api/src/services/action_runs/audit_context.rs",
    "action-context-fixtures": "crates/api/src/services/action_runs/audit_context/tests.rs",
    "action-audit-integration": "crates/api/src/services/action_runs/audit_log.rs",
    "bounded-durable-writer": "crates/api/src/middleware/audit/writer.rs",
    "durable-replay-boundary": "crates/api/src/middleware/audit/replay.rs",
    "trusted-actor-boundary": "crates/api/src/middleware/audit.rs",
    "live-mutation-gate": "crates/api/src/trading_service/helpers.rs",
    "route-mutation-matrix": "scripts/check_mutation_audit_contract.sh",
    "websocket-read-only-boundary": "crates/api/src/routers/websocket.rs",
    "non-skipping-product-proof": "test/e2e/pr_dt_request_correlation.spec.ts",
    "completion-governance": "scripts/check_pr_cj_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-CJ completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-CJ\b", queue):
    fail("completed PR-CJ remains in the local queue")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-14 PR-CJ High-Risk Audit Trail and Request Correlation Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-CJ"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence):
    fail(f"evidence type drift: expected={sorted(evidence)}, actual={sorted(indexed)}")
for kind, artifact in evidence.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

event = (root / evidence["typed-event-schema"]).read_text(encoding="utf-8")
for field in (
    "method", "path", "status", "actor_kind", "action_kind", "resource_kind", "run_id",
    "ticket_id", "order_id", "client_order_id", "venue", "symbol", "problem_code",
    "venues", "symbols",
):
    if not re.search(rf"pub\(crate\) {field}: ", event):
        fail(f"typed event field is missing: {field}")
if "#[serde(flatten)]\n    pub context: AuditEventContext" not in event:
    fail("AuditEvent does not flatten the typed context")

projection = (root / evidence["action-context-projection"]).read_text(encoding="utf-8")
for marker in (
    "crate::route_specs::route_specs()",
    "context.action_kind = Some(run.kind);",
    "context.resource_kind = Some(resource_kind(run.kind));",
    "context.problem_code",
    '"/executionRun/runId"',
    '"/executionRun/ticketId"',
    '"/clientOrderId"',
    '"/closeRunId"',
):
    if marker not in projection:
        fail(f"ActionRun typed projection marker is missing: {marker}")

action_audit = (root / evidence["action-audit-integration"]).read_text(encoding="utf-8")
if action_audit.count(".with_context(action_event_context(run))") != 1:
    fail("ActionRun durable audit is not wired to the typed context")

writer = (root / evidence["bounded-durable-writer"]).read_text(encoding="utf-8")
for marker in (
    "mpsc::sync_channel(AUDIT_WRITER_QUEUE_CAPACITY)",
    "sender.try_send(WriterCommand::Write",
    "file.sync_data()",
    "WriterCommand::Shutdown",
):
    if marker not in writer:
        fail(f"bounded durable writer marker is missing: {marker}")

submit = (root / "crates/api/src/trading_service/submit.rs").read_text(encoding="utf-8")
if submit.count("ensure_live_order_mutation_audit_trail(intent.mode)?;") != 2:
    fail("live submit and unwind must both pass the audit readiness gate")
helpers = (root / evidence["live-mutation-gate"]).read_text(encoding="utf-8")
if "ensure_live_order_mutation_audit_trail(record.intent.mode)" not in helpers:
    fail("remote cancel helper is detached from live audit readiness")
if "ensure_remote_cancel_audit_trail(&existing)?;" not in submit:
    fail("remote cancel does not invoke its audit gate before the external engine")

websocket = (root / evidence["websocket-read-only-boundary"]).read_text(encoding="utf-8")
match = re.search(r"enum ClientMessage\s*\{(?P<body>.*?)\n\}", websocket, re.DOTALL)
if match is None:
    fail("ClientMessage enum is missing")
variants = set(re.findall(r"(?m)^\s{4}([A-Z][A-Za-z0-9_]*)\b", match.group("body")))
if variants != {"Auth", "Subscribe", "Unsubscribe", "Ping"}:
    fail(f"product websocket command boundary drifted: {sorted(variants)}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence.values():
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"coverage is not exact for {artifact}")

print(f"PR-CJ static completion contract passed ({len(evidence)} evidence rows)")
PY

bash "$ROOT/scripts/check_mutation_audit_contract.sh"

if [[ "${PR_CJ_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DT_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dt_completion.sh"
  PR_DK_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dk_completion.sh"
fi

if [[ "${PR_CJ_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p api audit_context --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api audit --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api core_high_risk_mutations_preserve_identity_across_terminal_outcomes --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dt -- --workers=1
fi

printf 'PR-CJ completion gate passed\n'
