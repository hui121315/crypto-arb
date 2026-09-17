#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::panels::modules::review) struct VenueQualityChartMeta {
    pub(in crate::panels::modules::review) source: String,
    pub(in crate::panels::modules::review) freshness: String,
    pub(in crate::panels::modules::review) problem: Option<String>,
}

impl VenueQualityChartMeta {
    pub(crate) fn ready(source: impl Into<String>, freshness: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            freshness: freshness.into(),
            problem: None,
        }
    }

    pub(crate) fn stale(
        source: impl Into<String>,
        freshness: impl Into<String>,
        problem: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            freshness: freshness.into(),
            problem: Some(problem.into()),
        }
    }

    pub(crate) fn waiting() -> Self {
        Self::ready("读取中", "等待快照")
    }

    pub(crate) fn failed(problem: impl Into<String>) -> Self {
        Self {
            source: "读取失败".to_owned(),
            freshness: "无可用快照".to_owned(),
            problem: Some(problem.into()),
        }
    }

    pub(crate) fn label(&self) -> String {
        match self.problem.as_deref() {
            Some(problem) => format!("{} · {} · 降级：{problem}", self.source, self.freshness),
            None => format!("{} · {}", self.source, self.freshness),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_label_keeps_problem_context() {
        let meta = VenueQualityChartMeta::stale("执行质量样本", "2.5s", "timeout · req-7");

        assert_eq!(meta.label(), "执行质量样本 · 2.5s · 降级：timeout · req-7");
    }
}
