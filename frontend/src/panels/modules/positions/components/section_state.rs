//! 持仓各区段（汇总/持仓/风控/余额）的单值区段态。
//!
//! 单值区段 + 区段运行态的组合已抽到通用
//! [`crate::state::table_runtime::SectionSlot`]（`TableRuntime` 的标量兄弟），
//! positions 直接复用它：四段都从同一份 `PortfolioSnapshot` `LoadState` 派生，
//! loading/ready/stale/error 语义与 review 表格共享同一个 `SectionStatus` 核心。
pub(in crate::panels::modules::positions) use crate::state::section::SectionStatus;
pub(in crate::panels::modules::positions) use crate::state::table_runtime::SectionSlot as SectionData;

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::ApiProblem;

    #[test]
    fn error_text_keeps_request_context() {
        let problem = ApiProblem::new("RATE_LIMITED", "rate limited")
            .with_status(429)
            .with_request_id(Some("req-1".into()))
            .with_retry_after_ms(Some(2_000));

        let section: SectionData<Vec<u8>> = SectionData::error(&problem);

        assert_eq!(
            section.status.empty_text("暂无", "读取中", "读取失败"),
            "读取失败：rate limited · HTTP 429 · request_id req-1 · retry 2000ms"
        );
    }

    #[test]
    fn stale_value_is_visible_but_not_fresh() {
        let section = SectionData::stale(vec![1_u8], &ApiProblem::new("TIMEOUT", "slow"));

        assert_eq!(section.value, vec![1]);
        assert!(!section.has_fresh_value());
        assert_eq!(
            section.status.stale_note("显示上次快照").as_deref(),
            Some("显示上次快照：slow")
        );
    }

    #[test]
    fn loading_is_not_ready_empty() {
        let section: SectionData<Vec<u8>> = SectionData::loading();

        assert_eq!(
            section.status.empty_text("暂无", "读取中", "读取失败"),
            "读取中"
        );
    }
}
