#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="${TMPDIR:-/tmp}/crossline-unused-dependencies.$$"
trap 'rm -rf "$TMP_DIR"' EXIT
mkdir -p "$TMP_DIR"

METADATA="$TMP_DIR/metadata.json"
DIRECT="$TMP_DIR/direct.tsv"
WORKSPACE="$TMP_DIR/workspace.tsv"
FAILED=0

fail() {
  printf 'FAIL %s\n' "$*" >&2
  FAILED=1
}

cargo metadata --locked --no-deps --format-version 1 >"$METADATA"
jq -r '
  .packages[] |
  .manifest_path as $manifest |
  .name as $package |
  .dependencies[] |
  select(.kind != "build") |
  [$package, $manifest, (.rename // .name), (.kind // "normal")] | @tsv
' "$METADATA" | sort -u >"$DIRECT"

while IFS=$'\t' read -r package manifest dependency kind; do
  crate_dir="$(dirname "$manifest")"
  identifier="${dependency//-/_}"
  pattern="(^|[^[:alnum:]_])${identifier}(::|!|[[:space:]]*\\{|[[:space:]]*$)|#\\[${identifier}([^[:alnum:]_]|$)"
  paths=()

  for path in "$crate_dir/src" "$crate_dir/tests" "$crate_dir/benches" "$crate_dir/build.rs"; do
    [ -e "$path" ] && paths+=("$path")
  done

  if [ "${#paths[@]}" -eq 0 ] || ! rg -q --glob '*.rs' "$pattern" "${paths[@]}"; then
    fail "unreferenced direct dependency: package=$package dependency=$dependency kind=$kind"
  fi
done <"$DIRECT"

awk '
  /^\[workspace.dependencies\]/{ in_section = 1; next }
  /^\[/{ in_section = 0 }
  in_section && /^[[:space:]]*[A-Za-z0-9_-]+[[:space:]]*=/ {
    name = $0
    sub(/=.*/, "", name)
    gsub(/[[:space:]]/, "", name)
    print name
  }
' "$ROOT/Cargo.toml" | sort -u >"$WORKSPACE"

while IFS= read -r dependency; do
  if ! rg -q --glob Cargo.toml "^${dependency}[[:space:]]*=[[:space:]]*\\{[[:space:]]*workspace[[:space:]]*=[[:space:]]*true" \
    "$ROOT/crates" "$ROOT/shared-types"; then
    fail "stale workspace dependency declaration: $dependency"
  fi
done <"$WORKSPACE"

if [ "$FAILED" -ne 0 ]; then
  exit 1
fi

printf 'OK unused direct dependency gate (%s direct declarations)\n' "$(wc -l <"$DIRECT" | tr -dc '0-9')"
