#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="smart"
PLAN_ONLY=0
# Keep the incremental verification snapshot outside Cargo's disposable target
# tree so `cargo clean` does not turn the next focused finish into a full gate.
STATE_DIR="${CROSSLINE_FINISH_STATE_DIR:-$ROOT/.crossline-runtime/finish-state}"
STATE_FILE="$STATE_DIR/files.tsv"
RELEASE_DIST="${CROSSLINE_RELEASE_DIST:-$ROOT/target/release-frontend-dist}"
TMP_BASE="${TMPDIR:-/tmp}/crossline-finish.$$"
CURRENT_SNAPSHOT="$TMP_BASE.current.tsv"
CURRENT_PATHS="$TMP_BASE.paths"
CURRENT_HASHES="$TMP_BASE.hashes"
trap 'rm -f "$CURRENT_SNAPSHOT" "$CURRENT_PATHS" "$CURRENT_HASHES"' EXIT

usage() {
  cat <<'EOF'
usage: scripts/finish_batch.sh [--release] [--plan]

  default     Detect changed surfaces and run only their focused checks.
  --release   Run one workspace-level strong finish before push or release.
  --plan      Print the selected finish plan without executing it.

Set CROSSLINE_FINISH_FILES to a newline-separated path list to plan or verify a
specific batch instead of the whole dirty worktree.
EOF
}

for arg in "$@"; do
  case "$arg" in
    --release) MODE="release" ;;
    --plan) PLAN_ONLY=1 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

cd "$ROOT"

snapshot_tree() {
  git ls-files -co --exclude-standard | sort -u | while IFS= read -r path; do
    [ -f "$path" ] && printf '%s\n' "$path"
  done > "$CURRENT_PATHS"
  git hash-object --stdin-paths < "$CURRENT_PATHS" > "$CURRENT_HASHES"
  paste "$CURRENT_PATHS" "$CURRENT_HASHES" > "$CURRENT_SNAPSHOT"
}

snapshot_tree
CHANGE_SOURCE="git"
if [ -n "${CROSSLINE_FINISH_FILES:-}" ]; then
  CHANGE_SOURCE="explicit"
elif [ -s "$STATE_FILE" ]; then
  CHANGE_SOURCE="last-success"
fi

changed_files() {
  if [ -n "${CROSSLINE_FINISH_FILES:-}" ]; then
    printf '%s\n' "$CROSSLINE_FINISH_FILES"
    return
  fi
  if [ -s "$STATE_FILE" ]; then
    awk -F '\t' '
      NR == FNR { previous[$1] = $2; next }
      { current[$1] = 1; if (!($1 in previous) || previous[$1] != $2) print $1 }
      END { for (path in previous) if (!(path in current)) print path }
    ' "$STATE_FILE" "$CURRENT_SNAPSHOT" | sort -u
    return
  fi
  {
    git diff --name-only --diff-filter=ACMRTUXB HEAD
    git ls-files --others --exclude-standard
  } | awk 'NF' | sort -u
}

CHANGED="$(changed_files)"

matches() {
  [ -n "$CHANGED" ] && rg -q "$1" <<< "$CHANGED"
}

BACKEND=0
FRONTEND_RUST=0
FRONTEND_UI=0
DOCS=0
SHELL=0
WORKSPACE_RUST=0
MODULE_SIZE=0

if matches '^(Cargo\.(toml|lock)|rust-toolchain\.toml|\.clippy\.toml|crates/|shared-types/)'; then
  BACKEND=1
fi
if matches '^(frontend/.*\.rs|frontend/Cargo\.(toml|lock)|shared-types/)'; then
  FRONTEND_RUST=1
fi
if matches '^(frontend/(src|styles|index\.html|Trunk\.toml)|DESIGN\.md|test/e2e/)'; then
  FRONTEND_UI=1
fi
if matches '^(AGENTS\.md|README\.md|DESIGN\.md|docs/)'; then
  DOCS=1
fi
if matches '^(scripts/.*\.sh|\.github/.*\.ya?ml)$'; then
  SHELL=1
fi
if matches '^(Cargo\.(toml|lock)|rust-toolchain\.toml|\.clippy\.toml|shared-types/)'; then
  WORKSPACE_RUST=1
fi
if matches '^(crates/(portfolio|review|api|options|realtime|simulation|trading)/|frontend/|shared-types/src/(portfolio|review|strategy|system|venues)\.rs|scripts/(check_module_size\.sh|module_size_debt_allowlist\.tsv))'; then
  MODULE_SIZE=1
fi

if [ "$MODE" = "release" ]; then
  BACKEND=1
  FRONTEND_RUST=1
  FRONTEND_UI=1
  DOCS=1
  SHELL=1
  WORKSPACE_RUST=1
  MODULE_SIZE=1
fi

PACKAGES=()
if [ "$BACKEND" -eq 1 ] && [ "$WORKSPACE_RUST" -eq 0 ]; then
  while IFS= read -r package; do
    [ -n "$package" ] || continue
    [ -f "$ROOT/crates/$package/Cargo.toml" ] || continue
    PACKAGES[${#PACKAGES[@]}]="$package"
  done < <(printf '%s\n' "$CHANGED" | sed -n 's#^crates/\([^/]*\)/.*#\1#p' | sort -u)
fi

package_csv="workspace"
if [ "$WORKSPACE_RUST" -eq 0 ]; then
  package_csv="none"
  if [ "${#PACKAGES[@]}" -gt 0 ]; then
    package_csv="$(IFS=,; printf '%s' "${PACKAGES[*]}")"
  fi
fi

printf 'CROSSLINE finish plan: mode=%s source=%s backend=%s packages=%s frontend_rust=%s frontend_ui=%s docs=%s shell=%s module_size=%s\n' \
  "$MODE" "$CHANGE_SOURCE" "$BACKEND" "$package_csv" "$FRONTEND_RUST" "$FRONTEND_UI" "$DOCS" "$SHELL" "$MODULE_SIZE"

if [ "$PLAN_ONLY" -eq 1 ]; then
  exit 0
fi
if [ -z "$CHANGED" ] && [ "$MODE" = "smart" ]; then
  printf 'OK finish: no changed files\n'
  exit 0
fi

STARTED_AT="$(date +%s)"
PHASES=0

run_phase() {
  local label="$1"
  shift
  PHASES=$((PHASES + 1))
  printf '\n==> %s\n' "$label"
  "$@"
}

run_active_docs() {
  bash scripts/check_authority_docs.sh
  bash scripts/check_doc_path_references.sh
}

run_backend() {
  if [ "$WORKSPACE_RUST" -eq 1 ] || [ "${#PACKAGES[@]}" -eq 0 ]; then
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace --no-fail-fast
    return
  fi
  local clippy_args=(cargo clippy)
  local test_args=(cargo test)
  local package
  for package in "${PACKAGES[@]}"; do
    clippy_args+=(-p "$package")
    test_args+=(-p "$package")
  done
  clippy_args+=(--all-targets -- -D warnings)
  test_args+=(--no-fail-fast)
  "${clippy_args[@]}"
  "${test_args[@]}"
}

run_frontend_rust() {
  cargo clippy --manifest-path frontend/Cargo.toml \
    --target wasm32-unknown-unknown --all-targets -- -D warnings
  cargo test --manifest-path frontend/Cargo.toml --lib --no-fail-fast
}

run_backend_format() {
  cargo fmt --all -- --check
}

run_frontend_format() {
  cargo fmt --manifest-path frontend/Cargo.toml --all -- --check
}

run_ui_static() {
  bash scripts/product_copy_gate.sh
  bash scripts/build_frontend_css.sh >/dev/null
  bash scripts/check_frontend_css_generated.sh
  bash scripts/product_ui_perf_gate.sh
}

run_frontend_release() {
  (
    cd frontend
    NO_COLOR=true trunk build --release --dist "$RELEASE_DIST"
  )
}

run_shell_syntax() {
  local path
  while IFS= read -r path; do
    case "$path" in
      scripts/*.sh) bash -n "$path" ;;
    esac
  done <<< "$CHANGED"
}

if [ "$SHELL" -eq 1 ]; then
  run_phase "Changed shell syntax" run_shell_syntax
fi
if [ "$DOCS" -eq 1 ]; then
  run_phase "Current documentation contracts" run_active_docs
fi
if [ "$BACKEND" -eq 1 ]; then
  run_phase "Rust formatting" run_backend_format
  run_phase "Rust module boundaries" bash scripts/check_crate_root_boundaries.sh
fi
if [ "$FRONTEND_RUST" -eq 1 ]; then
  run_phase "Frontend formatting" run_frontend_format
  run_phase "Frontend module boundaries" bash scripts/check_frontend_module_boundaries.sh
fi
if [ "$MODULE_SIZE" -eq 1 ]; then
  run_phase "Module size" bash scripts/check_module_size.sh
fi
if [ "$FRONTEND_UI" -eq 1 ]; then
  run_phase "UI static contract" run_ui_static
fi
if [ "$MODE" = "release" ]; then
  run_phase "Frontend release build" run_frontend_release
  run_phase "Wasm budget" env \
    WASM_DIST_DIR="$RELEASE_DIST" WASM_REQUIRE_OZ=1 \
    bash scripts/check_wasm_budget.sh
fi
if [ "$BACKEND" -eq 1 ]; then
  run_phase "Backend lint and tests" run_backend
fi
if [ "$FRONTEND_RUST" -eq 1 ]; then
  run_phase "Frontend Wasm lint and tests" run_frontend_rust
fi
if [ -z "${CROSSLINE_FINISH_FILES:-}" ]; then
  mkdir -p "$STATE_DIR"
  cp "$CURRENT_SNAPSHOT" "$STATE_FILE"
fi

ELAPSED=$(( $(date +%s) - STARTED_AT ))
printf '\nOK finish: %s phases passed in %ss (retired audit matrix excluded)\n' \
  "$PHASES" "$ELAPSED"
