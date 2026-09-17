//! 机会列表行的流式增量 patch：removed/changed 按 id 收敛到同一投影集合。
//! futures 与 opportunities 模块共用，避免各自复制 patch 语义。

use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn patch_projected_rows<T: Clone>(
    rows: &mut Vec<T>,
    removed_ids: &[String],
    changed_rows: Vec<T>,
    row_id: impl Fn(&T) -> &str,
) {
    if !removed_ids.is_empty() {
        let removed = removed_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        rows.retain(|row| !removed.contains(row_id(row)));
    }
    if changed_rows.is_empty() {
        return;
    }
    let changed = changed_rows
        .into_iter()
        .map(|row| (row_id(&row).to_owned(), row))
        .collect::<BTreeMap<_, _>>();
    for row in rows.iter_mut() {
        if let Some(next) = changed.get(row_id(row)) {
            *row = next.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::patch_projected_rows;

    #[test]
    fn removes_then_replaces_matching_rows_in_place() {
        let mut rows = vec![("a", 1), ("b", 1), ("c", 1)];
        patch_projected_rows(
            &mut rows,
            &["b".to_owned()],
            vec![("c", 2), ("d", 2)],
            |row| row.0,
        );
        assert_eq!(rows, vec![("a", 1), ("c", 2)]);
    }

    #[test]
    fn empty_patch_is_noop() {
        let mut rows = vec![("a", 1)];
        patch_projected_rows(&mut rows, &[], Vec::new(), |row| row.0);
        assert_eq!(rows, vec![("a", 1)]);
    }
}
