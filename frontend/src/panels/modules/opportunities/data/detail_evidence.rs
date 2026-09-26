use super::{duration_label, DetailEvidence};
use crate::panels::modules::opportunity_counts::snapshot_clock;

#[derive(Clone, PartialEq)]
pub(crate) struct EvidenceAge {
    pub received: (i64, i64),
    pub freshness_ms: Option<i64>,
    pub observed_at_ms: Option<i64>,
}

impl EvidenceAge {
    pub(super) fn new(freshness_ms: Option<i64>) -> Self {
        Self {
            received: snapshot_clock(),
            freshness_ms: freshness_ms.filter(|age| *age >= 0),
            observed_at_ms: None,
        }
    }

    pub(super) fn observed_at(mut self, observed_at_ms: i64) -> Self {
        self.observed_at_ms = (observed_at_ms > 0).then_some(observed_at_ms);
        self
    }

    pub(super) fn align_to(&mut self, observed_at_ms: i64) {
        if let Some(previous) = self.observed_at_ms {
            self.freshness_ms = self.freshness_ms.map(|age| {
                age.saturating_add(observed_at_ms.saturating_sub(previous).max(0))
            });
            self.observed_at_ms = Some(previous.max(observed_at_ms));
        }
    }

    pub(crate) fn label_at(&self, clock: (i64, i64)) -> String {
        let elapsed = clock.0.saturating_sub(self.received.0)
            .max(clock.1.saturating_sub(self.received.1)).max(0);
        self.freshness_ms.map_or_else(|| "未知".into(), |age| {
            duration_label(age.saturating_add(elapsed))
        })
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct RetainedEvidence {
    pub source: String,
    pub age: EvidenceAge,
    pub request_id: String,
}

impl DetailEvidence {
    pub(super) fn retain_from(&mut self, previous: &Self) {
        self.retained = Some(previous.retained.clone().unwrap_or_else(|| RetainedEvidence {
            source: previous.source.clone(),
            age: previous.freshness.clone(),
            request_id: previous.request_id.clone(),
        }));
    }

    pub(crate) fn data_label_at(&self, clock: (i64, i64)) -> String {
        match &self.retained {
            Some(old) => format!("上次数据 · {} · 数据年龄 {}", old.source, old.age.label_at(clock)),
            None => format!("{} · 数据年龄 {}", self.source, self.freshness.label_at(clock)),
        }
    }

    pub(crate) fn data_request_id(&self) -> &str {
        self.retained.as_ref().map_or(self.request_id.as_str(), |old| old.request_id.as_str())
    }
}
