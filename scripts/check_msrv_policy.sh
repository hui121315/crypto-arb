#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLCHAIN="$ROOT/rust-toolchain.toml"
FRONTEND_MANIFEST="$ROOT/frontend/Cargo.toml"
CI="$ROOT/.github/workflows/ci.yml"
PRODUCTION_MSRV="1.86"
DEV_TEST_FRONTEND_FLOOR="1.88"
PINNED_TOOLCHAIN="1.95.0"

fail() {
  printf 'MSRV policy failed: %s\n' "$*" >&2
  exit 1
}

require_file_line() {
  local file="$1"
  local expected="$2"

  grep -Fqx "$expected" "$file" || fail "expected $file to contain: $expected"
}

job_body() {
  local job="$1"

  awk -v job="$job" '
    $0 == "  " job ":" { inside = 1; next }
    inside && /^  [[:alnum:]_-]+:$/ { exit }
    inside { print }
  ' "$CI"
}

require_job_line() {
  local job="$1"
  local expected="$2"

  job_body "$job" | grep -Fqx -- "$expected" \
    || fail "expected CI job $job to contain: $expected"
}

require_file_line "$ROOT/Cargo.toml" "rust-version = \"$PRODUCTION_MSRV\""
require_file_line "$FRONTEND_MANIFEST" "rust-version = \"$DEV_TEST_FRONTEND_FLOOR\""
require_file_line "$TOOLCHAIN" "channel = \"$PINNED_TOOLCHAIN\""
require_file_line "$TOOLCHAIN" 'components = ["rustfmt", "clippy"]'

if grep -Fq 'dtolnay/rust-toolchain@stable' "$CI"; then
  fail 'CI must not use a floating Rust stable channel'
fi

for job in fmt clippy test runtime-contracts frontend browser-smoke supply-chain build; do
  require_job_line "$job" '      - uses: dtolnay/rust-toolchain@1.95.0'
done

require_job_line msrv '      - uses: dtolnay/rust-toolchain@1.86.0'
require_job_line msrv '        run: cargo check --workspace --locked --lib --bins'
require_job_line dev-msrv '      - uses: dtolnay/rust-toolchain@1.88.0'
require_job_line dev-msrv '      - run: cargo check --workspace --locked --all-targets'
require_job_line dev-msrv '        run: cargo check --locked --target wasm32-unknown-unknown --all-targets'
require_job_line fmt '        run: cargo fmt --all -- --check'
require_job_line clippy '      - run: cargo clippy --workspace --all-targets -- -D warnings'
require_job_line test '      - run: cargo test --workspace --all-features'
require_job_line frontend '          targets: wasm32-unknown-unknown'
require_job_line frontend '        run: cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings'

printf 'OK MSRV policy: production=%s dev_test_frontend_floor=%s pinned_toolchain=%s\n' \
  "$PRODUCTION_MSRV" "$DEV_TEST_FRONTEND_FLOOR" "$PINNED_TOOLCHAIN"
