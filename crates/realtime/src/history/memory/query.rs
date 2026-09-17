use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

pub(super) fn latest_matching_rows<T: Clone>(
    rows: &VecDeque<T>,
    limit: usize,
    matches: impl Fn(&T) -> bool,
    occurred_at_ms: impl Fn(&T) -> i64,
) -> Vec<T> {
    let limit = limit.max(1).min(rows.len().max(1));
    let mut selected = BinaryHeap::with_capacity(limit);
    for (index, row) in rows.iter().enumerate() {
        if !matches(row) {
            continue;
        }
        selected.push((Reverse(occurred_at_ms(row)), index));
        if selected.len() > limit {
            selected.pop();
        }
    }
    let mut selected = selected.into_vec();
    selected.sort_by(|left, right| {
        let left_ts = occurred_at_ms(&rows[left.1]);
        let right_ts = occurred_at_ms(&rows[right.1]);
        right_ts.cmp(&left_ts).then_with(|| left.1.cmp(&right.1))
    });
    selected
        .into_iter()
        .map(|(_, index)| rows[index].clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Debug)]
    struct CloneTrackedRow {
        occurred_at_ms: i64,
        clone_count: Arc<AtomicUsize>,
    }

    impl Clone for CloneTrackedRow {
        fn clone(&self) -> Self {
            self.clone_count.fetch_add(1, Ordering::Relaxed);
            Self {
                occurred_at_ms: self.occurred_at_ms,
                clone_count: Arc::clone(&self.clone_count),
            }
        }
    }

    #[test]
    fn bounded_query_clones_only_selected_latest_rows() {
        let clone_count = Arc::new(AtomicUsize::new(0));
        let rows = (0..100)
            .map(|occurred_at_ms| CloneTrackedRow {
                occurred_at_ms,
                clone_count: Arc::clone(&clone_count),
            })
            .collect::<VecDeque<_>>();

        let selected = latest_matching_rows(&rows, 3, |_| true, |row| row.occurred_at_ms);

        assert_eq!(
            selected
                .iter()
                .map(|row| row.occurred_at_ms)
                .collect::<Vec<_>>(),
            vec![99, 98, 97]
        );
        assert_eq!(clone_count.load(Ordering::Relaxed), 3);
    }
}
