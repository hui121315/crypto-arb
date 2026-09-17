#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
BROWSER_GREP='every always-on bearer route preserves browser auth and typed error boundaries|high-risk action and secret mutation persist correlated redacted audit pairs|opportunities websocket authenticates with ticket before subscribe|PR-CQ credential save failure exits pending and records operator-visible evidence'

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

python3 - "$ROOT" "$MODE" <<'PY'
from pathlib import Path
import csv
import re
import shutil
import sys
import tempfile

root = Path(sys.argv[1])
mode = sys.argv[2]

PR_TITLE = "PR-AC API Security & Secret Governance"
VERIFY_ANCHOR = "`bash scripts/check_pr_ac_completion.sh --self-test`"
HISTORY_HEADING = "## 2026-07-17 PR-AC API Security and Secret Governance Closure"

EVIDENCE = {
    "successor-pr-dk": ("scripts/check_pr_dk_completion.sh", "check_pr_dk_completion.sh --self-test"),
    "successor-pr-ca": ("scripts/check_pr_ca_completion.sh", "check_pr_ca_completion.sh --self-test"),
    "successor-pr-fi": ("scripts/check_pr_fi_completion.sh", "check_pr_fi_completion.sh --self-test"),
    "successor-pr-fj": ("scripts/check_pr_fj_completion.sh", "check_pr_fj_completion.sh --self-test"),
    "successor-pr-dg": ("scripts/check_pr_dg_completion.sh", "check_pr_dg_completion.sh --self-test"),
    "successor-pr-cq": ("scripts/check_pr_cq_completion.sh", "check_pr_cq_completion.sh --self-test"),
    "credential-maintenance-authority": ("scripts/check_pr_q_completion.sh", "check_pr_q_completion.sh --self-test"),
    "bind-security": ("crates/common/src/config.rs", "non_loopback_with_auth_and_cors_requires_audit_log"),
    "request-id-normalization": ("crates/common/src/request_id.rs", "sanitize_rejects_empty_unsafe_or_overlong_request_id"),
    "request-extension": ("crates/api/src/middleware/trace.rs", "request_extension_carries_normalized_request_id"),
    "verified-actor-fingerprint": ("crates/api/src/middleware/audit.rs", "extract_actor_uses_verified_bearer_token_fingerprint"),
    "durable-audit-writer": ("crates/api/src/middleware/audit/writer.rs", "durable_ack_syncs_each_action_event_and_shutdown_drains_before_exit"),
    "durable-audit-replay": ("crates/api/src/middleware/audit/replay.rs", "replay_turns_interrupted_action_run_into_fail_closed_terminal"),
    "mutation-audit-matrix": ("scripts/check_mutation_audit_contract.sh", "check_mutation_audit_contract.sh"),
    "security-runtime-smoke": ("scripts/verify_security_contract.sh", "verify_security_contract.sh"),
    "atomic-dotenv": ("crates/api/src/services/venue_credentials/dotenv.rs", "atomic_dotenv_write_replaces_file_and_removes_temp_artifact"),
    "keychain-secret-backend": ("crates/api/src/services/venue_credentials/keychain.rs", "keychain_backend_reports_encrypted_and_reads_saved_secret"),
    "secret-clear-migrate": ("crates/api/src/services/venue_credentials/maintenance.rs", "clear_credentials_replays_the_same_action_run"),
    "keychain-fallback-cleanup": ("crates/api/src/services/venue_credentials/storage/maintenance.rs", "keychain_clear_removes_dotenv_fallback_before_restart"),
    "keychain-fallback-regressions": ("crates/api/src/services/venue_credentials/storage/tests.rs", "keychain_migration_removes_plaintext_dotenv_source"),
    "browser-rest-security": ("test/e2e/route_registry.spec.ts", "high-risk action and secret mutation persist correlated redacted audit pairs"),
    "browser-ws-auth": ("test/e2e/data_pipeline.spec.ts", "opportunities websocket authenticates with ticket before subscribe"),
    "browser-credential-failure": ("test/e2e/pr_cq_operator_qa.spec.ts", "PR-CQ credential save failure exits pending and records operator-visible evidence"),
    "completion-governance": ("scripts/check_pr_ac_completion.sh", "check_pr_ac_completion.sh --self-test"),
}

RUNNABLE = (
    ("crates/common/src/config.rs", "non_loopback_with_auth_and_cors_requires_audit_log"),
    ("crates/common/src/config.rs", "configured_actor_label_must_be_safe_and_static"),
    ("crates/common/src/request_id.rs", "sanitize_accepts_short_safe_ascii_request_id"),
    ("crates/common/src/request_id.rs", "sanitize_rejects_empty_unsafe_or_overlong_request_id"),
    ("crates/api/src/middleware/trace.rs", "preserves_safe_client_request_id"),
    ("crates/api/src/middleware/trace.rs", "replaces_unsafe_client_request_id"),
    ("crates/api/src/middleware/trace.rs", "request_extension_carries_normalized_request_id"),
    ("crates/api/src/middleware/audit.rs", "durable_ack_syncs_each_action_event_and_shutdown_drains_before_exit"),
    ("crates/api/src/middleware/audit.rs", "extract_actor_uses_verified_bearer_token_fingerprint"),
    ("crates/api/src/middleware/audit.rs", "verified_actor_distinguishes_bearer_tokens_without_exposing_secret"),
    ("crates/api/src/state.rs", "app_state_recovers_interrupted_action_run_from_durable_audit_snapshot"),
    ("crates/api/src/services/venue_credentials/tests.rs", "atomic_dotenv_write_replaces_file_and_removes_temp_artifact"),
    ("crates/api/src/services/venue_credentials/storage/tests.rs", "keychain_clear_removes_dotenv_fallback_before_restart"),
    ("crates/api/src/services/venue_credentials/storage/tests.rs", "keychain_migration_removes_plaintext_dotenv_source"),
    ("crates/api/src/services/venue_credentials/storage/tests.rs", "keychain_clear_masks_secret_when_dotenv_cleanup_fails"),
)

BROWSERS = {
    "test/e2e/route_registry.spec.ts": (
        "every always-on bearer route preserves browser auth and typed error boundaries",
        "high-risk action and secret mutation persist correlated redacted audit pairs",
    ),
    "test/e2e/data_pipeline.spec.ts": (
        "opportunities websocket authenticates with ticket before subscribe",
    ),
    "test/e2e/pr_cq_operator_qa.spec.ts": (
        "PR-CQ credential save failure exits pending and records operator-visible evidence",
    ),
}


def fail(message: str) -> None:
    raise ValueError(message)


def read_rows(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def require_markers(relative: str, *markers: str) -> str:
    source = (root / relative).read_text(encoding="utf-8")
    for marker in markers:
        if marker not in source:
            fail(f"missing {relative} marker: {marker}")
    return source


def require_runnable(relative: str, test_name: str) -> None:
    source = (root / relative).read_text(encoding="utf-8")
    pattern = re.compile(
        rf"(?ms)(?P<attrs>(?:^\s*#\[[^\n]+\]\s*\n)+)\s*(?:async\s+)?fn\s+{re.escape(test_name)}\s*\("
    )
    match = pattern.search(source)
    if match is None:
        fail(f"runnable test is missing: {relative}:{test_name}")
    attrs = match.group("attrs")
    if "#[test]" not in attrs and "#[tokio::test" not in attrs:
        fail(f"test attribute is missing: {relative}:{test_name}")
    if re.search(r"#\[(?:ignore|should_panic)|#\[cfg(?:_attr)?\(", attrs):
        fail(f"runnable test is skipped or cfg-disabled: {relative}:{test_name}")


def require_incomplete_queue_head(doc: str, queue: str) -> None:
    matched = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    if not matched:
        fail("local queue must retain a PR roadmap item at its head")
    head = matched.group(1)
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{head} ")), None)
    if row is None or "✅ 完成" in row:
        fail(f"local queue head must be an incomplete roadmap row: {head}")


def check(current_root: Path) -> None:
    global root
    root = current_root
    doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{PR_TITLE}`")), None)
    if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
        fail("roadmap row must be completed with no local remainder")
    if row.count(VERIFY_ANCHOR) != 1:
        fail("roadmap verification must name the destructive completion gate")

    queue = doc[doc.index("### 🟡 6.5"):]
    if re.search(r"(?m)^\d+\.\s+\*\*PR-AC\b", queue):
        fail("completed PR-AC remains in the local queue")
    require_incomplete_queue_head(doc, queue)

    successor_titles = (
        "PR-Q Config & Credential Safety",
        "PR-CA Route Inventory & High-Risk Audit Gate",
        "PR-CQ Local Runtime & Operator QA Contract",
        "PR-DG Settings Credential Runtime Diagnostics & Secret Persistence Contract",
        "PR-DK API Security Surface & High-Risk Audit Contract",
        "PR-FI API Security, Auth/CORS, WS Auth & High-Risk Audit Contract",
        "PR-FJ Security Verification, CI Gate & Runtime Smoke Contract",
    )
    for title in successor_titles:
        successor = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
        if successor is None or "✅ 完成" not in successor:
            fail(f"required authority is not complete: {title}")

    history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if HISTORY_HEADING not in history:
        fail("history closure appendix is missing")
    closure_paths = set(EVIDENCE[entry][0] for entry in EVIDENCE)
    closure_paths.update(relative for relative, _ in RUNNABLE)
    closure_paths.update(BROWSERS)
    closure_paths.update((
        "crates/api/src/services/venue_credentials/storage.rs",
        "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
        "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        "scripts/verify_repo_gates.sh",
    ))
    history_entry = history[history.index(HISTORY_HEADING):]
    for relative in closure_paths:
        if relative not in history_entry:
            fail(f"history closure is missing path: {relative}")

    rows = [entry for entry in read_rows(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv") if entry["pr_id"] == "PR-AC"]
    indexed = {entry["evidence_type"]: entry for entry in rows}
    if len(indexed) != len(rows) or set(indexed) != set(EVIDENCE):
        fail(f"evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for evidence_type, (artifact, command_anchor) in EVIDENCE.items():
        evidence = indexed[evidence_type]
        if evidence["artifact"] != artifact or not (root / artifact).is_file():
            fail(f"{evidence_type} artifact drifted or is missing")
        if command_anchor not in evidence["command"] or not evidence["notes"].strip():
            fail(f"{evidence_type} command or notes drifted")

    require_markers(
        "crates/common/src/config.rs",
        "pub fn ensure_bind_security(&self) -> AppResult<()>",
        "configure APP_SECURITY__ALLOWED_ORIGINS with explicit origins (no `*`)",
        "set APP_SECURITY__AUDIT_LOG_PATH",
        "verified_actor_label",
    )
    require_markers(
        "crates/common/src/request_id.rs",
        "pub const MAX_REQUEST_ID_LEN: usize = 64;",
        "value.len() > MAX_REQUEST_ID_LEN",
        "byte.is_ascii_alphanumeric()",
    )
    require_markers(
        "crates/api/src/middleware/trace.rs",
        "common::request_id::normalize",
        "request.extensions_mut().insert(request_id);",
    )
    require_markers(
        "crates/api/src/middleware/audit.rs",
        "pub(crate) fn record_durable",
        "insert_verified_bearer_actor",
        "fn verified_bearer_actor(",
        "x-forwarded-for",
    )
    require_markers(
        "crates/api/src/middleware/audit/writer.rs",
        "file.sync_data()",
        "WriterCommand::Shutdown",
        "DURABLE_ACK_TIMEOUT",
    )
    require_markers(
        "crates/api/src/middleware/audit/replay.rs",
        "ACTION_RUN_REPLAY_UNAVAILABLE",
        "restart_in_flight",
    )
    require_markers(
        "crates/api/src/services/venue_credentials/storage.rs",
        "environment_fallback_present",
        "maintenance::clear_fields(fields).await",
        "maintenance::migrate_fields(fields).await",
    )
    storage = require_markers(
        "crates/api/src/services/venue_credentials/storage/maintenance.rs",
        "remove_dotenv_fields",
        "mask_cleared",
        "migrate_fields_to_backend",
    )
    clear_pattern = re.compile(
        r"SecretBackend::Keychain\s*=>\s*\{.*?keychain::remove_fields\(fields\)\?;.*?"
        r"mask_cleared\(fields\);.*?remove_dotenv_fields\(fields, dotenv_path\)\.await",
        re.S,
    )
    if clear_pattern.search(storage) is None:
        fail("Keychain clear must mask runtime state and remove dotenv fallback")
    migrate_pattern = re.compile(
        r"if backend == SecretBackend::Keychain\s*\{.*?"
        r"remove_dotenv_fields\(&migrated_keys, dotenv_path\)\.await",
        re.S,
    )
    if migrate_pattern.search(storage) is None:
        fail("Keychain migration must remove the plaintext dotenv source")
    require_markers(
        "crates/api/src/services/venue_credentials/maintenance.rs",
        "storage::environment_fallback_present",
        "storage::clear_fields(&env_keys).await?",
        "storage::migrate_fields(&env_keys).await?",
    )
    require_markers(
        "crates/api/src/services/venue_credentials/dotenv.rs",
        "write_dotenv_temp_then_rename",
        "file.sync_all()",
        "std::fs::rename(temp_path, target_path)",
    )
    require_markers(
        "crates/api/src/services/venue_credentials/keychain.rs",
        "set_generic_password",
        "delete_generic_password",
    )
    require_markers(
        "scripts/check_mutation_audit_contract.sh",
        "inventory_high_risk_count",
        "venue_credentials.migrate",
    )
    require_markers(
        "scripts/verify_security_contract.sh",
        "public_bind_wildcard_cors_gate",
        "api_security_runtime_smoke",
    )

    for relative, test_name in RUNNABLE:
        require_runnable(relative, test_name)
    for relative, titles in BROWSERS.items():
        source = (root / relative).read_text(encoding="utf-8")
        for title in titles:
            escaped = re.escape(title)
            if re.search(rf"test\.(?:skip|fixme)\(\s*['\"]{escaped}['\"]", source):
                fail(f"browser proof is skipped: {title}")
            if not re.search(rf"test\(\s*['\"]{escaped}['\"]", source):
                fail(f"browser proof is missing: {title}")

    repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    if repo_gate.count("check_pr_ac_completion.sh") != 2:
        fail("repo gate must execute PR-AC in docs and full scopes")

    coverage = {entry["file"]: entry for entry in read_rows(root / "docs/PRODUCT_AUDIT_COVERAGE.tsv")}
    for relative in closure_paths:
        if relative.startswith("docs/"):
            continue
        if coverage.get(relative, {}).get("coverage_status") != "exact":
            fail(f"exact coverage missing for {relative}")


def assert_rejected(relative: str, transform, label: str) -> None:
    path = root / relative
    baseline = path.read_text(encoding="utf-8")
    changed = transform(baseline)
    if changed == baseline:
        fail(f"self-test setup drifted: {label}")
    path.write_text(changed, encoding="utf-8")
    try:
        check(root)
    except ValueError:
        pass
    else:
        fail(f"self-test accepted {label}")
    finally:
        path.write_text(baseline, encoding="utf-8")


def self_test(source_root: Path) -> None:
    paths = {
        "crates/api/src/services/venue_credentials/storage.rs",
        "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
        "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        "docs/audit_history/PRODUCT_AUDIT_HISTORY.md",
        "scripts/verify_repo_gates.sh",
    }
    paths.update(artifact for artifact, _ in EVIDENCE.values())
    paths.update(relative for relative, _ in RUNNABLE)
    paths.update(BROWSERS)
    with tempfile.TemporaryDirectory(prefix="crossline-pr-ac-completion-") as temp:
        test_root = Path(temp) / "repo"
        for relative in paths:
            target = test_root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_root / relative, target)
        check(test_root)
        assert_rejected("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace(f"| `{PR_TITLE}` | ✅ 完成 |", f"| `{PR_TITLE}` | 🟡 部分完成 |", 1), "a downgraded roadmap row")
        assert_rejected("docs/PRODUCT_AUDIT_EVIDENCE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("PR-AC\tkeychain-fallback-regressions\t")) + "\n", "an incomplete evidence matrix")
        assert_rejected("crates/api/src/services/venue_credentials/storage/maintenance.rs", lambda text: text.replace("remove_dotenv_fields(&migrated_keys, dotenv_path).await", "Ok::<(), CredentialUpdateError>(())", 1), "migration retaining plaintext dotenv")
        assert_rejected("crates/api/src/services/venue_credentials/storage/maintenance.rs", lambda text: text.replace("keychain::remove_fields(fields)?;\n            mask_cleared(fields);\n            if let Err(error) = remove_dotenv_fields(fields, dotenv_path).await", "keychain::remove_fields(fields)?;\n            mask_cleared(fields);\n            if let Err(error) = Ok::<(), CredentialUpdateError>(())", 1), "clear retaining dotenv fallback")
        assert_rejected("crates/api/src/services/venue_credentials/storage/tests.rs", lambda text: text.replace("#[tokio::test]\nasync fn keychain_clear_removes_dotenv_fallback_before_restart", "#[tokio::test]\n#[ignore]\nasync fn keychain_clear_removes_dotenv_fallback_before_restart", 1), "a skipped fallback regression")
        assert_rejected("crates/common/src/request_id.rs", lambda text: text.replace("value.len() > MAX_REQUEST_ID_LEN", "false", 1), "unbounded request ids")
        assert_rejected("crates/api/src/middleware/audit.rs", lambda text: text.replace("fn verified_bearer_actor", "fn removed_verified_bearer_actor", 1), "detached verified actor derivation")
        assert_rejected("crates/api/src/middleware/audit/writer.rs", lambda text: text.replace("file.sync_data()", "file.sync_all()", 1), "detached durable audit sync contract")
        browser, titles = next(iter(BROWSERS.items()))
        assert_rejected(browser, lambda text: text.replace(f'test("{titles[0]}"', f'test.skip("{titles[0]}"', 1), "a skipped browser proof")
        assert_rejected("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-AC stale**", 1), "completed PR-AC returned to the queue")
        assert_rejected("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("1. **PR-AD", "1. **PR-AC", 1), "a completed queue head")
        assert_rejected("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("1. **PR-AD", "1. **PR-ZZ", 1), "an untracked queue head")
        assert_rejected("docs/PRODUCT_AUDIT_COVERAGE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("scripts/check_pr_ac_completion.sh\t")) + "\n", "missing exact coverage")
        assert_rejected("scripts/verify_repo_gates.sh", lambda text: text.replace("check_pr_ac_completion.sh", "removed_pr_ac_completion.sh", 1), "single-scope repo wiring")
    print("PR-AC completion destructive self-test passed")


try:
    if mode == "--self-test":
        self_test(root)
    else:
        check(root)
        print(f"OK PR-AC static contract ({len(EVIDENCE)} evidence types; {len(RUNNABLE)} runnable anchors; 4 browser anchors)")
except (OSError, KeyError, ValueError) as exc:
    raise SystemExit(f"PR-AC completion gate failed: {exc}") from exc
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

if [[ "${PR_AC_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test \
    "$ROOT/test/e2e/route_registry.spec.ts" \
    "$ROOT/test/e2e/data_pipeline.spec.ts" \
    "$ROOT/test/e2e/pr_cq_operator_qa.spec.ts" \
    --list >/dev/null
fi

if [[ "${PR_AC_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_pr_q_completion.sh"
  PR_DG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dg_completion.sh"
  PR_DK_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dk_completion.sh"
  PR_CA_SKIP_TESTS=1 PR_CA_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ca_completion.sh"
  PR_CQ_SKIP_TESTS=1 PR_CQ_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_cq_completion.sh"
  bash "$ROOT/scripts/check_pr_fi_completion.sh"
  bash "$ROOT/scripts/check_pr_fj_completion.sh"
  bash "$ROOT/scripts/check_mutation_audit_contract.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_AC_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p common --lib request_id --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p common --lib configured_actor_label_must_be_safe_and_static --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api middleware::trace::tests --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api middleware::audit::tests --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api services::venue_credentials --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" bash "$ROOT/scripts/verify_security_contract.sh"
  CI=1 npx playwright test \
    "$ROOT/test/e2e/route_registry.spec.ts" \
    "$ROOT/test/e2e/data_pipeline.spec.ts" \
    "$ROOT/test/e2e/pr_cq_operator_qa.spec.ts" \
    --grep "$BROWSER_GREP" --workers=1
fi

printf 'PR-AC API security and secret governance completion passed\n'
