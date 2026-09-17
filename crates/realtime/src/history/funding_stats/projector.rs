use super::*;
use std::collections::BTreeMap;

const MAX_CYCLE_BUCKET: u32 = 9;

/// Bounded live projection loaded once from history and then updated per cycle.
#[derive(Debug, Default)]
pub struct FundingDiffStatsProjector {
    rows: BTreeMap<FundingDiffKey, BTreeMap<i64, FundingDiffRow>>,
}

impl FundingDiffStatsProjector {
    pub fn replace(&mut self, rows: Vec<FundingDiffRow>) {
        self.rows.clear();
        self.extend(rows);
    }

    pub fn apply(
        &mut self,
        rows: &[FundingDiffRow],
        computed_at_ms: i64,
    ) -> Vec<FundingDiffStatsRow> {
        self.extend(rows.iter().cloned());
        self.snapshot(computed_at_ms)
    }

    pub fn snapshot(&self, computed_at_ms: i64) -> Vec<FundingDiffStatsRow> {
        let mut out = self
            .rows
            .values()
            .filter_map(|rows| {
                let mut rows = rows.values().cloned().collect::<Vec<_>>();
                build_pair_row(&mut rows, computed_at_ms)
            })
            .collect::<Vec<_>>();
        out.sort_by_key(|row| std::cmp::Reverse(row.latest_at_ms));
        out
    }

    fn extend(&mut self, rows: impl IntoIterator<Item = FundingDiffRow>) {
        for row in rows.into_iter().filter(valid_row) {
            self.rows
                .entry(FundingDiffKey::from(&row))
                .or_default()
                .insert(row.occurred_at_ms, row);
        }
        for rows in self.rows.values_mut() {
            prune_pair_history(rows);
        }
    }

    #[cfg(test)]
    pub(super) fn retained_sample_count(&self) -> usize {
        self.rows.values().map(BTreeMap::len).sum()
    }
}

fn prune_pair_history(rows: &mut BTreeMap<i64, FundingDiffRow>) {
    let Some(latest) = rows.last_key_value().map(|(_, row)| row) else {
        return;
    };
    let window_ms = i64::from(base_interval_hours(latest).saturating_mul(MAX_CYCLE_BUCKET))
        .saturating_mul(HOUR_MS);
    let cutoff_ms = latest.occurred_at_ms.saturating_sub(window_ms);
    rows.retain(|occurred_at_ms, _| *occurred_at_ms >= cutoff_ms);
}
