//! 凭证探针矩阵的实盘就绪推导（PR-DG）。
//!
//! 保存交易所凭证后，后端会回填一组 read-only 探针（balance/positions/open
//! orders/order permission/account mode）。本模块把这些散点探针归并成一个
//! fail-closed 的实盘就绪判定：只有当**每一条**实盘必需链路都存在且明确 `Ok`
//! 时，才认为该交易所凭证“可实盘交易”。任何缺失、`Failed`（被交易所明确拒绝）
//! 或 `Unknown`（瞬时/未证明）都会阻断——“已配置”绝不冒充“可实盘交易”。
//!
//! 私有 WS 运行态就绪不在保存期探针矩阵内判定，由 operation-health 运行态证据
//! 层（PR-AS）单独门禁。

use crate::venues::{VenueCredentialProbeStatus, VenueCredentialValidationEvidence};
use serde::{Deserialize, Serialize};

/// 实盘交易所需的凭证探针链路。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialProbeLink {
    BalanceRead,
    PositionsRead,
    OpenOrdersRead,
    OrderPermission,
    AccountModeRead,
}

impl CredentialProbeLink {
    /// 与后端写入的 `VenueCredentialProbe.kind` 对齐的链路标识。
    pub fn kind(self) -> &'static str {
        match self {
            Self::BalanceRead => "balance_read",
            Self::PositionsRead => "positions_read",
            Self::OpenOrdersRead => "open_orders_read",
            Self::OrderPermission => "order_permission",
            Self::AccountModeRead => "account_mode_read",
        }
    }

    /// 实盘下单硬门禁要求的完整链路集合。
    pub fn live_required() -> [CredentialProbeLink; 5] {
        [
            Self::BalanceRead,
            Self::PositionsRead,
            Self::OpenOrdersRead,
            Self::OrderPermission,
            Self::AccountModeRead,
        ]
    }
}

/// 单条链路的归并结果状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialLinkStatus {
    /// 探针存在且明确通过。
    Ok,
    /// 探针存在但被交易所明确拒绝。
    Failed,
    /// 探针存在但瞬时/未证明。
    Unknown,
    /// 该链路根本没有探针证据。
    Missing,
}

impl CredentialLinkStatus {
    fn from_probe(status: VenueCredentialProbeStatus) -> Self {
        match status {
            VenueCredentialProbeStatus::Ok => Self::Ok,
            VenueCredentialProbeStatus::Failed => Self::Failed,
            VenueCredentialProbeStatus::Unknown => Self::Unknown,
        }
    }

    /// 是否为通过状态。
    pub fn is_ok(self) -> bool {
        matches!(self, Self::Ok)
    }
}

/// 实盘就绪聚合结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialReadiness {
    /// 全部必需链路通过。
    LiveReady,
    /// 至少一条链路被明确拒绝。
    Blocked,
    /// 无被拒链路，但仍有缺失/未证明链路。
    Incomplete,
}

/// 对一份保存期凭证校验证据做 fail-closed 的实盘就绪推导。
pub trait CredentialProbeMatrix {
    /// 查询某条链路的归并状态。
    fn link_status(&self, link: CredentialProbeLink) -> CredentialLinkStatus;

    /// 所有未通过（Failed/Unknown/Missing）的实盘必需链路。
    fn blocking_links(&self) -> Vec<CredentialProbeLink> {
        CredentialProbeLink::live_required()
            .into_iter()
            .filter(|&l| !self.link_status(l).is_ok())
            .collect()
    }

    /// fail-closed：仅当每条实盘必需链路都明确 `Ok` 时才就绪。
    fn is_live_trading_ready(&self) -> bool {
        CredentialProbeLink::live_required()
            .into_iter()
            .all(|l| self.link_status(l).is_ok())
    }

    /// 实盘就绪聚合结论：被拒优先（Blocked），其次缺失/未证明（Incomplete）。
    fn readiness(&self) -> CredentialReadiness {
        let blocking = self.blocking_links();
        if blocking.is_empty() {
            return CredentialReadiness::LiveReady;
        }
        if blocking
            .iter()
            .any(|&l| self.link_status(l) == CredentialLinkStatus::Failed)
        {
            return CredentialReadiness::Blocked;
        }
        CredentialReadiness::Incomplete
    }
}

impl CredentialProbeMatrix for VenueCredentialValidationEvidence {
    fn link_status(&self, link: CredentialProbeLink) -> CredentialLinkStatus {
        match self.probes.iter().find(|p| p.kind == link.kind()) {
            Some(probe) => CredentialLinkStatus::from_probe(probe.status),
            None => CredentialLinkStatus::Missing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::venues::{
        VenueCredentialProbe, VenueCredentialProbeStatus, VenueCredentialValidationEvidence,
        VenueCredentialValidationStatus,
    };

    fn probe(kind: &str, status: VenueCredentialProbeStatus) -> VenueCredentialProbe {
        VenueCredentialProbe {
            kind: kind.to_owned(),
            status,
            scope: "test".to_owned(),
            source: "test".to_owned(),
            message: "test".to_owned(),
            checked_at_ms: 1,
            request_id: None,
        }
    }

    fn evidence(probes: Vec<VenueCredentialProbe>) -> VenueCredentialValidationEvidence {
        VenueCredentialValidationEvidence {
            status: VenueCredentialValidationStatus::ReadOnlyOk,
            checked_at_ms: 1,
            probes,
            permission_evidence: Vec::new(),
        }
    }

    fn all_ok() -> Vec<VenueCredentialProbe> {
        CredentialProbeLink::live_required()
            .into_iter()
            .map(|l| probe(l.kind(), VenueCredentialProbeStatus::Ok))
            .collect()
    }

    #[test]
    fn all_required_ok_is_live_ready() {
        let e = evidence(all_ok());
        assert!(e.is_live_trading_ready());
        assert!(e.blocking_links().is_empty());
        assert_eq!(e.readiness(), CredentialReadiness::LiveReady);
    }

    #[test]
    fn missing_link_is_not_ready_and_incomplete() {
        let mut probes = all_ok();
        probes.retain(|p| p.kind != CredentialProbeLink::OrderPermission.kind());
        let e = evidence(probes);
        assert!(!e.is_live_trading_ready());
        assert_eq!(
            e.link_status(CredentialProbeLink::OrderPermission),
            CredentialLinkStatus::Missing
        );
        assert_eq!(e.readiness(), CredentialReadiness::Incomplete);
    }

    #[test]
    fn unknown_link_is_not_ready_and_incomplete() {
        let mut probes = all_ok();
        probes.retain(|p| p.kind != CredentialProbeLink::BalanceRead.kind());
        probes.push(probe(
            CredentialProbeLink::BalanceRead.kind(),
            VenueCredentialProbeStatus::Unknown,
        ));
        let e = evidence(probes);
        assert!(!e.is_live_trading_ready());
        assert_eq!(e.readiness(), CredentialReadiness::Incomplete);
    }

    #[test]
    fn failed_link_blocks_even_with_others_ok() {
        let mut probes = all_ok();
        probes.retain(|p| p.kind != CredentialProbeLink::PositionsRead.kind());
        probes.push(probe(
            CredentialProbeLink::PositionsRead.kind(),
            VenueCredentialProbeStatus::Failed,
        ));
        let e = evidence(probes);
        assert!(!e.is_live_trading_ready());
        assert_eq!(e.readiness(), CredentialReadiness::Blocked);
        assert!(e
            .blocking_links()
            .contains(&CredentialProbeLink::PositionsRead));
    }

    #[test]
    fn failed_takes_precedence_over_missing() {
        // 一条 Failed + 一条 Missing -> Blocked（被拒优先）。
        let mut probes = all_ok();
        probes.retain(|p| {
            p.kind != CredentialProbeLink::AccountModeRead.kind()
                && p.kind != CredentialProbeLink::OpenOrdersRead.kind()
        });
        probes.push(probe(
            CredentialProbeLink::AccountModeRead.kind(),
            VenueCredentialProbeStatus::Failed,
        ));
        let e = evidence(probes);
        assert_eq!(e.readiness(), CredentialReadiness::Blocked);
        assert_eq!(
            e.link_status(CredentialProbeLink::OpenOrdersRead),
            CredentialLinkStatus::Missing
        );
    }

    #[test]
    fn empty_evidence_blocks_all_links() {
        let e = evidence(vec![]);
        assert!(!e.is_live_trading_ready());
        assert_eq!(e.blocking_links().len(), 5);
        assert_eq!(e.readiness(), CredentialReadiness::Incomplete);
    }

    #[test]
    fn extra_unrelated_probe_does_not_grant_readiness() {
        let mut probes = all_ok();
        probes.retain(|p| p.kind != CredentialProbeLink::OrderPermission.kind());
        // agent_approval 不是通用必需链路，不能替代 order_permission。
        probes.push(probe("agent_approval", VenueCredentialProbeStatus::Ok));
        let e = evidence(probes);
        assert!(!e.is_live_trading_ready());
        assert!(e
            .blocking_links()
            .contains(&CredentialProbeLink::OrderPermission));
    }
}
