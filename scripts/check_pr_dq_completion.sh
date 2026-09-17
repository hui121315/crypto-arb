#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
LLM_CONTRACT="$ROOT/shared-types/src/llm.rs"
FRONTEND_REST="$ROOT/frontend/src/api/rest.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DQ_SKIP_TESTS=1 bash "$0" >/dev/null
  llm_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dq-llm.XXXXXX")"
  rest_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dq-rest.XXXXXX")"
  cp "$LLM_CONTRACT" "$llm_backup"
  cp "$FRONTEND_REST" "$rest_backup"
  restore() {
    cp "$llm_backup" "$LLM_CONTRACT"
    cp "$rest_backup" "$FRONTEND_REST"
    rm -f "$llm_backup" "$rest_backup"
  }
  trap restore EXIT

  python3 - "$LLM_CONTRACT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "pub const MAX_LLM_EXTERNAL_PAYLOAD_BYTES: usize = 16 * 1024;"
if source.count(marker) != 1:
    raise SystemExit("PR-DQ self-test setup failed: LLM byte-cap marker drifted")
path.write_text(source.replace(marker, "pub const MAX_LLM_EXTERNAL_PAYLOAD_BYTES: usize = usize::MAX;", 1), encoding="utf-8")
PY
  if PR_DQ_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DQ completion self-test failed: unbounded LLM payload passed\n' >&2
    exit 1
  fi

  cp "$llm_backup" "$LLM_CONTRACT"
  printf '\nmod simulation;\n' >>"$FRONTEND_REST"
  if PR_DQ_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DQ completion self-test failed: simulation client re-entry passed\n' >&2
    exit 1
  fi

  printf 'PR-DQ completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-DQ Non-P0 Client DTO Cleanup & External Payload Safety"
evidence_contract = {
    "llm-external-payload-contract": "shared-types/src/llm.rs",
    "llm-outbound-audit-contract": "crates/api/src/routers/chat.rs",
    "llm-provider-evidence-contract": "crates/llm/src/evidence.rs",
    "options-not-integrated-contract": "crates/api/src/routers/options.rs",
    "simulation-http-surface-deletion": "crates/api/src/route_specs.rs",
    "frontend-dead-client-boundary": "frontend/src/api/rest.rs",
    "upstream-pr-fk-governance": "scripts/check_pr_fk_completion.sh",
    "upstream-pr-fm-governance": "scripts/check_pr_fm_completion.sh",
    "completion-governance": "scripts/check_pr_dq_completion.sh",
}
closure_paths = (
    "shared-types/src/lib.rs",
    "shared-types/src/llm.rs",
    "shared-types/src/problem.rs",
    "crates/llm/src/evidence.rs",
    "crates/llm/src/error.rs",
    "crates/llm/src/provider.rs",
    "crates/llm/src/router.rs",
    "crates/llm/src/providers/deepseek.rs",
    "crates/llm/tests/providers_test.rs",
    "crates/api/Cargo.toml",
    "crates/api/src/app.rs",
    "crates/api/src/route_specs.rs",
    "crates/api/src/routers/chat.rs",
    "crates/api/src/routers/options.rs",
    "crates/common/src/config.rs",
    "frontend/src/api/rest.rs",
    "frontend/src/api/rest/dto.rs",
    "scripts/check_frontend_dto_mirror.sh",
    "scripts/check_pr_fk_completion.sh",
    "scripts/check_pr_fm_completion.sh",
    "scripts/check_pr_dq_completion.sh",
    "scripts/verify_repo_gates.sh",
)
test_anchors = (
    ("shared-types/src/llm.rs", "sensitive_markers_fail_closed_before_external_serialization"),
    ("shared-types/src/llm.rs", "unknown_raw_fields_cannot_deserialize_into_allowlisted_contract"),
    ("shared-types/src/llm.rs", "multibyte_summary_respects_byte_cap"),
    ("crates/api/src/routers/chat.rs", "legacy_raw_chat_message_body_is_not_a_supported_contract"),
    ("crates/api/src/routers/chat.rs", "llm_audit_detail_never_contains_payload_summary"),
    ("crates/api/src/routers/options.rs", "positions_route_returns_not_integrated_envelope"),
    ("crates/api/src/app.rs", "deleted_simulation_routes_stay_absent_with_all_remaining_surfaces_enabled"),
    ("frontend/src/api/rest.rs", "legacy_options_and_simulation_cannot_reenter_the_main_rest_client"),
)
source_markers = {
    "shared-types/src/llm.rs": (
        "pub const MAX_LLM_EXTERNAL_PAYLOAD_BYTES: usize = 16 * 1024;",
        "pub struct LlmExternalPayload",
        "deny_unknown_fields",
        "reject_sensitive_marker",
    ),
    "crates/api/src/routers/chat.rs": (
        "Json(request): Json<LlmExternalRequest>",
        "ensure_external_readiness",
        "record_llm_durable_audit",
        '"redactedBytes"',
    ),
    "crates/api/src/routers/options.rs": (
        "OPTIONS_POSITIONS_UNSUPPORTED",
        "positions_unsupported_error",
    ),
    "crates/api/src/app.rs": (
        "deleted_simulation_routes_stay_absent_with_all_remaining_surfaces_enabled",
    ),
    "frontend/src/api/rest.rs": (
        "legacy_options_and_simulation_cannot_reenter_the_main_rest_client",
    ),
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DQ completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if "scripts/check_pr_dq_completion.sh" not in row:
    fail("roadmap row must name the PR-DQ completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DQ\b", queue):
    fail("completed PR-DQ remains in the local queue")
for upstream in (
    "PR-FK LLM External Payload, Redaction, Feature Gate & Provider Evidence Contract",
    "PR-FM Options Simulation Surface, Unsupported Semantics & Legacy Route Contract",
):
    upstream_row = next((line for line in doc.splitlines() if line.startswith(f"| `{upstream}`")), None)
    if upstream_row is None or "✅ 完成" not in upstream_row or "剩余：无。" not in upstream_row:
        fail(f"upstream completion drifted: {upstream}")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DQ"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    row = indexed[kind]
    if row["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")
    if not row["command"].strip():
        fail(f"{kind} lacks a verification command")

for relative_path, markers in source_markers.items():
    source = (root / relative_path).read_text(encoding="utf-8")
    for marker in markers:
        if marker not in source:
            fail(f"missing {relative_path} marker: {marker}")

api_cargo = (root / "crates/api/Cargo.toml").read_text(encoding="utf-8")
default_features = re.search(r"(?m)^default\s*=\s*\[([^\]]*)\]", api_cargo)
if default_features is None or "legacy-chat" in default_features.group(1) or "legacy-options" in default_features.group(1):
    fail("LLM/options diagnostic surfaces must remain default-off")
if "legacy-simulation" in api_cargo or "dep:simulation" in api_cargo:
    fail("deleted simulation HTTP dependency or feature returned")
for removed in (
    "crates/api/src/routers/simulation.rs",
    "frontend/src/api/rest/chat.rs",
    "frontend/src/api/rest/options.rs",
    "frontend/src/api/rest/simulation.rs",
):
    if (root / removed).exists():
        fail(f"removed client or HTTP surface returned: {removed}")
rest = (root / "frontend/src/api/rest.rs").read_text(encoding="utf-8")
if re.search(r"(?m)^\s*mod\s+(?:chat|options|simulation)\s*;\s*$", rest):
    fail("dead chat/options/simulation wrapper re-entered the main REST client")
dto = (root / "frontend/src/api/rest/dto.rs").read_text(encoding="utf-8")
if re.search(r"(?m)^\s*pub\s+(?:struct|enum)\s+", dto):
    fail("frontend REST DTO mirror returned")

for relative_path, function_name in test_anchors:
    source = (root / relative_path).read_text(encoding="utf-8")
    pattern = re.compile(
        rf"(?m)((?:^\s*#\[[^\n]+\]\s*\n)+)"
        rf"\s*(?:async\s+)?fn\s+{re.escape(function_name)}\s*\("
    )
    match = pattern.search(source)
    if match is None or not re.search(r"#\[(?:tokio::)?test", match.group(1)):
        fail(f"missing runnable test anchor {relative_path}::{function_name}")
    if re.search(r"ignore|should_panic", match.group(1)):
        fail(f"test anchor is skipped: {relative_path}::{function_name}")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-14 PR-DQ External Payload and Dead Client Closure" not in history:
    fail("PR-DQ history appendix is missing")
for artifact in set(evidence_contract.values()):
    if artifact not in history:
        fail(f"PR-DQ history appendix lacks closure path: {artifact}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage_rows = list(csv.DictReader(handle, delimiter="\t"))
coverage = {row["file"]: row for row in coverage_rows}
if len(coverage) != len(coverage_rows):
    fail("coverage ledger contains duplicate paths")
for relative_path in closure_paths:
    if not (root / relative_path).is_file():
        fail(f"missing closure path: {relative_path}")
    if coverage.get(relative_path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {relative_path}")

print(
    f"OK PR-DQ contract ({len(evidence_contract)} evidence types; "
    f"{len(test_anchors)} non-skipping tests; {len(closure_paths)} exact paths)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_pr_fk_completion.sh"

if [[ "${PR_DQ_SKIP_TESTS:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_pr_fm_completion.sh"
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p shared-types --lib llm --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p llm --all-targets --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p api --bin crypto-arb-api \
    --features legacy-chat routers::chat --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p api --bin crypto-arb-api \
    --features legacy-options routers::options --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/frontend/Cargo.toml" --lib \
    legacy_options_and_simulation_cannot_reenter_the_main_rest_client --no-fail-fast
  bash "$ROOT/scripts/check_frontend_dto_mirror.sh"
fi

printf 'OK PR-DQ non-P0 client and external payload contract\n'
