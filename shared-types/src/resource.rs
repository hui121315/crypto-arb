//! Generic resource envelope for non-market REST snapshots and bounded registries.

use crate::ApiProblem;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceStatus {
    #[default]
    Ready,
    Warming,
    Degraded,
    Partial,
    Error,
}

impl ResourceStatus {
    #[must_use]
    pub const fn has_usable_data(self) -> bool {
        matches!(self, Self::Ready | Self::Degraded | Self::Partial)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCoverage {
    pub expected: u64,
    pub observed: u64,
    pub coverage_pct: f64,
    pub truncated: bool,
}

impl ResourceCoverage {
    #[must_use]
    pub fn new(expected: usize, observed: usize) -> Self {
        let expected = expected as u64;
        let observed = observed as u64;
        let coverage_pct = if expected == 0 {
            1.0
        } else {
            (observed as f64 / expected as f64).clamp(0.0, 1.0)
        };
        Self {
            expected,
            observed,
            coverage_pct,
            truncated: observed < expected,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceEnvelope<T> {
    pub data: Option<T>,
    pub status: ResourceStatus,
    pub source: String,
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<ResourceCoverage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<ApiProblem>,
}

impl<T> ResourceEnvelope<T> {
    #[must_use]
    pub fn with_data(
        data: T,
        status: ResourceStatus,
        source: impl Into<String>,
        observed_at_ms: i64,
        problems: Vec<ApiProblem>,
    ) -> Self {
        debug_assert!(status.has_usable_data());
        Self {
            data: Some(data),
            status,
            source: source.into(),
            observed_at_ms,
            coverage: None,
            problems,
        }
    }

    #[must_use]
    pub fn unavailable(
        status: ResourceStatus,
        source: impl Into<String>,
        observed_at_ms: i64,
        problems: Vec<ApiProblem>,
    ) -> Self {
        debug_assert!(!status.has_usable_data());
        Self {
            data: None,
            status,
            source: source.into(),
            observed_at_ms,
            coverage: None,
            problems,
        }
    }

    #[must_use]
    pub fn with_coverage(mut self, coverage: ResourceCoverage) -> Self {
        self.coverage = Some(coverage);
        self
    }

    #[must_use]
    pub fn primary_problem(&self) -> Option<&ApiProblem> {
        self.problems.first()
    }

    pub fn into_data(self) -> Result<T, Box<ApiProblem>> {
        match self.data {
            Some(data) if self.status.has_usable_data() => Ok(data),
            Some(_) | None => Err(Box::new(self.problems.into_iter().next().unwrap_or_else(
                || {
                    ApiProblem::new(
                        "RESOURCE_DATA_UNAVAILABLE",
                        format!("{} resource has no usable data", self.source),
                    )
                    .with_source(self.source)
                },
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_complete_resource_has_full_coverage() {
        let coverage = ResourceCoverage::new(0, 0);

        assert_eq!(coverage.coverage_pct, 1.0);
        assert!(!coverage.truncated);
    }

    #[test]
    fn partial_resource_keeps_data_and_problem() {
        let envelope = ResourceEnvelope::with_data(
            vec![1_u8],
            ResourceStatus::Partial,
            "registry",
            1_000,
            vec![ApiProblem::new("TRUNCATED", "bounded")],
        )
        .with_coverage(ResourceCoverage::new(2, 1));

        assert_eq!(envelope.status, ResourceStatus::Partial);
        assert_eq!(
            envelope.primary_problem().map(|item| item.code.as_str()),
            Some("TRUNCATED")
        );
        assert_eq!(envelope.into_data(), Ok(vec![1]));
    }

    #[test]
    fn error_resource_never_publishes_missing_data() {
        let envelope = ResourceEnvelope::<u8>::unavailable(
            ResourceStatus::Error,
            "registry",
            1_000,
            vec![ApiProblem::new("FAILED", "unavailable")],
        );

        assert_eq!(
            envelope.into_data().map_err(|problem| problem.code.clone()),
            Err("FAILED".into())
        );
    }
}
