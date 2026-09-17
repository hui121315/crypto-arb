#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
BROWSER="$ROOT/test/e2e/pr_cq_operator_qa.spec.ts"
EXECUTION_ACTIONS="$ROOT/frontend/src/panels/modules/execution/data/actions.rs"
DOCKERFILE="$ROOT/deploy/Dockerfile.api"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-CQ completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  temp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-cq.XXXXXX")"
  cp "$DOC" "$temp/audit.md"
  cp "$EVIDENCE" "$temp/evidence.tsv"
  cp "$BROWSER" "$temp/browser.ts"
  cp "$EXECUTION_ACTIONS" "$temp/execution-actions.rs"
  cp "$DOCKERFILE" "$temp/Dockerfile.api"
  restore() {
    cp "$temp/audit.md" "$DOC"
    cp "$temp/evidence.tsv" "$EVIDENCE"
    cp "$temp/browser.ts" "$BROWSER"
    cp "$temp/execution-actions.rs" "$EXECUTION_ACTIONS"
    cp "$temp/Dockerfile.api" "$DOCKERFILE"
  }
  cleanup() {
    restore
    rm -rf "$temp"
  }
  trap cleanup EXIT

  assert_rejected() {
    local label="$1"
    local status
    set +e
    PR_CQ_SKIP_BROWSER_LIST=1 PR_CQ_SKIP_TESTS=1 PR_CQ_SKIP_UPSTREAM=1 \
      bash "$0" >/dev/null 2>&1
    status=$?
    set -e
    if [[ "$status" -eq 0 ]]; then
      fail "self-test accepted $label"
    fi
  }

  PR_CQ_SKIP_BROWSER_LIST=1 PR_CQ_SKIP_TESTS=1 PR_CQ_SKIP_UPSTREAM=1 \
    bash "$0" >/dev/null

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-CQ Local Runtime & Operator QA Contract` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-CQ self-test setup failed: roadmap row drifted")
path.write_text(source.replace(marker, marker.replace("✅ 完成", "🟡 部分完成"), 1), encoding="utf-8")
PY
  assert_rejected "a downgraded roadmap row"
  restore

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-CQ\tmutation-timeout\t"):
        rows.pop(index)
        path.write_text("\n".join(rows) + "\n", encoding="utf-8")
        break
else:
    raise SystemExit("PR-CQ self-test setup failed: evidence row missing")
PY
  assert_rejected "an incomplete evidence matrix"
  restore

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'test("PR-CQ credential save failure exits pending and records operator-visible evidence"'
if source.count(marker) != 1:
    raise SystemExit("PR-CQ self-test setup failed: browser marker drifted")
path.write_text(source.replace(marker, marker.replace("test(", "test.skip("), 1), encoding="utf-8")
PY
  assert_rejected "a skipped operator browser fixture"
  restore

  python3 - "$EXECUTION_ACTIONS" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '''    with_mutation_timeout(
        "对冲提交",
        client.confirm_hedge_with_context(&seed.opportunity_id, &seed.request, &request_context),
    )
    .await
'''
replacement = '''    client
        .confirm_hedge_with_context(&seed.opportunity_id, &seed.request, &request_context)
        .await
'''
if source.count(marker) != 1:
    raise SystemExit("PR-CQ self-test setup failed: execution timeout marker drifted")
path.write_text(source.replace(marker, replacement, 1), encoding="utf-8")
PY
  assert_rejected "execution submission without the bounded mutation timeout"
  restore

  python3 - "$DOCKERFILE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "http://127.0.0.1:8000/health/ready"
if source.count(marker) != 1:
    raise SystemExit("PR-CQ self-test setup failed: Docker readiness marker drifted")
path.write_text(source.replace(marker, "http://127.0.0.1:8000/health", 1), encoding="utf-8")
PY
  assert_rejected "a Docker healthcheck downgraded to liveness"
  restore

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "### 🟡 6.5 下一步执行队列"
if source.count(marker) != 1:
    raise SystemExit("PR-CQ self-test setup failed: queue heading drifted")
stale = "\n\n1. **PR-CQ Local Runtime & Operator QA Contract** — stale completed row"
path.write_text(source.replace(marker, marker + stale, 1), encoding="utf-8")
PY
  assert_rejected "completed PR-CQ returned to the queue head"

  printf 'PR-CQ completion self-test passed\n'
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
title = "PR-CQ Local Runtime & Operator QA Contract"
verify_anchor = "`bash scripts/check_pr_cq_completion.sh --self-test`"
evidence_contract = {
    "local-supervisor": ("scripts/dev_up.sh", "bash -n scripts/dev_up.sh"),
    "local-shutdown": ("scripts/dev_down.sh", "bash -n scripts/dev_down.sh"),
    "local-restart": ("scripts/dev_restart.sh", "bash -n scripts/dev_restart.sh"),
    "runtime-snapshot": ("scripts/verify_runtime.sh", "bash -n scripts/verify_runtime.sh"),
    "runtime-contract": ("scripts/verify_runtime_contracts.sh", "bash -n scripts/verify_runtime_contracts.sh"),
    "operator-runbook": ("README.md", "health/ready"),
    "env-security-contract": (".env.example", "APP_SECURITY__ALLOWED_ORIGINS"),
    "cors-env-parser-test": ("crates/common/src/config.rs", "cors_origin_env"),
    "credential-secret-contract": ("shared-types/src/venues/credentials.rs", "credential"),
    "secret-backend": ("crates/api/src/services/venue_credentials/storage.rs", "venue_credentials"),
    "docker-api-image": ("deploy/Dockerfile.api", "health/ready"),
    "docker-compose": ("deploy/docker-compose.yml", "health/ready"),
    "readiness-endpoint": ("crates/api/src/routers/health.rs", "routers::health::tests"),
    "mutation-timeout": ("frontend/src/api/rest/timeout.rs", "api::rest::timeout"),
    "settings-mutation-timeout": ("frontend/src/panels/modules/settings/data/actions/transport.rs", "settings"),
    "execution-mutation-timeout": ("frontend/src/panels/modules/execution/data/actions.rs", "execution"),
    "action-state-contract": ("shared-types/src/actions/state.rs", "action_state"),
    "operator-browser": ("test/e2e/pr_cq_operator_qa.spec.ts", "test:e2e:pr-cq"),
    "credential-failure-snapshot": ("test/e2e/pr_cq_operator_qa.spec.ts-snapshots/pr-cq-credential-save-failure.png", "test:e2e:pr-cq"),
    "execution-failure-snapshot": ("test/e2e/pr_cq_operator_qa.spec.ts-snapshots/pr-cq-execution-submit-failure.png", "test:e2e:pr-cq"),
    "product-suite": ("package.json", "test:e2e:pr-cq"),
    "ci-browser": (".github/workflows/ci.yml", "test:e2e:product"),
    "release-fixture": ("scripts/fixtures/release_qa_contract/package.json", "check_release_qa_contract.sh"),
    "release-gate": ("scripts/check_release_qa_contract.sh", "check_release_qa_contract.sh"),
    "repo-gate-wiring": ("scripts/verify_repo_gates.sh", "verify_repo_gates.sh"),
    "completion-governance": ("scripts/check_pr_cq_completion.sh", "check_pr_cq_completion.sh --self-test"),
}
browser_titles = (
    "PR-CQ credential save failure exits pending and records operator-visible evidence",
    "PR-CQ execution submit failure exits pending and remains retryable",
)
source_markers = {
    "deploy/Dockerfile.api": (
        "ca-certificates curl",
        'Authorization: Bearer ${APP_SECURITY__AUTH_TOKEN}',
        "http://127.0.0.1:8000/health/ready",
    ),
    "deploy/docker-compose.yml": (
        "APP_SECURITY__ALLOWED_ORIGINS:",
        "APP_CREDENTIALS__SECRET_BACKEND: runtime",
        "APP_STORAGE__DATA_DIR: /app/data",
        "api_data:/app/data",
        "http://127.0.0.1:8000/health/ready",
    ),
    ".env.example": (
        'APP_SECURITY__ALLOWED_ORIGINS=["http://127.0.0.1:8080"]',
        "APP_CREDENTIALS__SECRET_BACKEND=keychain",
    ),
    "README.md": (
        "CORS 环境变量使用 JSON/TOML 数组语法",
        "APP_CREDENTIALS__SECRET_BACKEND=keychain",
        "带 Bearer 的 `/health/ready`",
    ),
    "crates/common/src/config.rs": (
        "cors_origin_env_array_loads_and_satisfies_public_bind_contract",
        "config::tests::cors_origin_env_child",
    ),
    "frontend/src/api/rest/timeout.rs": (
        "MUTATION_TIMEOUT_MS: u32 = 20_000",
        '"MUTATION_TIMEOUT"',
        '"frontend.mutation_timeout"',
    ),
    "frontend/src/panels/modules/settings/data/actions/transport.rs": (
        "with_mutation_timeout(",
        "save_venue_credentials_task",
        "update_risk_config_task",
        "set_kill_switch_task",
    ),
    "frontend/src/panels/modules/execution/data/actions.rs": (
        "with_mutation_timeout(",
        "confirm_hedge_task",
        '"对冲提交"',
    ),
    ".github/workflows/ci.yml": ("browser-smoke:", "CI=1 npm run test:e2e:product"),
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-CQ completion gate failed: {message}")


def require_incomplete_queue_head(doc: str, queue: str) -> None:
    matched = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    if not matched:
        fail("local queue must retain a PR roadmap item at its head")
    head = matched.group(1)
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{head} ")), None)
    if row is None or "✅ 完成" in row:
        fail(f"local queue head must be an incomplete roadmap row: {head}")


def require_queue_integrity(doc: str, queue: str) -> None:
    items = re.findall(r"(?m)^(\d+)\.\s+\*\*(.+?)\*\*", queue)
    numbers = [int(number) for number, _ in items]
    titles = [title for _, title in items]
    if numbers != list(range(1, len(items) + 1)) or len(titles) != len(set(titles)):
        fail("queue numbering and titles must remain contiguous and unique")
    if not any("外部等待" in title for title in titles):
        fail("queue must retain an explicit external waiting pool")
    for title in titles:
        if "外部等待" in title:
            continue
        matched = re.match(r"(PR-[A-Z]+)\b", title)
        if not matched:
            fail(f"local queue item lacks a roadmap PR id: {title}")
        pr_id = matched.group(1)
        row = next((line for line in doc.splitlines() if line.startswith(f"| `{pr_id} ")), None)
        if row is None or "✅ 完成" in row:
            fail(f"local queue item must remain an incomplete roadmap row: {pr_id}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
fact_lines = history.split("## 附录 I：", 1)[0].splitlines() + doc.splitlines()
start = doc.find("### 🟡 6.3")
end = doc.find("### 🟡 6.4", start)
if start < 0 or end < 0:
    fail("bounded roadmap section is missing")
roadmap = doc[start:end]
rows = [line for line in roadmap.splitlines() if line.startswith(f"| `{title}`")]
if len(rows) != 1:
    fail("expected one PR-CQ roadmap row")
row = rows[0]
if "✅ 完成" not in row or "剩余：无" not in row or row.count(verify_anchor) != 1:
    fail("roadmap row must be complete, remaining-none and bound to the destructive gate")
audit = next((line for line in reversed(fact_lines) if line.startswith("| `AUD-16` |")), None)
if audit is None or "✅ 完成" not in audit or "剩余：无" not in audit:
    fail("AUD-16 configuration and secret lifecycle row must be complete")
for finding in (
    "一键前后端重启与 runtime snapshot",
    "README/Docker/.env 当前产品与安全语义",
    "runtime verify 与产品 browser smoke 强制进入验收",
    "E2E typed failure 与 mutation pending 退出路径",
):
    finding_row = next((line for line in reversed(fact_lines) if finding in line), None)
    if finding_row is None or not finding_row.startswith("| P0 / ✅ 完成 |"):
        fail(f"operator finding remains incomplete: {finding}")

queue = doc[doc.find("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-CQ\b", queue):
    fail("completed PR-CQ remains in the local queue")
require_incomplete_queue_head(doc, queue)
require_queue_integrity(doc, queue)

if "## 2026-07-16 PR-CQ Local Runtime and Operator QA Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [entry for entry in csv.DictReader(handle, delimiter="\t") if entry["pr_id"] == "PR-CQ"]
indexed = {entry["evidence_type"]: entry for entry in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, (artifact, command_anchor) in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or command_anchor not in evidence["command"]:
        fail(f"evidence anchor drifted: {evidence_type}")
    if not (root / artifact).is_file() or not evidence["notes"].strip():
        fail(f"evidence artifact or note is missing: {artifact}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {entry["file"]: entry for entry in csv.DictReader(handle, delimiter="\t")}
coverage_exempt = {
    "README.md",
    ".env.example",
    "deploy/Dockerfile.api",
    "deploy/docker-compose.yml",
    "test/e2e/pr_cq_operator_qa.spec.ts-snapshots/pr-cq-credential-save-failure.png",
    "test/e2e/pr_cq_operator_qa.spec.ts-snapshots/pr-cq-execution-submit-failure.png",
    "package.json",
    "scripts/fixtures/release_qa_contract/package.json",
}
for artifact, _ in evidence_contract.values():
    item = coverage.get(artifact)
    if artifact in coverage_exempt:
        continue
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

for relative, markers in source_markers.items():
    source = (root / relative).read_text(encoding="utf-8")
    for marker in markers:
        if marker not in source:
            fail(f"source marker missing: {relative}:{marker}")

browser = (root / "test/e2e/pr_cq_operator_qa.spec.ts").read_text(encoding="utf-8")
for title in browser_titles:
    escaped = re.escape(title)
    if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', browser):
        fail(f"browser anchor is skipped: {title}")
    if not re.search(rf'test\(\s*["\']{escaped}["\']', browser):
        fail(f"browser anchor is missing: {title}")

product = json.loads((root / "package.json").read_text(encoding="utf-8"))
release = json.loads((root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8"))
path = "test/e2e/pr_cq_operator_qa.spec.ts"
for package_name, package in (("product", product), ("release", release)):
    scripts = package.get("scripts", {})
    if scripts.get("test:e2e:pr-cq") != f"playwright test {path}":
        fail(f"{package_name} dedicated PR-CQ browser command drifted")
    if scripts.get("test:e2e:product", "").count(path) != 1:
        fail(f"{package_name} product suite must include PR-CQ exactly once")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo_gate.count("check_pr_cq_completion.sh") != 2:
    fail("repo gate must execute PR-CQ exactly once in docs and all scopes")

print(
    f"OK PR-CQ static contract ({len(evidence_contract)} evidence types; "
    f"{len(browser_titles)} browser anchors; authenticated Docker readiness and bounded mutations)"
)
PY

if [[ "${PR_CQ_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test "$BROWSER" --list >/dev/null
fi

if [[ "${PR_CQ_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_CN_SKIP_TESTS=1 PR_CN_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_cn_completion.sh"
  PR_DG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dg_completion.sh"
  PR_DU_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_du_completion.sh"
  PR_CK_SKIP_TESTS=1 PR_CK_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ck_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
  bash "$ROOT/scripts/check_release_qa_contract.sh"
fi

if [[ "${PR_CQ_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p common --lib cors_origin_env --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    --lib api::rest::timeout --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-cq -- --workers=1
fi

printf 'PR-CQ local runtime and operator QA completion passed\n'
