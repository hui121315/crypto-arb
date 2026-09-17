#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:---report}"
PROJECT_CLEAN_THRESHOLD_GIB="${PROJECT_CLEAN_THRESHOLD_GIB:-16}"
PROJECT_CLEAN_HARD_LIMIT_GIB="${PROJECT_CLEAN_HARD_LIMIT_GIB:-48}"
PROJECT_CLEAN_MIN_FREE_GIB="${PROJECT_CLEAN_MIN_FREE_GIB:-64}"
PROTECTED_STATE_WARN_GIB="${PROTECTED_STATE_WARN_GIB:-20}"

bytes_of() {
  local path="$1"
  if [ -e "$path" ]; then
    du -sk "$path" 2>/dev/null | awk '{print $1 * 1024}'
  else
    echo 0
  fi
}

size_of() {
  local path="$1"
  if [ -e "$path" ]; then
    du -sh "$path" 2>/dev/null || true
  fi
}

fmt_gib() {
  local bytes="$1"
  awk -v bytes="$bytes" 'BEGIN { printf "%.1fGiB", bytes / 1024 / 1024 / 1024 }'
}

crossline_tmp_target_paths() {
  local path
  for path in /tmp/crossline-*-target /tmp/codex-*-target; do
    [ -d "$path" ] || continue
    printf '%s\n' "$path"
  done
}

tmp_target_bytes() {
  local total=0
  local path size
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    size="$(bytes_of "$path")"
    total=$((total + size))
  done < <(crossline_tmp_target_paths)
  echo "$total"
}

report_tmp_targets() {
  local found=0
  local path
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    found=1
    size_of "$path"
  done < <(crossline_tmp_target_paths)

  if [ "$found" -eq 0 ]; then
    echo "0B /tmp/crossline-*-target"
  fi
}

remove_tmp_targets() {
  local path
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    rm -rf "$path"
  done < <(crossline_tmp_target_paths)
}

remove_trunk_ephemeral() {
  local path="$HOME/Library/Caches/dev.trunkrs.trunk"
  local item
  if [ -d "$path" ]; then
    for item in "$path"/* "$path"/.[!.]* "$path"/..?*; do
      [ -e "$item" ] || continue
      if [ -d "$item" ]; then
        case "$(basename "$item")" in
          dart-sass-* | sass-* | tailwindcss-* | tailwindcss-extra-* | wasm-bindgen-* | wasm-opt-* | binaryen-*)
            continue
            ;;
        esac
      fi
      rm -rf "$item"
    done
  fi
}

report_project() {
  echo "Project build artifacts:"
  size_of "$ROOT/target"
  size_of "$ROOT/frontend/target"
  size_of "$ROOT/frontend/dist"
  size_of "$ROOT/.trunk"
  report_tmp_targets
  size_of "$ROOT/.git"
}

report_global() {
  echo
  echo "Global development caches:"
  size_of "$HOME/.cargo"
  size_of "$HOME/.rustup"
  size_of "$HOME/.cache"
  size_of "$HOME/.cache/codex-runtimes"
  size_of "$HOME/.codex/.tmp"
  size_of "$HOME/Library/Caches"
  size_of "$HOME/Library/Caches/dev.trunkrs.trunk"
  size_of "$HOME/Library/Caches/ms-playwright"
}

safe_dev_cache_paths() {
  local path
  for path in \
    "$HOME/.cargo/registry/cache" \
    "$HOME/.cargo/git/db" \
    "$HOME/.cache/wasm-pack" \
    "$HOME/Library/Caches/Homebrew" \
    "$HOME/Library/Caches/pip" \
    "$HOME/Library/Caches/node-gyp" \
    "$HOME/Library/Caches/org.swift.swiftpm"; do
    [ -e "$path" ] || continue
    printf '%s\n' "$path"
  done
  if [ "${CACHE_HYGIENE_DELETE_PLAYWRIGHT:-0}" = "1" ]; then
    for path in \
      "$HOME/Library/Caches/ms-playwright" \
      "$HOME/Library/Caches/ms-playwright-go"; do
      [ -e "$path" ] || continue
      printf '%s\n' "$path"
    done
  fi
}

report_safe_dev_caches() {
  local found=0
  local path

  echo
  echo "Safe development caches pressure-cleanable:"
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    found=1
    size_of "$path"
  done < <(safe_dev_cache_paths)

  if [ "$found" -eq 0 ]; then
    echo "0B safe development caches"
  fi
}

report_observed_large_stores() {
  echo
  echo "Large stores reported only, not auto-cleaned:"
  size_of "$HOME/.codex"
  size_of "$HOME/.codex/sessions"
  size_of "$HOME/.codex/logs_2.sqlite"
  size_of "$HOME/Library/Caches"
  size_of "$HOME/Library/Application Support/Google/Chrome"
  size_of "$HOME/Library/Caches/Google/Chrome"
  size_of "$HOME/Library/Application Support/Codex"
  size_of "$HOME/Library/Application Support/aicoin"
}

report_cleanup_policy() {
  echo
  echo "Cleanup policy:"
  echo "auto-cleanable: project target dirs, frontend dist/.trunk, Trunk ephemeral cache"
  echo "hard limit: clean project build artifacts above ${PROJECT_CLEAN_HARD_LIMIT_GIB}GiB even when free disk is healthy"
  echo "preserved Trunk tools: wasm-bindgen, wasm-opt/binaryen, sass, and tailwind binaries"
  echo "pressure-cleanable: Cargo download cache, Homebrew/pip/node-gyp/SwiftPM caches"
  echo "playwright browsers: preserved by default; set CACHE_HYGIENE_DELETE_PLAYWRIGHT=1 to remove"
  echo "disk-pressure mode: CACHE_HYGIENE_DISK_PRESSURE=1 bash scripts/cache_hygiene.sh --auto-clean"
  echo "direct pressure clean: bash scripts/cache_hygiene.sh --pressure-clean"
  echo "protected: .codex state, browser profiles/caches, app support folders; report only unless explicitly requested"
}

safe_ephemeral_bytes() {
  local total=0
  local size
  for path in "$HOME/Library/Caches/dev.trunkrs.trunk" "$ROOT/frontend/dist" "$ROOT/.trunk"; do
    size="$(bytes_of "$path")"
    total=$((total + size))
  done
  size="$(tmp_target_bytes)"
  total=$((total + size))
  echo "$total"
}

safe_dev_cache_bytes() {
  local total=0
  local path size

  while IFS= read -r path; do
    [ -n "$path" ] || continue
    size="$(bytes_of "$path")"
    total=$((total + size))
  done < <(safe_dev_cache_paths)

  echo "$total"
}

reported_global_cache_bytes() {
  local total=0
  local size
  for path in \
    "$HOME/.cargo" \
    "$HOME/.rustup" \
    "$HOME/.cache" \
    "$HOME/Library/Caches"; do
    size="$(bytes_of "$path")"
    total=$((total + size))
  done
  echo "$total"
}

protected_store_bytes() {
  local total=0
  local size
  for path in \
    "$HOME/.codex/.tmp" \
    "$HOME/.codex/sessions" \
    "$HOME/.codex/logs_2.sqlite" \
    "$HOME/Library/Application Support/Google/Chrome" \
    "$HOME/Library/Caches/Google/Chrome" \
    "$HOME/Library/Application Support/Codex" \
    "$HOME/Library/Application Support/aicoin"; do
    size="$(bytes_of "$path")"
    total=$((total + size))
  done
  echo "$total"
}

report_cache_summary() {
  echo
  echo "Cache summary:"
  echo "build cache auto-cleanable: $(fmt_gib "$(project_build_bytes)")"
  echo "safe ephemeral auto-cleanable: $(fmt_gib "$(safe_ephemeral_bytes)")"
  echo "safe dev cache pressure-cleanable: $(fmt_gib "$(safe_dev_cache_bytes)")"
  echo "global dev caches retained: $(fmt_gib "$(reported_global_cache_bytes)")"
  echo "protected user/app state report-only: $(fmt_gib "$(protected_store_bytes)")"
}

report_pressure_advice() {
  local protected_bytes warn_bytes
  protected_bytes="$(protected_store_bytes)"
  warn_bytes=$((PROTECTED_STATE_WARN_GIB * 1024 * 1024 * 1024))

  if [ "$protected_bytes" -ge "$warn_bytes" ]; then
    echo
    echo "Disk pressure note:"
    echo "protected user/app state is above ${PROTECTED_STATE_WARN_GIB}GiB."
    echo "The largest usual bucket is .codex/sessions, which is conversation history,"
    echo "so this script reports it but does not delete it automatically."
  fi
}

project_build_bytes() {
  local total=0
  local size
  for path in "$ROOT/target" "$ROOT/frontend/target" "$ROOT/frontend/dist" "$ROOT/.trunk"; do
    size="$(bytes_of "$path")"
    total=$((total + size))
  done
  size="$(tmp_target_bytes)"
  total=$((total + size))
  echo "$total"
}

active_build_processes() {
  local process_list
  process_list="$(LC_ALL=C ps -axo pid=,comm=,args= 2>/dev/null)" || return 2
  printf '%s\n' "$process_list" \
    | LC_ALL=C awk '
      $2 ~ /^(cargo|rustc|trunk|wasm-bindgen|crypto-arb-api)$/ {
        print
        next
      }
      $0 ~ /target\/(debug|release)\/(api|crypto-arb-api)( |$)/ {
        print
      }
    '
}

clean_project() {
  local active
  if ! active="$(active_build_processes)"; then
    echo "Refusing to clean because build/dev process state is unavailable."
    exit 1
  fi
  if [ -n "$active" ]; then
    echo "Refusing to clean while build/dev processes are active:"
    echo "$active"
    exit 1
  fi

  echo "Before clean:"
  report_project

  (cd "$ROOT" && cargo clean)
  if [ -f "$ROOT/frontend/Cargo.toml" ]; then
    cargo clean --manifest-path "$ROOT/frontend/Cargo.toml"
  fi
  rm -rf "$ROOT/frontend/dist"
  rm -rf "$ROOT/.trunk"
  remove_tmp_targets

  echo
  echo "After clean:"
  report_project
}

clean_ephemeral() {
  local active
  echo "Before ephemeral clean:"
  report_global
  report_safe_dev_caches
  report_tmp_targets

  if ! active="$(active_build_processes)"; then
    echo "Skipping build-adjacent ephemeral clean because process state is unavailable."
  elif [ -z "$active" ]; then
    remove_trunk_ephemeral
    rm -rf "$ROOT/frontend/dist"
    rm -rf "$ROOT/.trunk"
    remove_tmp_targets
  else
    echo "Skipping build-adjacent ephemeral clean while build/dev processes are active."
  fi

  echo
  echo "After ephemeral clean:"
  report_global
  report_safe_dev_caches
  report_tmp_targets
  report_cleanup_policy
}

clean_dev_caches() {
  local active path
  if ! active="$(active_build_processes)"; then
    echo "Refusing to clean development caches because process state is unavailable."
    exit 1
  fi
  if [ -n "$active" ]; then
    echo "Refusing to clean development caches while build/dev processes are active:"
    echo "$active"
    exit 1
  fi

  echo "Before development cache clean:"
  report_safe_dev_caches

  while IFS= read -r path; do
    [ -n "$path" ] || continue
    rm -rf "$path"
  done < <(safe_dev_cache_paths)

  echo
  echo "After development cache clean:"
  report_safe_dev_caches
}

pressure_clean() {
  clean_project
  echo
  clean_ephemeral
  echo
  clean_dev_caches
  echo
  report_observed_large_stores
  report_cache_summary
  report_pressure_advice
  report_cleanup_policy
}

auto_clean() {
  report_project
  local project_bytes threshold_bytes hard_limit_bytes free_bytes min_free_bytes
  project_bytes="$(project_build_bytes)"
  threshold_bytes=$((PROJECT_CLEAN_THRESHOLD_GIB * 1024 * 1024 * 1024))
  hard_limit_bytes=$((PROJECT_CLEAN_HARD_LIMIT_GIB * 1024 * 1024 * 1024))
  free_bytes="$(df -Pk "$ROOT" | awk 'NR == 2 {printf "%.0f\n", $4 * 1024}')"
  min_free_bytes=$((PROJECT_CLEAN_MIN_FREE_GIB * 1024 * 1024 * 1024))

  if [ "${CACHE_HYGIENE_FORCE_PROJECT:-0}" = "1" ]; then
    echo
    echo "CACHE_HYGIENE_FORCE_PROJECT=1, cleaning project build artifacts."
    clean_project
  elif [ "${CACHE_HYGIENE_DISK_PRESSURE:-0}" = "1" ]; then
    echo
    echo "CACHE_HYGIENE_DISK_PRESSURE=1, cleaning project build artifacts at the safe checkpoint."
    clean_project
  elif [ "$project_bytes" -ge "$hard_limit_bytes" ]; then
    echo
    echo "Project build artifacts crossed the ${PROJECT_CLEAN_HARD_LIMIT_GIB}GiB hard limit, cleaning."
    clean_project
  elif [ "$project_bytes" -ge "$threshold_bytes" ] && [ "$free_bytes" -lt "$min_free_bytes" ]; then
    echo
    echo "Project build artifacts crossed ${PROJECT_CLEAN_THRESHOLD_GIB}GiB and free disk is below ${PROJECT_CLEAN_MIN_FREE_GIB}GiB, cleaning."
    clean_project
  else
    echo
    echo "Retaining project build cache to avoid repeated compilation heat and disk writes."
  fi

  echo
  clean_ephemeral
  if [ "${CACHE_HYGIENE_DISK_PRESSURE:-0}" = "1" ]; then
    echo
    echo "CACHE_HYGIENE_DISK_PRESSURE=1, cleaning safe development caches."
    clean_dev_caches
  fi
  echo
  report_observed_large_stores
  report_cache_summary
  report_pressure_advice
  report_cleanup_policy
}

case "$MODE" in
  --report)
    report_project
    report_global
    report_safe_dev_caches
    report_observed_large_stores
    report_cache_summary
    report_pressure_advice
    report_cleanup_policy
    ;;
  --clean-project)
    clean_project
    ;;
  --clean-ephemeral)
    clean_ephemeral
    ;;
  --clean-dev-caches)
    clean_dev_caches
    ;;
  --pressure-clean)
    pressure_clean
    ;;
  --auto-clean)
    auto_clean
    ;;
  *)
    echo "usage: $0 [--report|--clean-project|--clean-ephemeral|--clean-dev-caches|--pressure-clean|--auto-clean]" >&2
    exit 2
    ;;
esac
