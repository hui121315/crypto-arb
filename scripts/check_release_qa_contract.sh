#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE_ROOT="$ROOT/scripts/fixtures/release_qa_contract"
SCRIPT_PATH="${BASH_SOURCE[0]}"

failures=0

fail() {
  printf 'FAIL release QA contract: %s\n' "$*" >&2
  failures=1
}

require_fixed() {
  local file="$1"
  local text="$2"
  local label="$3"
  if ! rg -Fq -- "$text" "$file"; then
    fail "$label missing in ${file#$ROOT/}"
  fi
}

check_contract() {
  local root="$1"
  local trunk="$root/frontend/Trunk.toml"
  local dev_up="$root/scripts/dev_up.sh"
  local package="$root/package.json"
  local playwright="$root/playwright.config.ts"

  for file in "$trunk" "$dev_up" "$package" "$playwright"; do
    [[ -f "$file" ]] || fail "required file missing: $file"
  done
  (( failures == 0 )) || return 1

  require_fixed "$trunk" "release = false" "Trunk dev default"
  require_fixed "$dev_up" "cargo build --locked -p api" "dev API build contract"
  require_fixed "$dev_up" "trunk serve --release=false" "dev frontend profile"
  require_fixed "$playwright" 'const qaProfile = process.env.CROSSLINE_E2E_PROFILE ?? "dev";' "Playwright dev default"
  require_fixed "$playwright" 'python3 -m http.server ${webPort}' "Playwright prebuilt release frontend"
  require_fixed "$playwright" 'trunk build --release=false && python3 -m http.server ${webPort}' "Playwright built dev frontend profile"
  require_fixed "$playwright" '--directory dist' "Playwright dev static artifact directory"

  if ! jq -e '
    .scripts["test:e2e:portfolio-first-frame"] == "playwright test test/e2e/data_pipeline.spec.ts -g \"portfolio first frame stays within budget after module switch\" --workers=1"
    and .scripts["test:e2e:pr-ay"] == "playwright test test/e2e/pr_ay_partial_failure.spec.ts"
    and .scripts["test:e2e:pr-az"] == "playwright test test/e2e/pr_ay_partial_failure.spec.ts"
    and .scripts["test:e2e:pr-bb"] == "playwright test test/e2e/pr_bb_settings_runtime.spec.ts"
    and .scripts["test:e2e:pr-bc"] == "playwright test test/e2e/pr_bc_review_closure.spec.ts"
    and .scripts["test:e2e:pr-bd"] == "playwright test test/e2e/pr_bd_action_scope.spec.ts"
    and .scripts["test:e2e:product"] == "npm run test:e2e:portfolio-first-frame && playwright test test/e2e/data_pipeline.spec.ts test/e2e/pr_bk_observability.spec.ts test/e2e/route_registry.spec.ts test/e2e/pr_bx_runtime.spec.ts test/e2e/pr_co_visual.spec.ts test/e2e/pr_ck_action_evidence.spec.ts test/e2e/pr_df_execution_action.spec.ts test/e2e/pr_dg_settings_credentials.spec.ts test/e2e/pr_dh_review_ledger.spec.ts test/e2e/pr_dz_portfolio_truth.spec.ts test/e2e/pr_cd_portfolio_evidence.spec.ts test/e2e/pr_ea_execution_finality.spec.ts test/e2e/pr_eb_instrument_sizing.spec.ts test/e2e/instrument-coverage.spec.ts test/e2e/pr_ed_account_state.spec.ts test/e2e/pr_ee_profitability_evidence.spec.ts test/e2e/pr_eg_runtime_health.spec.ts test/e2e/pr_ar_private_ws_health.spec.ts test/e2e/pr_as_credential_health.spec.ts test/e2e/pr_bb_settings_runtime.spec.ts test/e2e/pr_bc_review_closure.spec.ts test/e2e/pr_bd_action_scope.spec.ts test/e2e/pr_au_transport_health.spec.ts test/e2e/pr_av_problem_recovery.spec.ts test/e2e/pr_dj_runtime_gate.spec.ts test/e2e/pr_dl_transport_runtime.spec.ts test/e2e/pr_ds_ws_runtime.spec.ts test/e2e/pr_dt_request_correlation.spec.ts test/e2e/pr_du_settings_control_plane.spec.ts test/e2e/pr_dv_scoped_preflight.spec.ts test/e2e/pr_dw_review_runtime.spec.ts test/e2e/pr_el_binance_identity.spec.ts test/e2e/pr_em_bybit_account.spec.ts test/e2e/pr_fc_account_evidence.spec.ts test/e2e/pr_eo_kucoin_pro_ws.spec.ts test/e2e/pr_ep_htx_ws_gate.spec.ts test/e2e/pr_er_gate_runtime.spec.ts test/e2e/pr_es_venue_capability_matrix.spec.ts test/e2e/pr_eu_runtime_snapshot.spec.ts test/e2e/pr_ev_exchange_problem.spec.ts test/e2e/pr_an_http_execution_evidence.spec.ts test/e2e/pr_ap_account_margin_evidence.spec.ts test/e2e/pr_ew_resource_envelope.spec.ts test/e2e/pr_fa_runtime.spec.ts test/e2e/pr_et_runtime.spec.ts test/e2e/pr_af_opportunity_semantics.spec.ts test/e2e/pr_ft_load_state.spec.ts test/e2e/pr_cn_workspace_runtime.spec.ts test/e2e/pr_cq_operator_qa.spec.ts test/e2e/pr_ay_partial_failure.spec.ts --grep-invert \"portfolio first frame stays within budget after module switch\" --workers=1"
    and .scripts["verify:dev"] == "bash scripts/check_release_qa_contract.sh && CROSSLINE_E2E_PROFILE=dev CI=1 npm run test:e2e:product"
    and .scripts["verify:release"] == "bash scripts/check_release_qa_contract.sh --release"
  ' "$package" >/dev/null; then
    fail "package product/verify commands drifted"
  fi

  (( failures == 0 ))
}

check_release_runner_contract() {
  require_fixed "$SCRIPT_PATH" "cargo build --locked --release -p api" "locked release API build"
  require_fixed "$SCRIPT_PATH" 'rm -rf "$ROOT/frontend/dist"' "fresh release frontend artifact"
  require_fixed "$SCRIPT_PATH" "trunk build --locked=true --release=true" "locked release frontend build"
  require_fixed "$SCRIPT_PATH" "WASM_REQUIRE_OZ=1 bash" "required wasm optimizer gate"
  require_fixed "$SCRIPT_PATH" '"$ROOT/target/release/crypto-arb-api"' "release API runtime binary"
  require_fixed "$SCRIPT_PATH" 'process = subprocess.Popen(' "managed release API process"
  require_fixed "$SCRIPT_PATH" '"RUST_LOG": "info"' "deterministic release API logging"
  require_fixed "$SCRIPT_PATH" 'RELEASE_QA_API_URL' "external release API override"
  require_fixed "$SCRIPT_PATH" '"verify_runtime_contracts.sh"' "release runtime gate"
  require_fixed "$SCRIPT_PATH" '"test:e2e:product"' "release browser gate"
}

find_port() {
  python3 - <<'PY'
import socket
with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}

run_release_runtime_and_browser() {
  local runtime_dir="$1"
  local api_port="$2"
  python3 - "$ROOT" "$runtime_dir" "$api_port" <<'PY'
import os
import subprocess
import sys
import time

root, runtime_dir, api_port = sys.argv[1:]
external_api_url = os.environ.get("RELEASE_QA_API_URL", "").strip().rstrip("/")
api_url = external_api_url or f"http://127.0.0.1:{api_port}"
api_log_path = os.path.join(runtime_dir, "api.log")
data_dir = os.path.join(runtime_dir, "data")
os.makedirs(data_dir, exist_ok=True)
api_env = os.environ.copy()
api_env.update({
    "APP_HOST": "127.0.0.1",
    "APP_PORT": api_port,
    "APP_STORAGE__DATA_DIR": data_dir,
    "RUST_LOG": "info",
})

process = None
if not external_api_url:
    with open(api_log_path, "ab", buffering=0) as api_log:
        process = subprocess.Popen(
            [os.path.join(root, "target/release/crypto-arb-api")],
            cwd=root,
            env=api_env,
            stdout=api_log,
            stderr=subprocess.STDOUT,
        )
try:
    for _ in range(90):
        if process is not None and process.poll() is not None:
            raise RuntimeError("release API exited before health check")
        health = subprocess.run(
            ["curl", "-fsS", "-m", "2", f"{api_url}/health"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        if health.returncode == 0:
            break
        time.sleep(1)
    else:
        raise RuntimeError("release API did not become healthy")

    print("==> Release runtime contracts", flush=True)
    runtime_env = os.environ.copy()
    runtime_env["API_URL"] = api_url
    subprocess.run(
        ["bash", os.path.join(root, "scripts", "verify_runtime_contracts.sh")],
        cwd=root,
        env=runtime_env,
        check=True,
    )

    print("==> Release browser gate", flush=True)
    browser_env = os.environ.copy()
    browser_env.update({"CROSSLINE_E2E_PROFILE": "release", "CI": "1"})
    subprocess.run(
        ["npm", "run", "test:e2e:product"],
        cwd=root,
        env=browser_env,
        check=True,
    )
except Exception:
    if process is not None and os.path.exists(api_log_path):
        with open(api_log_path, "r", encoding="utf-8", errors="replace") as api_log:
            sys.stderr.write("".join(api_log.readlines()[-120:]))
    raise
finally:
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
PY
}

run_release_qa() (
  local runtime_dir api_port
  runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/crossline-release-qa.XXXXXX")"
  api_port="$(find_port)"

  cleanup() {
    rm -rf "$runtime_dir"
  }
  trap cleanup EXIT

  printf '==> Locked release API build\n'
  cargo build --locked --release -p api

  printf '==> Locked release frontend build\n'
  rm -rf "$ROOT/frontend/dist"
  (
    cd "$ROOT/frontend"
    env -u NO_COLOR trunk build --locked=true --release=true
  )

  printf '==> Release binary budget\n'
  bash "$ROOT/scripts/check_api_binary_budget.sh"

  printf '==> Release Wasm budget (wasm-opt required)\n'
  WASM_REQUIRE_OZ=1 bash "$ROOT/scripts/check_wasm_budget.sh"

  run_release_runtime_and_browser "$runtime_dir" "$api_port"

)

case "${1:-}" in
  "")
    check_contract "$ROOT"
    check_release_runner_contract
    (( failures == 0 )) || exit 1
    printf 'OK release QA contract\n'
    ;;
  --self-test)
    check_contract "$FIXTURE_ROOT"
    check_release_runner_contract
    if (failures=0; check_contract "$FIXTURE_ROOT/bad" >/dev/null 2>&1); then
      fail "negative fixture unexpectedly passed"
    fi
    (( failures == 0 )) || exit 1
    printf 'OK release QA contract self-test\n'
    ;;
  --release)
    check_contract "$ROOT"
    check_release_runner_contract
    (( failures == 0 )) || exit 1
    run_release_qa
    printf 'OK release QA verified\n'
    ;;
  *)
    printf 'usage: %s [--self-test|--release]\n' "$0" >&2
    exit 2
    ;;
esac
