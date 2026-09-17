#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="$ROOT/scripts/check_allow_debt.sh"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/crossline-allow-debt-self-test.XXXXXX")"

cleanup() {
  rm -rf "$TMP_ROOT"
}
trap cleanup EXIT

write_baseline_source() {
  printf '%s\n' \
    '#[allow(dead_code)]' \
    'pub fn legacy() {}' \
    >"$TMP_ROOT/crates/exchange/src/legacy.rs"
}

run_gate() {
  ALLOW_DEBT_BASE_REF=origin/HEAD bash "$TMP_ROOT/scripts/check_allow_debt.sh"
}

expect_success() {
  local output

  if ! output="$(run_gate 2>&1)"; then
    echo "FAIL expected inherited allow gate to pass"
    printf '%s\n' "$output"
    exit 1
  fi
  if ! grep -Fq 'INHERITED allow: crates/exchange/src/legacy.rs:1 outer_allow dead_code' <<<"$output"; then
    echo "FAIL inherited allow gate did not report the merge-base match"
    printf '%s\n' "$output"
    exit 1
  fi
}

expect_default_baseline_fallback() {
  local base_commit output

  base_commit="$(git -C "$TMP_ROOT" rev-parse HEAD)"
  git -C "$TMP_ROOT" update-ref -d refs/remotes/origin/HEAD
  if ! output="$(env -u ALLOW_DEBT_BASE_REF bash "$TMP_ROOT/scripts/check_allow_debt.sh" 2>&1)"; then
    git -C "$TMP_ROOT" update-ref refs/remotes/origin/HEAD "$base_commit"
    echo "FAIL expected missing default remote baseline to fall back to HEAD"
    printf '%s\n' "$output"
    exit 1
  fi
  git -C "$TMP_ROOT" update-ref refs/remotes/origin/HEAD "$base_commit"
  if ! grep -Fq 'BASELINE allow debt: HEAD merge-base' <<<"$output"; then
    echo "FAIL allow gate did not report the default HEAD baseline fallback"
    printf '%s\n' "$output"
    exit 1
  fi
}

expect_failure() {
  local label="$1"
  local output

  if output="$(run_gate 2>&1)"; then
    echo "FAIL expected allow gate to reject ${label}"
    exit 1
  fi
  if ! grep -Fq 'FAIL production allow without owner/expiry' <<<"$output"; then
    echo "FAIL allow gate did not reject ${label} for missing ownership"
    printf '%s\n' "$output"
    exit 1
  fi
}

mkdir -p "$TMP_ROOT/scripts" "$TMP_ROOT/crates/exchange/src" "$TMP_ROOT/frontend/src" "$TMP_ROOT/shared-types/src"
cp "$CHECK" "$TMP_ROOT/scripts/check_allow_debt.sh"
printf '%s\n' '# path\tline\tkind\tscope\tlints\towner\tticket\texpires\treason' >"$TMP_ROOT/scripts/allow_debt_allowlist.tsv"
printf '%s\n' 'pub fn frontend_anchor() {}' >"$TMP_ROOT/frontend/src/lib.rs"
printf '%s\n' 'pub struct SharedAnchor;' >"$TMP_ROOT/shared-types/src/lib.rs"
write_baseline_source

git init --quiet "$TMP_ROOT"
git -C "$TMP_ROOT" config user.email "allow-debt-self-test@example.invalid"
git -C "$TMP_ROOT" config user.name "allow debt self test"
git -C "$TMP_ROOT" add .
git -C "$TMP_ROOT" commit --quiet -m "baseline"
git -C "$TMP_ROOT" update-ref refs/remotes/origin/HEAD "$(git -C "$TMP_ROOT" rev-parse HEAD)"

expect_success
expect_default_baseline_fallback

printf '%s\n' \
  '// source moved without explicit ownership' \
  '#[allow(dead_code)]' \
  'pub fn legacy() {}' \
  >"$TMP_ROOT/crates/exchange/src/legacy.rs"
expect_failure "moved inherited allow"

write_baseline_source
printf '%s\n' \
  '#[allow(unused)]' \
  'pub fn legacy() {}' \
  >"$TMP_ROOT/crates/exchange/src/legacy.rs"
expect_failure "changed inherited allow"

write_baseline_source
printf '%s\n' \
  '#[allow(dead_code)]' \
  'pub fn legacy() {}' \
  '' \
  '#[allow(dead_code)]' \
  'pub fn added() {}' \
  >"$TMP_ROOT/crates/exchange/src/legacy.rs"
expect_failure "new production allow"

echo "allow-debt baseline self-test passed"
