#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
MATRIX="$ROOT/scripts/exchange_operation_evidence_matrix.tsv"
WS_CONTRACT="$ROOT/shared-types/src/exchange_ws.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BZ completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  temp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-bz.XXXXXX")"
  cp "$DOC" "$temp/audit.md"
  cp "$MATRIX" "$temp/matrix.tsv"
  cp "$WS_CONTRACT" "$temp/exchange-ws.rs"
  restore() {
    cp "$temp/audit.md" "$DOC"
    cp "$temp/matrix.tsv" "$MATRIX"
    cp "$temp/exchange-ws.rs" "$WS_CONTRACT"
    rm -rf "$temp"
  }
  trap restore EXIT

  PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$MATRIX" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "Binance\trecorded\trecorded\trecorded\trecorded\t"
if source.count(marker) != 1:
    raise SystemExit("PR-BZ self-test setup failed: recorded matrix marker drifted")
path.write_text(source.replace(marker, "Binance\tmissing\trecorded\trecorded\trecorded\t", 1), encoding="utf-8")
PY
  if PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a missing REST operation bucket"
  fi
  cp "$temp/matrix.tsv" "$MATRIX"

  python3 - "$MATRIX" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
match = re.search(r"sha256:([0-9a-f]{64})", source)
if match is None:
    raise SystemExit("PR-BZ self-test setup failed: fixture hash marker drifted")
digest = match.group(1)
replacement = digest[:-1] + ("0" if digest[-1] != "0" else "1")
path.write_text(source[:match.start(1)] + replacement + source[match.end(1):], encoding="utf-8")
PY
  if PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a stale fixture hash"
  fi
  cp "$temp/matrix.tsv" "$MATRIX"

  python3 - "$MATRIX" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "\tack_not_final\n"
if marker not in source:
    raise SystemExit("PR-BZ self-test setup failed: finality boundary drifted")
path.write_text(source.replace(marker, "\tack_is_final\n", 1), encoding="utf-8")
PY
  if PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted an ACK-as-final boundary"
  fi
  cp "$temp/matrix.tsv" "$MATRIX"

  python3 - "$WS_CONTRACT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "&& self.evidence.as_ref().is_some_and(|evidence| {"
if source.count(marker) != 1:
    raise SystemExit("PR-BZ self-test setup failed: live evidence marker drifted")
path.write_text(source.replace(marker, "|| self.evidence.as_ref().is_some_and(|evidence| {", 1), encoding="utf-8")
PY
  if PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a live writer evidence bypass"
  fi
  cp "$temp/exchange-ws.rs" "$WS_CONTRACT"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "### 🟡 6.5 下一步执行队列"
if source.count(marker) != 1:
    raise SystemExit("PR-BZ self-test setup failed: queue heading drifted")
stale = "\n\n1. **PR-BZ Exchange Official Evidence Registry & Fixture Gate** — stale completed row"
path.write_text(source.replace(marker, marker + stale, 1), encoding="utf-8")
PY
  if PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted completed PR-BZ in the local queue"
  fi
  cp "$temp/audit.md" "$DOC"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "### 🟡 6.5 下一步执行队列"
if source.count(marker) != 1:
    raise SystemExit("PR-BZ self-test setup failed: queue heading drifted")
stale = "\n\n1. **PR-CB Settings Credential Health & ActionState Contract** — stale completed row"
path.write_text(source.replace(marker, marker + stale, 1), encoding="utf-8")
PY
  if PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted completed PR-CB at the local queue head"
  fi
  cp "$temp/audit.md" "$DOC"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "真实 live 脱敏样本仅属外部等待池，不阻塞本地完成状态"
if source.count(marker) < 2:
    raise SystemExit("PR-BZ self-test setup failed: external boundary marker drifted")
path.write_text(source.replace(marker, "真实 live 脱敏样本已在本地完成", 1), encoding="utf-8")
PY
  if PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a false local live-sample claim"
  fi
  cp "$temp/audit.md" "$DOC"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-FW Exchange Official Evidence, Adapter Schema Fixture & Trading Capability Gate` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-BZ self-test setup failed: PR-FW authority marker drifted")
path.write_text(source.replace(marker, marker.replace("✅ 完成", "🟡 部分完成"), 1), encoding="utf-8")
PY
  if PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted an unfinished registry authority"
  fi

  printf 'PR-BZ completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import hashlib
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
pr_id = "PR-BZ"
title = "PR-BZ Exchange Official Evidence Registry & Fixture Gate"
verify_anchor = "`bash scripts/check_pr_bz_completion.sh --self-test`"
external_boundary = "真实 live 脱敏样本仅属外部等待池，不阻塞本地完成状态"
evidence_contract = {
    "canonical-operation-matrix": "scripts/exchange_operation_evidence_matrix.tsv",
    "endpoint-evidence-governance": "scripts/check_exchange_evidence_debt.sh",
    "operation-evidence-governance": "scripts/check_exchange_operation_evidence_matrix.sh",
    "rest-runtime-registry": "crates/exchange/src/rest_registry.rs",
    "ws-runtime-registry": "crates/exchange/src/ws/trading.rs",
    "shared-ws-submission-boundary": "shared-types/src/exchange_ws.rs",
    "shared-transport-summary": "shared-types/src/transport_registry.rs",
    "api-transport-registry-parity": "crates/api/src/routers/trading/tests/cases_registry.rs",
    "diagnostic-fixture-closure": "crates/exchange/tests/diag_real_responses.rs",
    "runtime-registry-parity": "crates/exchange/tests/ws_trading_specs_test.rs",
    "okx-venue-authority": "scripts/check_pr_ek_completion.sh",
    "binance-venue-authority": "scripts/check_pr_el_completion.sh",
    "bybit-venue-authority": "scripts/check_pr_em_completion.sh",
    "bitget-venue-authority": "scripts/check_pr_en_completion.sh",
    "kucoin-venue-authority": "scripts/check_pr_eo_completion.sh",
    "htx-venue-authority": "scripts/check_pr_ep_completion.sh",
    "hyperliquid-venue-authority": "scripts/check_pr_eq_completion.sh",
    "gate-venue-authority": "scripts/check_pr_er_completion.sh",
    "instrument-identity-authority": "scripts/check_pr_eb_completion.sh",
    "execution-finality-authority": "scripts/check_pr_bw_completion.sh",
    "fee-registry-authority": "scripts/check_pr_bv_completion.sh",
    "runtime-health-authority": "scripts/check_pr_bx_completion.sh",
    "audit-evidence-index": "scripts/check_product_audit_evidence_index.sh",
    "completion-governance": "scripts/check_pr_bz_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-BZ completion gate failed: {message}")


def require_incomplete_queue_head(doc: str, queue: str) -> None:
    matched = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    if not matched:
        fail("local queue must retain a PR roadmap item at its head")
    head = matched.group(1)
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{head} ")), None)
    if row is None or "✅ 完成" in row:
        fail(f"local queue head must be an incomplete roadmap row: {head}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
fact_lines = history.split("## 附录 I：", 1)[0].splitlines() + doc.splitlines()
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be complete with no local remainder")
if row.count(verify_anchor) != 1 or external_boundary not in row:
    fail("roadmap row must bind the destructive gate and external live boundary")

queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-BZ\b", queue):
    fail("completed PR-BZ remains in the local queue")
successor_title = "PR-CA Route Inventory & High-Risk Audit Gate"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
    None,
)
if successor_row is None:
    fail("PR-CA successor roadmap row is missing")
successor_complete = "✅ 完成" in successor_row and "剩余：无。" in successor_row
successor_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CA\b", queue))
next_title = "PR-CB Settings Credential Health & ActionState Contract"
next_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{next_title}`")),
    None,
)
if next_row is None:
    fail("PR-CB next-successor roadmap row is missing")
next_complete = "✅ 完成" in next_row and "剩余：无。" in next_row
next_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CB\b", queue))
following_title = "PR-CD Portfolio AccountState Evidence & Risk LoadState Contract"
following_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{following_title}`")),
    None,
)
if following_row is None:
    fail("PR-CD following-successor roadmap row is missing")
following_complete = "✅ 完成" in following_row and "剩余：无。" in following_row
following_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CD\b", queue))
final_title = "PR-CM Venue API Status Center & Runtime Probe Contract"
final_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{final_title}`")),
    None,
)
if final_row is None:
    fail("PR-CM final-successor roadmap row is missing")
final_complete = "✅ 完成" in final_row and "剩余：无" in final_row
final_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CM\b", queue))
after_title = "PR-CN Frontend Workstation State & Navigation Runtime Contract"
after_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{after_title}`")),
    None,
)
if after_row is None:
    fail("PR-CN after-successor roadmap row is missing")
after_complete = "✅ 完成" in after_row and "剩余：无" in after_row
after_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CN\b", queue))
terminal_title = "PR-CQ Local Runtime & Operator QA Contract"
terminal_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{terminal_title}`")),
    None,
)
if terminal_row is None:
    fail("PR-CQ terminal-successor roadmap row is missing")
terminal_complete = "✅ 完成" in terminal_row and "剩余：无" in terminal_row
terminal_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CQ\b", queue))
if successor_complete:
    if successor_is_head:
        fail("completed PR-CA successor remains at the local queue head")
    if next_complete:
        if next_is_head:
            fail("completed PR-CB next-successor remains at the local queue head")
        if following_complete:
            if following_is_head:
                fail("completed PR-CD following-successor remains at the local queue head")
            if final_complete:
                if final_is_head:
                    fail("completed PR-CM final-successor remains at the local queue head")
                if after_complete:
                    if after_is_head:
                        fail("completed PR-CN after-successor remains at the local queue head")
                    if terminal_complete:
                        if terminal_is_head:
                            fail("completed PR-CQ terminal-successor remains at the local queue head")
                        require_incomplete_queue_head(doc, queue)
                    elif not terminal_is_head:
                        fail("PR-CQ must become the local queue head after PR-CN completion")
                elif not after_is_head:
                    fail("PR-CN must become the local queue head after PR-CM completion")
            elif not final_is_head:
                fail("PR-CM must become the local queue head after PR-CD completion")
        elif not following_is_head:
            fail("PR-CD must become the local queue head after PR-CB completion")
    elif not next_is_head:
        fail("PR-CB must become the local queue head after PR-CA completion")
elif not successor_is_head:
    fail("PR-CA must remain the local queue head until its completion contract closes")
queue_items = re.findall(r"(?m)^\d+\.\s+\*\*", queue)
if not queue_items:
    fail("local queue plus external pool must not be empty")
if queue.count(external_boundary) != 1:
    fail("external waiting pool must retain the live-sample boundary exactly once")

for line in doc.splitlines():
    if "PR-BZ" not in line or not line.startswith("|"):
        continue
    cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
    if any(marker in cell for cell in cells[:2] for marker in ("🟡", "⏳", "❌")):
        fail(f"unfinished audit row still delegates work to PR-BZ: {cells[0]}")

for audit_id in (
    "AUD-113", "AUD-114", "AUD-115", "AUD-116", "AUD-117", "AUD-118",
    "AUD-119", "AUD-120", "AUD-123", "AUD-147", "AUD-156", "AUD-167",
    "AUD-194",
):
    audit = next((line for line in reversed(fact_lines) if f"`{audit_id}`" in line), None)
    if audit is None or "✅ 完成" not in audit:
        fail(f"absorbed audit row remains incomplete: {audit_id}")

finding = next(
    (line for line in reversed(fact_lines) if "HTX/KuCoin `SchemaPending` 写路径的生产与 UI/runtime 防绕过" in line),
    None,
)
if finding is None or "✅ 完成" not in finding:
    fail("SchemaPending UI/runtime boundary finding remains incomplete")

for successor in (
    "PR-FW", "PR-EK", "PR-EL", "PR-EM", "PR-EN", "PR-EO", "PR-EP",
    "PR-EQ", "PR-ER", "PR-EB", "PR-BW", "PR-BV", "PR-BX",
):
    successor_row = next(
        (line for line in doc.splitlines() if line.startswith(f"| `{successor} ")),
        None,
    )
    if successor_row is None or "✅ 完成" not in successor_row:
        fail(f"required successor authority is not complete: {successor}")

if "## 2026-07-15 PR-BZ Exchange Official Evidence Registry and Fixture Gate Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [entry for entry in csv.DictReader(handle, delimiter="\t") if entry["pr_id"] == pr_id]
indexed = {entry["evidence_type"]: entry for entry in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{evidence_type} artifact drifted or is missing")
    if not evidence["command"].strip() or not evidence["notes"].strip():
        fail(f"{evidence_type} lacks command or notes")

matrix_path = root / "scripts/exchange_operation_evidence_matrix.tsv"
with matrix_path.open(encoding="utf-8", newline="") as handle:
    matrix_rows = list(csv.DictReader(handle, delimiter="\t"))
expected_venues = {"Binance", "Okx", "Bybit", "Bitget", "Gate", "Htx", "Kucoin", "Hyperliquid"}
if {entry["venue"] for entry in matrix_rows} != expected_venues or len(matrix_rows) != 8:
    fail("canonical operation matrix must contain exactly the eight supported venues")

seen_paths: set[str] = set()
for entry in matrix_rows:
    venue = entry["venue"]
    for column in (
        "rest_trade_write_order_ack", "rest_private_order_status",
        "rest_private_account_balance", "rest_private_account_position",
    ):
        if entry[column] != "recorded":
            fail(f"{venue} REST operation bucket is not recorded: {column}")
    expected_ws = (
        "display_only_schema_pending" if venue == "Htx"
        else "display_only_runtime_evidence" if venue == "Kucoin"
        else "recorded_place_cancel"
    )
    if entry["ws_live_write_path"] != expected_ws or entry["ws_private_stream_evidence"] != "recorded":
        fail(f"{venue} WS operation boundary drifted")
    if entry["close_position_boundary"] != "display_only_without_operation_evidence":
        fail(f"{venue} close-position boundary drifted")
    if entry["diagnostic_fixture_boundary"] != "committed_diag_fixture":
        fail(f"{venue} diagnostic fixture boundary drifted")
    if entry["finality_boundary"] != "ack_not_final":
        fail(f"{venue} ACK/finality boundary drifted")

    fixture_groups = (
        (entry["diagnostic_fixture_ids"], entry["diagnostic_fixture_hashes"]),
        (entry["ws_operation_fixture_ids"], entry["ws_operation_fixture_hashes"]),
    )
    for raw_paths, raw_hashes in fixture_groups:
        paths = [value.strip() for value in raw_paths.split(",") if value.strip()]
        hashes = [value.strip() for value in raw_hashes.split(",") if value.strip()]
        if not paths or len(paths) != len(hashes):
            fail(f"{venue} fixture/hash cardinality drifted")
        for artifact, expected_hash in zip(paths, hashes, strict=True):
            if artifact in seen_paths:
                fail(f"fixture reused across matrix rows: {artifact}")
            seen_paths.add(artifact)
            fixture = root / artifact
            if not fixture.is_file():
                fail(f"matrix fixture is missing: {artifact}")
            actual_hash = "sha256:" + hashlib.sha256(fixture.read_bytes()).hexdigest()
            if actual_hash != expected_hash:
                fail(f"matrix fixture hash drifted: {artifact}")

markers = {
    "shared-types/src/exchange_ws.rs": (
        "pub fn is_live_submittable(&self) -> bool",
        "&& self.evidence.as_ref().is_some_and(|evidence| {",
        "ExchangeWsReleaseStatus::ProductionReady",
        "ExchangeWsSupportStatus::Ready | ExchangeWsSupportStatus::RequiresPermission",
    ),
    "crates/exchange/src/rest_registry.rs": (
        "operation_matrix_rest_buckets_match_runtime_registry_projection",
        "operation_matrix_rest_buckets_match_exact_allowlist_endpoint_projection",
        "OPERATION_MATRIX_TSV",
    ),
    "crates/exchange/src/ws/trading.rs": (
        "pub fn trading_ws_operation_registry()",
        "ws_operation_registry_keeps_close_position_display_only",
        "fixture_hash: evidence.fixture_hash.map(str::to_owned)",
    ),
    "shared-types/src/transport_registry.rs": (
        "rest_operation_matrix_recorded_bucket_count",
        "rest_operation_matrix_missing_bucket_count",
        "transport_registry_summary_requires_full_rest_operation_evidence_metadata",
    ),
    "crates/exchange/tests/ws_trading_specs_test.rs": (
        "operation_matrix_boundaries_match_ws_registry_projection",
        "operation_matrix_ws_fixtures_match_runtime_registry_evidence",
        "close_position_without_operation_evidence_stays_display_only",
    ),
    "crates/api/src/routers/trading/tests/cases_registry.rs": (
        "transport_registry_route_summary_matches_operation_matrix_tsv",
        "transport_registry_route_preserves_rest_operation_matrix_buckets",
        "assert_eq!(0, response.summary.ws_close_position_rows)",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

diagnostic = (root / "crates/exchange/tests/diag_real_responses.rs").read_text(encoding="utf-8")
if re.search(r"#\[(?:ignore|should_panic)]", diagnostic):
    fail("diagnostic evidence tests must remain non-skipping")
for venue in ("binance", "okx", "bybit", "bitget", "gate", "htx", "kucoin", "hyperliquid"):
    if f"diagnose_{venue}_real_response" not in diagnostic:
        fail(f"missing runnable diagnostic fixture: {venue}")

verify_source = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if verify_source.count("check_pr_bz_completion.sh") != 2:
    fail("repo docs/full gate wiring drifted")
predecessor = (root / "scripts/check_pr_by_completion.sh").read_text(encoding="utf-8")
for marker in (
    'successor_title = "PR-BZ Exchange Official Evidence Registry & Fixture Gate"',
    "successor_complete",
    "successor_is_head",
):
    if marker not in predecessor:
        fail(f"PR-BY predecessor is not successor-aware: {marker}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {entry["file"]: entry for entry in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence_contract.values():
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

print(
    f"OK PR-BZ static contract ({len(evidence_contract)} evidence rows; "
    "8 venues, 32 recorded REST buckets, pinned fixtures, ACK/finality and external-live boundaries)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_BZ_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_exchange_evidence_debt.sh"
  bash "$ROOT/scripts/check_exchange_operation_evidence_matrix.sh"
  PR_EK_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ek_completion.sh"
  bash "$ROOT/scripts/check_pr_el_completion.sh"
  PR_EM_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_em_completion.sh"
  PR_EN_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_en_completion.sh"
  bash "$ROOT/scripts/check_pr_eo_completion.sh"
  bash "$ROOT/scripts/check_pr_ep_completion.sh"
  bash "$ROOT/scripts/check_pr_eq_completion.sh"
  bash "$ROOT/scripts/check_pr_er_completion.sh"
fi

if [[ "${PR_BZ_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types exchange_ws --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange rest_registry --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange ws_operation_registry --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test ws_trading_specs_test --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test diag_real_responses --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api registry_route --no-fail-fast
fi

printf 'PR-BZ Exchange Official Evidence Registry and Fixture Gate completion passed\n'
