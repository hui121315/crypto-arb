#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
WS_RUNTIME="$ROOT/frontend/src/api/ws_runtime.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DL_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dl.XXXXXX")"
  cp "$WS_RUNTIME" "$backup"
  restore() {
    cp "$backup" "$WS_RUNTIME"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$WS_RUNTIME" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '"requestId": request_id,'
if source.count(marker) != 1:
    raise SystemExit("PR-DL self-test setup failed: websocket requestId marker drifted")
path.write_text(source.replace(marker, '"legacyRequestId": request_id,'), encoding="utf-8")
PY
  if PR_DL_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DL completion self-test failed: requestId regression passed\n' >&2
    exit 1
  fi
  printf 'PR-DL completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-DL Frontend Transport Runtime & ApiProblem Contract"
verify_anchor = "`bash scripts/check_pr_dl_completion.sh --self-test`"
evidence_contract = {
    "shared-request-id": "frontend/src/api/request_id.rs",
    "rest-request-id": "frontend/src/api/rest/transport.rs",
    "ws-command-correlation": "frontend/src/api/ws_runtime.rs",
    "ws-server-correlation": "crates/api/src/routers/websocket.rs",
    "dto-single-source-gate": "scripts/check_frontend_dto_mirror.sh",
    "load-state-governance": "scripts/verify_repo_gates.sh",
    "product-browser": "test/e2e/pr_dl_transport_runtime.spec.ts",
    "product-suite-contract": "package.json",
    "release-browser-contract": "scripts/check_release_qa_contract.sh",
    "completion-governance": "scripts/check_pr_dl_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DL completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DL\b", queue):
    fail("completed PR-DL remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DL"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "frontend/src/api/request_id.rs": (
        'format!("web-{}", request_id_entropy())',
        "generated_request_id_is_frontend_scoped_and_unique",
    ),
    "frontend/src/api/rest/transport.rs": (
        "crate::api::request_id::next_request_id",
        "let request_id = next_request_id();",
        "HEADER_REQUEST_ID",
    ),
    "frontend/src/api/ws_runtime.rs": (
        "struct PendingSubscribe",
        '"requestId": request_id,',
        '"WS_ACK_REQUEST_ID_UNKNOWN"',
        "runtime_out_of_order_ack_correlates_by_request_id",
        "runtime_unknown_ack_request_id_is_typed_and_non_connecting",
    ),
    "crates/api/src/routers/websocket.rs": (
        'rename = "requestId"',
        "control_request_id",
        "Some(\"web-subscribe-command\")",
        "subscribe_command_parses_and_normalizes_request_id",
    ),
    "scripts/verify_repo_gates.sh": (
        "frontend async API errors must not be erased with .await.ok()",
        "frontend resources must expose LoadState instead of module-local LocalResource<Option>",
        "check_frontend_dto_mirror.sh",
        "check_pr_dl_completion.sh",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

browser_path = evidence_contract["product-browser"]
browser = (root / browser_path).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in (
    "PR-DL websocket subscribe correlates browser request id before marking connected",
    "PR-DL mismatched websocket ack stays typed without hiding REST fallback rows",
    "requestId does not match a pending command",
    "text?.includes(`request_id ${id}-stale`)",
):
    if marker not in browser:
        fail(f"browser fixture missing transport marker: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
if scripts.get("test:e2e:pr-dl") != "playwright test test/e2e/pr_dl_transport_runtime.spec.ts":
    fail("dedicated browser command is missing")
if scripts.get("test:e2e:product", "").count(browser_path) != 1:
    fail("product suite must include the PR-DL fixture exactly once")
release_fixture = json.loads(
    (root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8")
)
if release_fixture.get("scripts", {}).get("test:e2e:product", "").count(browser_path) != 1:
    fail("release QA fixture must include the PR-DL browser fixture exactly once")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if path == "package.json":
        continue
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-DL contract ({len(evidence_contract)} evidence types; "
    "request-id correlation + DTO/LoadState governance + Chromium closure)"
)
PY

bash "$ROOT/scripts/check_frontend_dto_mirror.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"

if [[ "${PR_DL_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-8}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p api routers::websocket::tests --no-fail-fast
  cargo test --manifest-path "$ROOT/frontend/Cargo.toml" api::request_id::tests --no-fail-fast
  cargo test --manifest-path "$ROOT/frontend/Cargo.toml" api::ws_runtime::tests --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dl -- --workers=1
fi

printf 'OK PR-DL frontend transport runtime and ApiProblem contract\n'
