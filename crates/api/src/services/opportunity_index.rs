use arc_swap::ArcSwapOption;
use chrono::{DateTime, Utc};
use realtime::SnapshotEntry;
use shared_types::{ArbitrageOpportunityDto, OpportunityScanReport};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Debug)]
pub(crate) struct OpportunityIndex {
    current: ArcSwapOption<OpportunityIndexSnapshot>,
    next_version: AtomicU64,
    update_version: watch::Sender<u64>,
}

#[derive(Debug)]
struct OpportunityIndexSnapshot {
    version: u64,
    snapshot_id: String,
    entry: Arc<SnapshotEntry<OpportunityScanReport>>,
    row_by_id: HashMap<String, usize>,
}

impl OpportunityIndexSnapshot {
    fn row(&self, id: &str) -> Option<&ArbitrageOpportunityDto> {
        self.row_by_id
            .get(id)
            .and_then(|index| self.entry.value.opportunities.get(*index))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpportunitySnapshotMismatch {
    pub expected: String,
    pub actual: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpportunityIndexHealth {
    pub version: u64,
    pub snapshot_id: Option<String>,
    pub published_at_ms: Option<i64>,
    pub freshness_ms: Option<i64>,
    pub rows: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct OpportunityIndexRead {
    snapshot: Arc<OpportunityIndexSnapshot>,
}

impl OpportunityIndexRead {
    pub(crate) fn version(&self) -> u64 {
        self.snapshot.version
    }

    pub(crate) fn snapshot_id(&self) -> &str {
        &self.snapshot.snapshot_id
    }

    pub(crate) fn rows(&self) -> &[ArbitrageOpportunityDto] {
        &self.snapshot.entry.value.opportunities
    }

    pub(crate) fn report(&self) -> &OpportunityScanReport {
        &self.snapshot.entry.value
    }

    pub(crate) fn cached_at(&self) -> DateTime<Utc> {
        self.snapshot.entry.cached_at
    }

    pub(crate) fn entry(&self) -> Arc<SnapshotEntry<OpportunityScanReport>> {
        Arc::clone(&self.snapshot.entry)
    }
}

impl Default for OpportunityIndex {
    fn default() -> Self {
        Self {
            current: ArcSwapOption::empty(),
            next_version: AtomicU64::new(0),
            update_version: watch::channel(0).0,
        }
    }
}

impl OpportunityIndex {
    pub(crate) fn current(&self, id: &str) -> Option<ArbitrageOpportunityDto> {
        self.current.load_full()?.row(id).cloned()
    }

    pub(crate) fn current_bound(&self, id: &str) -> Option<(String, ArbitrageOpportunityDto)> {
        let snapshot = self.current.load_full()?;
        let row = snapshot.row(id)?.clone();
        Some((snapshot.snapshot_id.clone(), row))
    }

    pub(crate) fn publish_report(
        &self,
        snapshot_id: String,
        published_at: DateTime<Utc>,
        report: OpportunityScanReport,
    ) -> u64 {
        let version = self
            .next_version
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let mut row_by_id = HashMap::with_capacity(report.opportunities.len());
        for (index, opportunity) in report.opportunities.iter().enumerate() {
            if !opportunity.id.is_empty() {
                row_by_id.entry(opportunity.id.clone()).or_insert(index);
            }
        }
        self.current.store(Some(Arc::new(OpportunityIndexSnapshot {
            version,
            snapshot_id,
            entry: Arc::new(SnapshotEntry {
                value: report,
                cached_at: published_at,
            }),
            row_by_id,
        })));
        self.update_version
            .send_modify(|published| *published = version);
        version
    }

    #[cfg(test)]
    pub(crate) fn publish(
        &self,
        snapshot_id: String,
        published_at_ms: i64,
        opportunities: &[ArbitrageOpportunityDto],
    ) -> u64 {
        let published_at = DateTime::<Utc>::from_timestamp_millis(published_at_ms)
            .unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
        self.publish_report(
            snapshot_id,
            published_at,
            OpportunityScanReport {
                opportunities: opportunities.to_vec(),
                ..OpportunityScanReport::default()
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn entry(&self) -> Option<Arc<SnapshotEntry<OpportunityScanReport>>> {
        self.current
            .load_full()
            .map(|snapshot| Arc::clone(&snapshot.entry))
    }

    pub(crate) fn subscribe_updates(&self) -> watch::Receiver<u64> {
        self.update_version.subscribe()
    }

    pub(crate) fn version(&self) -> u64 {
        self.current
            .load_full()
            .map_or(0, |snapshot| snapshot.version)
    }

    pub(crate) fn get_bound(
        &self,
        id: &str,
        expected_snapshot_id: Option<&str>,
    ) -> Result<Option<(String, ArbitrageOpportunityDto)>, OpportunitySnapshotMismatch> {
        let snapshot = self.current.load_full();
        if let Some(expected) = expected_snapshot_id.filter(|value| !value.trim().is_empty()) {
            let actual = snapshot
                .as_ref()
                .map_or_else(String::new, |snapshot| snapshot.snapshot_id.clone());
            if actual != expected {
                return Err(OpportunitySnapshotMismatch {
                    expected: expected.to_owned(),
                    actual,
                });
            }
        }
        Ok(snapshot.and_then(|snapshot| {
            snapshot
                .row(id)
                .cloned()
                .map(|row| (snapshot.snapshot_id.clone(), row))
        }))
    }

    pub(crate) fn health(&self, now_ms: i64) -> OpportunityIndexHealth {
        let Some(snapshot) = self.current.load_full() else {
            return OpportunityIndexHealth {
                version: 0,
                snapshot_id: None,
                published_at_ms: None,
                freshness_ms: None,
                rows: 0,
            };
        };
        let published_at_ms = snapshot.entry.cached_at.timestamp_millis();
        OpportunityIndexHealth {
            version: snapshot.version,
            snapshot_id: Some(snapshot.snapshot_id.clone()),
            published_at_ms: Some(published_at_ms),
            freshness_ms: Some(now_ms.saturating_sub(published_at_ms).max(0)),
            rows: snapshot.entry.value.opportunities.len(),
        }
    }

    pub(crate) fn read(&self) -> Option<OpportunityIndexRead> {
        self.current
            .load_full()
            .map(|snapshot| OpportunityIndexRead { snapshot })
    }
}

#[cfg(test)]
mod tests;
