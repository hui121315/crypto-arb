#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
BROWSER_GREP='every always-on bearer route preserves browser auth and typed error boundaries|gated diagnostic routes stay absent even with local bearer credentials|high-risk action and secret mutation persist correlated redacted audit pairs|settings exposes bounded watchlist prewarm and truthful toast queue runtime|settings exposes durable watchlist storage and delivery provenance'

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

python3 - "$ROOT" "$MODE" <<'PY'
from __future__ import annotations

from collections import Counter
from pathlib import Path
import csv
import re
import shutil
import sys
import tempfile


root = Path(sys.argv[1])
mode = sys.argv[2]
PR_TITLE = "PR-AD API Surface & Legacy Feature Gate"
VERIFY_ANCHOR = "`bash scripts/check_pr_ad_completion.sh --self-test`"
HISTORY_HEADING = "## 2026-07-17 PR-AD API Surface and Legacy Feature Closure"
EVIDENCE = {
    "successor-route-surface": ("scripts/check_pr_do_completion.sh", "PR_DO_SKIP_TESTS=1"),
    "successor-route-audit": ("scripts/check_pr_ca_completion.sh", "PR_CA_SKIP_TESTS=1"),
    "successor-watchlist-alerts": ("scripts/check_pr_dp_completion.sh", "PR_DP_SKIP_TESTS=1"),
    "successor-external-payload": ("scripts/check_pr_dq_completion.sh", "PR_DQ_SKIP_TESTS=1"),
    "successor-llm-boundary": ("scripts/check_pr_fk_completion.sh", "check_pr_fk_completion.sh"),
    "successor-options-simulation": ("scripts/check_pr_fm_completion.sh", "check_pr_fm_completion.sh"),
    "route-inventory-authority": ("docs/API_ROUTE_INVENTORY.tsv", "check_route_inventory.sh"),
    "runtime-route-registry": ("crates/api/src/route_specs.rs", "route_specs::tests"),
    "runtime-registry-regressions": ("crates/api/src/route_specs/tests.rs", "route_specs::tests"),
    "runtime-route-log": ("crates/api/src/app.rs", "disabled_route_registry_summary"),
    "durable-watchlist-alert-service": ("crates/api/src/services/watchlist_alerts.rs", "watchlist"),
    "watchlist-mutation-audit": ("crates/api/src/routers/watchlist.rs", "watchlist.delete"),
    "alert-mutation-audit": ("crates/api/src/routers/alerts.rs", "alert_rule.delete"),
    "shared-alert-webhook-contract": ("shared-types/src/alerts.rs", "webhook_channel_is_fail_closed"),
    "shared-llm-payload-contract": ("shared-types/src/llm.rs", "sensitive_markers_fail_closed"),
    "llm-auth-audit-boundary": ("crates/api/src/routers/chat.rs", "llm_audit_detail_never_contains"),
    "browser-route-policy": ("test/e2e/route_registry.spec.ts", "playwright test"),
    "browser-watchlist-alerts": ("test/e2e/watchlist_alerts_runtime.spec.ts", "playwright test"),
    "completion-governance": ("scripts/check_pr_ad_completion.sh", "check_pr_ad_completion.sh --self-test"),
}
RUNNABLE = (
    ("crates/api/src/route_specs/tests.rs", "disabled_route_summary_is_unique_sorted_and_stable"),
    ("crates/api/src/route_specs/tests.rs", "enabling_compiled_shared_gate_removes_disabled_feature"),
    ("crates/api/src/route_specs/tests.rs", "requested_uncompiled_chat_surfaces_are_operator_visible"),
    ("crates/api/src/route_specs/tests.rs", "requested_uncompiled_options_surface_is_operator_visible"),
    ("crates/api/src/routers/watchlist/tests.rs", "watchlist_delete_cascades_rule_and_cooldown_atomically"),
    ("crates/api/src/routers/watchlist/tests.rs", "unavailable_durable_storage_rolls_back_watchlist_mutation"),
    ("crates/api/src/routers/alerts/tests.rs", "alert_rule_mutations_replay_without_duplicate_side_effects"),
    ("shared-types/src/alerts.rs", "webhook_channel_is_fail_closed_even_with_https_url"),
    ("shared-types/src/llm.rs", "sensitive_markers_fail_closed_before_external_serialization"),
    ("shared-types/src/llm.rs", "multibyte_summary_respects_byte_cap"),
    ("crates/api/src/routers/chat.rs", "outbound_requires_auth_and_a_live_audit_writer"),
    ("crates/api/src/routers/chat.rs", "llm_audit_detail_never_contains_payload_summary"),
    ("crates/api/src/app.rs", "deleted_simulation_routes_stay_absent_with_all_remaining_surfaces_enabled"),
)
BROWSERS = {
    "test/e2e/route_registry.spec.ts": (
        "every always-on bearer route preserves browser auth and typed error boundaries",
        "gated diagnostic routes stay absent even with local bearer credentials",
        "high-risk action and secret mutation persist correlated redacted audit pairs",
    ),
    "test/e2e/watchlist_alerts_runtime.spec.ts": (
        "settings exposes bounded watchlist prewarm and truthful toast queue runtime",
        "settings exposes durable watchlist storage and delivery provenance",
    ),
}
CLOSURE_PATHS = set(path for path, _ in EVIDENCE.values()) | {
    "crates/api/Cargo.toml",
    "crates/api/src/routers/watchlist/tests.rs",
    "crates/api/src/routers/alerts/tests.rs",
    "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
    "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
    "docs/PRODUCT_AUDIT_COVERAGE.tsv",
    "scripts/verify_repo_gates.sh",
}


def fail(message: str) -> None:
    raise ValueError(message)


def read_rows(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def require_markers(current_root: Path, relative: str, *markers: str) -> str:
    source = (current_root / relative).read_text(encoding="utf-8")
    for marker in markers:
        if marker not in source:
            fail(f"missing {relative} marker: {marker}")
    return source


def require_runnable(current_root: Path, relative: str, test_name: str) -> None:
    source = (current_root / relative).read_text(encoding="utf-8")
    pattern = re.compile(
        rf"(?ms)(?P<attrs>(?:^\s*#\[[^\n]+\]\s*\n)+)"
        rf"\s*(?:async\s+)?fn\s+{re.escape(test_name)}\s*\("
    )
    match = pattern.search(source)
    if match is None:
        fail(f"runnable test is missing: {relative}:{test_name}")
    attrs = match.group("attrs")
    if "#[test]" not in attrs and "#[tokio::test" not in attrs:
        fail(f"test attribute is missing: {relative}:{test_name}")
    if "#[ignore" in attrs or "#[should_panic" in attrs:
        fail(f"test is skipped: {relative}:{test_name}")


def require_incomplete_queue_head(doc: str, queue: str) -> None:
    matched = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    if not matched:
        fail("local queue must retain a roadmap PR at its head")
    head = matched.group(1)
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{head} ")), None)
    if row is None or "✅ 完成" in row:
        fail(f"local queue head must remain incomplete: {head}")


def check(current_root: Path) -> None:
    doc = (current_root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{PR_TITLE}`")), None)
    if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
        fail("roadmap row must be completed with no local remainder")
    if row.count(VERIFY_ANCHOR) != 1:
        fail("roadmap verification must name the destructive completion gate")

    queue = doc[doc.index("### 🟡 6.5"):]
    if re.search(r"(?m)^\d+\.\s+\*\*PR-AD\b", queue):
        fail("completed PR-AD remains in the local queue")
    require_incomplete_queue_head(doc, queue)

    successors = (
        "PR-DO API Surface Legacy Route Classification & Exposure Gate",
        "PR-CA Route Inventory & High-Risk Audit Gate",
        "PR-DP Watchlist Alert Persistence & Runtime Status Contract",
        "PR-DQ Non-P0 Client DTO Cleanup & External Payload Safety",
        "PR-FK LLM External Payload, Redaction, Feature Gate & Provider Evidence Contract",
        "PR-FM Options Simulation Surface, Unsupported Semantics & Legacy Route Contract",
    )
    for title in successors:
        successor = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
        if successor is None or "✅ 完成" not in successor or "剩余：无。" not in successor:
            fail(f"required successor authority is incomplete: {title}")

    evidence_rows = [
        entry
        for entry in read_rows(current_root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv")
        if entry["pr_id"] == "PR-AD"
    ]
    indexed = {entry["evidence_type"]: entry for entry in evidence_rows}
    if len(indexed) != len(evidence_rows) or set(indexed) != set(EVIDENCE):
        fail(f"evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for evidence_type, (artifact, command_anchor) in EVIDENCE.items():
        evidence = indexed[evidence_type]
        if evidence["artifact"] != artifact or not (current_root / artifact).is_file():
            fail(f"{evidence_type} artifact drifted or is missing")
        if command_anchor not in evidence["command"] or not evidence["notes"].strip():
            fail(f"{evidence_type} command or notes drifted")

    route_source = require_markers(
        current_root,
        "crates/api/src/route_specs.rs",
        "use std::collections::BTreeSet;",
        "collect::<BTreeSet<_>>()",
        "requested_but_uncompiled_route_features",
        "api_surface.chat:requested_but_uncompiled",
        "api_surface.llm_diagnostics:requested_but_uncompiled",
        "api_surface.options:requested_but_uncompiled",
        '#[path = "route_specs/tests.rs"]',
    )
    if route_source.count("fn requested_but_uncompiled_route_features") != 2:
        fail("compiled and all-feature unavailable-surface helpers must both exist")
    require_markers(
        current_root,
        "crates/api/src/app.rs",
        "disabled_route_feature_flags",
        "route_specs::disabled_route_registry_summary(&surface)",
        "deleted_simulation_routes_stay_absent_with_all_remaining_surfaces_enabled",
    )
    require_markers(
        current_root,
        "crates/api/src/services/watchlist_alerts.rs",
        "watchlist_alert_mutation_lock",
        "snapshot-then-commit",
        "persist_rows",
        "WATCHLIST_STORAGE_UNAVAILABLE",
    )
    require_markers(current_root, "crates/api/src/routers/watchlist.rs", '"watchlist.create"', '"watchlist.delete"')
    require_markers(current_root, "crates/api/src/routers/alerts.rs", '"alert_rule.create"', '"alert_rule.delete"')
    require_markers(
        current_root,
        "shared-types/src/llm.rs",
        "pub const MAX_LLM_EXTERNAL_PAYLOAD_BYTES: usize = 16 * 1024;",
        "deny_unknown_fields",
        "reject_sensitive_marker",
    )
    require_markers(
        current_root,
        "crates/api/src/routers/chat.rs",
        "ensure_external_readiness",
        "record_llm_durable_audit",
        '"redactedBytes"',
    )

    api_cargo = (current_root / "crates/api/Cargo.toml").read_text(encoding="utf-8")
    default_features = re.search(r"(?m)^default\s*=\s*\[([^\]]*)\]", api_cargo)
    if default_features is None or any(
        feature in default_features.group(1) for feature in ("legacy-chat", "legacy-options")
    ):
        fail("legacy chat and options surfaces must remain default-off")
    if "legacy-simulation" in api_cargo or "dep:simulation" in api_cargo:
        fail("deleted simulation API feature returned")

    inventory = read_rows(current_root / "docs/API_ROUTE_INVENTORY.tsv")
    endpoint_keys = [
        (method, entry["path"])
        for entry in inventory
        for method in entry["methods"].split(",")
    ]
    if len(endpoint_keys) != 95 or len(set(endpoint_keys)) != 95:
        fail(f"route endpoint inventory must remain unique and complete at 95, got {len(endpoint_keys)}")
    exposure = Counter(
        entry["default_exposure"]
        for entry in inventory
        for _ in entry["methods"].split(",")
    )
    if exposure != Counter({"always": 78, "default_off": 17}):
        fail(f"route exposure matrix drifted: {dict(exposure)}")
    optional = [entry for entry in inventory if entry["feature_flag"].startswith("api_surface.")]
    if len(optional) != 17 or any(entry["default_exposure"] != "default_off" for entry in optional):
        fail("all optional API surface endpoints must remain default-off")
    support = [
        entry
        for entry in inventory
        if entry["path"].startswith("/api/watchlist")
        or entry["path"].startswith("/api/alerts/")
    ]
    if len(support) != 6 or any(
        entry["feature_flag"] != "api_surface.watchlist_alerts"
        or entry["auth_policy"] != "bearer"
        or entry["shared_dto"] != "shared"
        for entry in support
    ):
        fail("watchlist and alert routes must retain the shared authenticated feature gate")
    if any(
        entry["methods"] in {"POST", "DELETE"} and entry["audit_policy"] != "required"
        for entry in support
    ):
        fail("watchlist and alert mutations must retain required audit policy")
    if any(entry["path"].startswith("/api/simulation") for entry in inventory):
        fail("deleted simulation HTTP routes returned")

    for relative, test_name in RUNNABLE:
        require_runnable(current_root, relative, test_name)
    for relative, titles in BROWSERS.items():
        source = (current_root / relative).read_text(encoding="utf-8")
        for title in titles:
            escaped = re.escape(title)
            if re.search(rf"(?m)^\s*test\.(?:skip|fixme)\(\s*['\"]{escaped}['\"]", source):
                fail(f"browser anchor is skipped: {relative}:{title}")
            if not re.search(rf"(?m)^\s*test\(\s*['\"]{escaped}['\"]", source):
                fail(f"browser anchor is missing: {relative}:{title}")

    history = (current_root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if HISTORY_HEADING not in history:
        fail("history closure appendix is missing")
    history_entry = history[history.index(HISTORY_HEADING):]
    for relative in sorted(CLOSURE_PATHS):
        if relative not in history_entry:
            fail(f"history closure is missing path: {relative}")

    coverage_rows = read_rows(current_root / "docs/PRODUCT_AUDIT_COVERAGE.tsv")
    coverage = {entry["file"]: entry for entry in coverage_rows}
    if len(coverage) != len(coverage_rows):
        fail("coverage ledger contains duplicate paths")
    for relative in sorted(CLOSURE_PATHS):
        if relative.startswith("docs/"):
            continue
        if coverage.get(relative, {}).get("coverage_status") != "exact":
            fail(f"exact coverage is missing: {relative}")

    repo_gate = (current_root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    invocation = 'PR_AD_SKIP_TESTS=1 PR_AD_SKIP_UPSTREAM=1 PR_AD_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_ad_completion.sh"'
    if repo_gate.count(invocation) != 2:
        fail("PR-AD completion gate must be wired once in docs scope and once in full scope")


def copy_fixture() -> tuple[tempfile.TemporaryDirectory[str], Path]:
    temp = tempfile.TemporaryDirectory(prefix="crossline-pr-ad-")
    fixture = Path(temp.name)
    paths = CLOSURE_PATHS | set(path for path, _ in RUNNABLE) | set(BROWSERS) | {
        "docs/API_ROUTE_INVENTORY.tsv",
        "docs/audit_history/PRODUCT_AUDIT_HISTORY.md",
    }
    for relative in sorted(paths):
        source = root / relative
        target = fixture / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
    return temp, fixture


def replace_once(path: Path, old: str, new: str) -> None:
    source = path.read_text(encoding="utf-8")
    if source.count(old) != 1:
        fail(f"self-test setup drifted for {path}: {old}")
    path.write_text(source.replace(old, new, 1), encoding="utf-8")


def replace_first(path: Path, old: str, new: str) -> None:
    source = path.read_text(encoding="utf-8")
    if old not in source:
        fail(f"self-test setup drifted for {path}: {old}")
    path.write_text(source.replace(old, new, 1), encoding="utf-8")


def expect_failure(label: str, mutate) -> None:
    temp, fixture = copy_fixture()
    try:
        mutate(fixture)
        try:
            check(fixture)
        except ValueError:
            return
        fail(f"self-test accepted regression: {label}")
    finally:
        temp.cleanup()


if mode == "--self-test":
    check(root)
    expect_failure(
        "uncompiled surface hidden",
        lambda fixture: replace_once(
            fixture / "crates/api/src/route_specs.rs",
            "api_surface.chat:requested_but_uncompiled",
            "api_surface.chat:unavailable",
        ),
    )
    expect_failure(
        "disabled registry no longer deduplicated",
        lambda fixture: replace_once(
            fixture / "crates/api/src/route_specs.rs",
            "collect::<BTreeSet<_>>()",
            "collect::<Vec<_>>()",
        ),
    )
    expect_failure(
        "watchlist route exposed by default",
        lambda fixture: replace_once(
            fixture / "docs/API_ROUTE_INVENTORY.tsv",
            "/api/watchlist\tGET\twatchlist\tproduct_support\tdefault_off\t",
            "/api/watchlist\tGET\twatchlist\tproduct_support\talways\t",
        ),
    )
    expect_failure(
        "watchlist audit removed",
        lambda fixture: replace_once(
            fixture / "crates/api/src/routers/watchlist.rs",
            '"watchlist.delete"',
            '"watchlist.remove"',
        ),
    )
    expect_failure(
        "LLM payload unbounded",
        lambda fixture: replace_once(
            fixture / "shared-types/src/llm.rs",
            "pub const MAX_LLM_EXTERNAL_PAYLOAD_BYTES: usize = 16 * 1024;",
            "pub const MAX_LLM_EXTERNAL_PAYLOAD_BYTES: usize = usize::MAX;",
        ),
    )
    expect_failure(
        "simulation route returned",
        lambda fixture: (fixture / "docs/API_ROUTE_INVENTORY.tsv").write_text(
            (fixture / "docs/API_ROUTE_INVENTORY.tsv").read_text(encoding="utf-8")
            + "/api/simulation\tPOST\tsimulation\tnon_main\tdefault_off\tmedium\tbearer\tread\tapi_surface.simulation\tshared\tnone\tapi-simulation\tdeleted route\n",
            encoding="utf-8",
        ),
    )
    expect_failure(
        "runtime test skipped",
        lambda fixture: replace_once(
            fixture / "crates/api/src/route_specs/tests.rs",
            "#[test]\nfn disabled_route_summary_is_unique_sorted_and_stable",
            "#[test]\n#[ignore]\nfn disabled_route_summary_is_unique_sorted_and_stable",
        ),
    )
    expect_failure(
        "completed row returned to queue",
        lambda fixture: replace_once(
            fixture / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
            "### 🟡 6.5 下一步执行队列\n",
            "### 🟡 6.5 下一步执行队列\n\n1. **PR-AD API Surface & Legacy Feature Gate** — stale\n",
        ),
    )
    expect_failure(
        "evidence row removed",
        lambda fixture: (fixture / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").write_text(
            "\n".join(
                line
                for line in (fixture / "docs/PRODUCT_AUDIT_EVIDENCE.tsv")
                .read_text(encoding="utf-8")
                .splitlines()
                if not line.startswith("PR-AD\tcompletion-governance\t")
            )
            + "\n",
            encoding="utf-8",
        ),
    )
    expect_failure(
        "single-scope repository wiring",
        lambda fixture: replace_first(
            fixture / "scripts/verify_repo_gates.sh",
            'PR_AD_SKIP_TESTS=1 PR_AD_SKIP_UPSTREAM=1 PR_AD_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_ad_completion.sh"',
            "true # removed PR-AD docs-scope gate",
        ),
    )
    print("OK PR-AD completion self-test (10 destructive regressions)")
else:
    check(root)
    print(
        f"OK PR-AD static contract ({len(EVIDENCE)} evidence types; "
        f"{len(RUNNABLE)} Rust anchors; {sum(map(len, BROWSERS.values()))} browser anchors)"
    )
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_route_inventory.sh"
bash "$ROOT/scripts/check_route_endpoint_metadata.sh"
bash "$ROOT/scripts/check_route_runtime_policy.sh"

if [[ "${PR_AD_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DO_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_do_completion.sh"
  PR_CA_SKIP_TESTS=1 PR_CA_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ca_completion.sh"
  PR_DP_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dp_completion.sh"
  PR_DQ_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dq_completion.sh"
fi

if [[ "${PR_AD_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api route_specs::tests --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api --all-features route_specs::tests --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types --lib alerts::tests::webhook_channel_is_fail_closed_even_with_https_url --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api watchlist --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api alert_rule_mutations_replay_without_duplicate_side_effects --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types --lib llm --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api --features legacy-chat routers::chat --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api --features legacy-options routers::options --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api --all-features deleted_simulation_routes_stay_absent_with_all_remaining_surfaces_enabled --no-fail-fast
fi

if [[ "${PR_AD_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  CI=1 npx playwright test \
    test/e2e/route_registry.spec.ts \
    test/e2e/watchlist_alerts_runtime.spec.ts \
    --grep "$BROWSER_GREP" \
    --workers=1
fi

printf 'OK PR-AD API surface and legacy feature completion gate\n'
