#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/crossline-table-budget-fixture.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

GOOD_DIR="$TMP_DIR/good"
BAD_DIR="$TMP_DIR/bad"
mkdir -p "$GOOD_DIR" "$BAD_DIR"

cat >"$GOOD_DIR/paged_table.rs" <<'RS'
fn view() {
    view! {
        <>
            <table>
                <tbody>
                    {rows.iter().map(row_view).collect_view()}
                </tbody>
            </table>
            <PageControls total=total current_page=page page_size=PAGE_SIZE/>
        </>
    }
}
RS

cat >"$GOOD_DIR/runtime_table.rs" <<'RS'
fn view() {
    view! {
        <table data-table-budget="table-runtime">
            <tbody>
                {rows.iter().map(row_view).collect_view()}
            </tbody>
        </table>
    }
}
RS

cat >"$BAD_DIR/import_context_table.rs" <<'RS'
use crate::panels::modules::pagination::PageControls;
use crate::panels::modules::pagination::page_slice;
use crate::state::module_runtime::use_table_runtime;

const PAGE_SIZE: usize = 25;

fn view() {
    view! {
        <table>
            <tbody>
                {unbounded_rows.iter().map(row_view).collect_view()}
            </tbody>
        </table>
    }
}
RS

cat >"$BAD_DIR/after_const_table.rs" <<'RS'
fn view() {
    view! {
        <>
            <table>
                <tbody>
                    {unbounded_rows.iter().map(row_view).collect_view()}
                </tbody>
            </table>
            let not_a_page_control = true;
            const PAGE_SIZE: usize = 25;
        </>
    }
}
RS

cat >"$BAD_DIR/adjacent_context_table.rs" <<'RS'
const PAGE_SIZE: usize = 25;

fn view() {
    view! {
        <>
            <table>
                <tbody>
                    {safe_rows.iter().map(row_view).collect_view()}
                </tbody>
            </table>
            <PageControls total=total current_page=page page_size=PAGE_SIZE/>
            <table>
                <tbody>
                    {unbounded_rows.iter().map(row_view).collect_view()}
                </tbody>
            </table>
        </>
    }
}
RS

cat >"$BAD_DIR/unbounded_then_bounded_adjacent.rs" <<'RS'
fn view() {
    view! {
        <>
            <table>
                <tbody>
                    {unbounded_rows.iter().map(row_view).collect_view()}
                </tbody>
            </table>
            <table data-table-budget="bounded-small">
                <tbody>
                    {safe_rows.iter().map(row_view).collect_view()}
                </tbody>
            </table>
        </>
    }
}
RS

bash "$ROOT/scripts/check_frontend_table_budget.sh" "$GOOD_DIR" >/dev/null

for fixture in "$BAD_DIR"/*.rs; do
  name="$(basename "$fixture")"
  if bash "$ROOT/scripts/check_frontend_table_budget.sh" "$fixture" >"$TMP_DIR/$name.out" 2>"$TMP_DIR/$name.err"; then
    printf 'frontend table budget self-test failed: %s unexpectedly passed\n' "$name" >&2
    cat "$TMP_DIR/$name.out" >&2
    exit 1
  fi

  if ! grep -q "$name" "$TMP_DIR/$name.err"; then
    printf 'frontend table budget self-test failed: %s was not reported\n' "$name" >&2
    cat "$TMP_DIR/$name.err" >&2
    exit 1
  fi
done

printf 'OK frontend table budget self-test\n'
