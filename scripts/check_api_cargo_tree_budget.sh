#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUDGET="$ROOT/scripts/api_cargo_tree_budget.tsv"
TMP_DIR="${TMPDIR:-/tmp}/crossline-api-tree-budget.$$"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$TMP_DIR"

TODAY="${API_TREE_BUDGET_TODAY:-$(date +%F)}"
DIRECT_TREE="$TMP_DIR/direct-tree.txt"
DUPLICATE_NAMES="$TMP_DIR/duplicate-names.txt"
BUDGET_DUPLICATE_NAMES="$TMP_DIR/budget-duplicate-names.txt"
BAD="$TMP_DIR/bad.txt"
: >"$BAD"

validate_budget_file() {
  awk -F '\t' -v today="$TODAY" '
    BEGIN { ok = 1 }
    /^#/ || NF == 0 { next }
    NF < 7 {
      printf "FAIL malformed api cargo tree budget line %d: expected 7 tab-separated fields\n", NR
      ok = 0
      next
    }
    $1 !~ /^(max_direct_normal_deps|max_duplicate_package_names|duplicate_package_name|forbidden_default_crate)$/ {
      printf "FAIL invalid api cargo tree budget kind line %d: %s\n", NR, $1
      ok = 0
    }
    $2 !~ /^[A-Za-z0-9_-]+$/ {
      printf "FAIL invalid api cargo tree budget target line %d: %s\n", NR, $2
      ok = 0
    }
    $3 !~ /^[0-9]+$/ {
      printf "FAIL invalid api cargo tree budget cap line %d: %s\n", NR, $3
      ok = 0
    }
    $4 == "" || $5 == "" || $6 == "" || $7 == "" {
      printf "FAIL api cargo tree budget line %d must include owner, ticket, expires and reason\n", NR
      ok = 0
    }
    $6 < today {
      printf "FAIL expired api cargo tree budget line %d: %s %s expired %s\n", NR, $1, $2, $6
      ok = 0
    }
    seen[$1 "\t" $2]++ {
      printf "FAIL duplicate api cargo tree budget entry line %d: %s %s\n", NR, $1, $2
      ok = 0
    }
    END { exit ok ? 0 : 1 }
  ' "$BUDGET"
}

budget_cap() {
  local kind="$1"
  local target="$2"
  awk -F '\t' -v kind="$kind" -v target="$target" '
    !/^#/ && NF >= 7 && $1 == kind && $2 == target { print $3; found = 1; exit }
    END { exit found ? 0 : 1 }
  ' "$BUDGET"
}

direct_dependency_count() {
  awk '/^1/ { count++ } END { print count + 0 }' "$DIRECT_TREE"
}

duplicate_package_name_count() {
  wc -l <"$DUPLICATE_NAMES" | tr -dc '0-9'
}

assert_direct_dependency_cap() {
  local expected actual
  expected="$(budget_cap max_direct_normal_deps api)"
  actual="$(direct_dependency_count)"
  if [ "$actual" -ne "$expected" ]; then
    printf 'FAIL api direct normal dependency count changed: %s (budget %s); update scripts/api_cargo_tree_budget.tsv with review evidence\n' "$actual" "$expected" >&2
    exit 1
  fi
}

assert_forbidden_default_crates_absent() {
  local crate
  while IFS=$'\t' read -r kind crate _cap _owner _ticket _expires _reason; do
    [ "$kind" = "forbidden_default_crate" ] || continue
    if rg -n "^1${crate} " "$DIRECT_TREE" >"$BAD"; then
      printf 'FAIL non-P0 crate appears in default api normal dependency tree: %s\n' "$crate" >&2
      cat "$BAD" >&2
      exit 1
    fi
  done < <(awk -F '\t' '!/^#/ && NF >= 7 { print }' "$BUDGET")
}

emit_duplicate_package_names() {
  cargo tree -p api -d -e normal --depth 0 \
    | awk '/^[A-Za-z0-9_.+-]+ v[0-9]/ { seen[$1] = 1 } END { for (name in seen) print name }' \
    | sort
}

emit_budget_duplicate_package_names() {
  awk -F '\t' '!/^#/ && NF >= 7 && $1 == "duplicate_package_name" { print $2 }' "$BUDGET" | sort
}

assert_duplicate_package_budget() {
  local expected actual
  expected="$(budget_cap max_duplicate_package_names api)"
  actual="$(duplicate_package_name_count)"
  if [ "$actual" -ne "$expected" ]; then
    printf 'FAIL api duplicate package-name count changed: %s (budget %s); update scripts/api_cargo_tree_budget.tsv with review evidence\n' "$actual" "$expected" >&2
    exit 1
  fi

  emit_budget_duplicate_package_names >"$BUDGET_DUPLICATE_NAMES"
  if ! diff -u "$BUDGET_DUPLICATE_NAMES" "$DUPLICATE_NAMES" >"$BAD"; then
    printf 'FAIL api duplicate package-name set changed; update scripts/api_cargo_tree_budget.tsv with owner/ticket/reason\n' >&2
    cat "$BAD" >&2
    exit 1
  fi
}

validate_budget_file
# --target all keeps the direct dependency count host-independent
# (e.g. the macOS-only security-framework credential backend).
cargo tree -p api --depth 1 -e normal --target all --prefix depth >"$DIRECT_TREE"
emit_duplicate_package_names >"$DUPLICATE_NAMES"
assert_direct_dependency_cap
assert_forbidden_default_crates_absent
assert_duplicate_package_budget

printf 'OK api cargo tree budget (%s direct normal deps, %s duplicate package names)\n' \
  "$(direct_dependency_count)" \
  "$(duplicate_package_name_count)"
