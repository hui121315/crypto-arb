//! Instrument metadata evidence enums and compatibility names.
//!
//! `InstrumentSpec` is the only metadata DTO. The historical envelope name is
//! retained as an alias so callers cannot create a second, drifting contract.

use serde::{Deserialize, Serialize};

/// 元数据来源——区分“官方 endpoint 已核验”与“缓存/人工/未核验”。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentMetadataSource {
    /// 直接来自交易所官方 instrument/market endpoint 的本轮核验结果。
    OfficialEndpoint,
    /// 来自先前核验后缓存的快照，未在本轮重新核验。
    CachedSnapshot,
    /// 人工录入，未经官方 endpoint 证明。
    Manual,
    /// 未核验/占位，不可作为事实源。
    Unverified,
}

/// 合约/标的上市状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentListingStatus {
    /// 正常可交易。
    Trading,
    /// 已公布但尚未开盘。
    PreLaunch,
    /// 暂停交易。
    Suspended,
    /// 已下架。
    Delisted,
    /// 状态未知/未核验。
    Unknown,
}

/// Compatibility alias for the former duplicate metadata envelope.
pub type InstrumentMetadataEnvelope = crate::instrument_registry::InstrumentSpec;
