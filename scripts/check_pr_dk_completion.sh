#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
TRACE="$ROOT/crates/api/src/middleware/trace.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DK_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dk.XXXXXX")"
  cp "$TRACE" "$backup"
  restore() {
    cp "$backup" "$TRACE"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$TRACE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "request.extensions_mut().insert(request_id);"
if source.count(marker) != 1:
    raise SystemExit("PR-DK self-test setup failed: request extension marker drifted")
path.write_text(
    source.replace(marker, "let _request_id_extension = request_id;"),
    encoding="utf-8",
)
PY
  if PR_DK_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DK completion self-test failed: request extension regression passed\n' >&2
    exit 1
  fi
  printf 'PR-DK completion self-test passed\n'
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
title = "PR-DK API Security Surface & High-Risk Audit Contract"
verify_anchor = "`bash scripts/check_pr_dk_completion.sh --self-test`"
browser_command = (
    'playwright test test/e2e/route_registry.spec.ts test/e2e/data_pipeline.spec.ts '
    '--grep "every always-on bearer route preserves browser auth and typed error boundaries|'
    'high-risk action and secret mutation persist correlated redacted audit pairs|'
    'opportunities websocket authenticates with ticket before subscribe"'
)
evidence_contract = {
    "bind-security": "crates/common/src/config.rs",
    "request-extension": "crates/api/src/middleware/trace.rs",
    "security-runtime-gate": "scripts/verify_security_contract.sh",
    "route-classification": "scripts/check_route_inventory.sh",
    "route-governance": "scripts/check_pr_fh_completion.sh",
    "high-risk-audit-matrix": "scripts/check_mutation_audit_contract.sh",
    "durable-action-audit": "scripts/check_pr_fi_completion.sh",
    "ws-channel-boundary": "crates/api/src/routers/websocket.rs",
    "browser-rest-security": "test/e2e/route_registry.spec.ts",
    "browser-ws-auth": "test/e2e/data_pipeline.spec.ts",
    "product-suite-contract": "package.json",
    "completion-governance": "scripts/check_pr_dk_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DK completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DK\b", queue):
    fail("completed PR-DK remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DK"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "crates/common/src/config.rs": (
        "non_loopback_with_wildcard_cors_origin_is_rejected",
        "configure APP_SECURITY__ALLOWED_ORIGINS with explicit origins (no `*`)",
    ),
    "crates/api/src/middleware/trace.rs": (
        "pub(crate) struct RequestId(String);",
        "request.extensions_mut().insert(request_id);",
        "request_extension_carries_normalized_request_id",
    ),
    "scripts/verify_security_contract.sh": (
        "public_bind_wildcard_cors_gate",
        "request_id_extension",
        "api_security_runtime_smoke",
    ),
    "docs/API_ROUTE_INVENTORY.tsv": (
        "auth_policy\taudit_policy",
        "/api/auth/ws-ticket\tPOST\t",
        "/ws\tGET\twebsocket\tmain_p0\talways\tmedium\tbearer_ws\tread",
    ),
    "scripts/check_mutation_audit_contract.sh": (
        "/api/arbitrage/opportunities/:id/confirm",
        "/api/trading/orders/:id/cancel",
        "/api/trading/portfolio/positions/:venue/:symbol/close-pair",
        "inventory_high_risk_count",
    ),
    "crates/api/src/routers/websocket.rs": (
        "const MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 32;",
        "unknown_channel_is_rejected",
        "cap_blocks_new_subscriptions",
        "subscribe_requires_auth_when_security_enabled",
    ),
    "scripts/check_pr_fh_completion.sh": (
        "Route Inventory, Legacy Feature Gate & Product Surface Contract",
        "route-default-off-security-runtime",
    ),
    "scripts/check_pr_fi_completion.sh": (
        "terminal-route-replay-contract",
        "durable-action-audit-ack-shutdown",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

browser_anchors = {
    "test/e2e/route_registry.spec.ts": (
        "every always-on bearer route preserves browser auth and typed error boundaries",
        "high-risk action and secret mutation persist correlated redacted audit pairs",
    ),
    "test/e2e/data_pipeline.spec.ts": (
        "opportunities websocket authenticates with ticket before subscribe",
    ),
}
for path, titles in browser_anchors.items():
    source = (root / path).read_text(encoding="utf-8")
    for test_title in titles:
        escaped = re.escape(test_title)
        if re.search(rf"(?m)^\s*test\.(?:skip|fixme)\(\s*['\"]{escaped}['\"]", source):
            fail(f"browser anchor must not be skipped: {test_title}")
        if not re.search(rf"(?m)^\s*test\(\s*['\"]{escaped}['\"]", source):
            fail(f"missing non-skipping browser anchor: {test_title}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
if scripts.get("test:e2e:pr-dk") != browser_command:
    fail("dedicated PR-DK browser command drifted")
product = scripts.get("test:e2e:product", "")
for path in browser_anchors:
    if product.count(path) != 1:
        fail(f"product suite must include {path} exactly once")
release_fixture = json.loads(
    (root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8")
)
release_product = release_fixture.get("scripts", {}).get("test:e2e:product", "")
for path in browser_anchors:
    if release_product.count(path) != 1:
        fail(f"release QA product suite must include {path} exactly once")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if path == "package.json":
        continue
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-DK contract ({len(evidence_contract)} evidence types; "
    "95-route policy + 20 high-risk mutations + REST/WS auth closure)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_route_inventory.sh"
bash "$ROOT/scripts/check_route_endpoint_metadata.sh"
bash "$ROOT/scripts/check_route_runtime_policy.sh"
bash "$ROOT/scripts/check_mutation_audit_contract.sh"
bash "$ROOT/scripts/check_pr_fh_completion.sh"
bash "$ROOT/scripts/check_pr_fi_completion.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"

if [[ "${PR_DK_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" bash "$ROOT/scripts/verify_security_contract.sh"
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dk -- --workers=1
fi

printf 'OK PR-DK API security surface and high-risk audit contract\n'
