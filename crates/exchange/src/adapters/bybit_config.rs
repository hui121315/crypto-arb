//! Bybit adapter configuration types.

use crate::signing::bybit as sign;
use crate::venue_spec::VenueId;

#[derive(Debug, Clone)]
pub struct BybitCredentials {
    pub api_key: String,
    pub api_secret: String,
}

/// Bybit V5 `accountType`（修复 P1 3.1）。
///
/// Bybit 历史上分 UTA（Unified Trading Account，2023+）与 Classic Account（2023 前老账户）：
/// - `Unified`：默认。新注册用户、UTA 升级用户。
/// - `Contract`：Classic Account 衍生品账户（独立保证金）。
/// - `Spot`：Classic Account 现货账户。
/// - `Fund`：Funding Account（充提中转）。
///
/// 老 Classic 账户用户传 `UNIFIED` 会得到空 `result.list`。本 enum 让运行时配置正确账户类型。
/// 文档参考：[Get Wallet Balance](https://bybit-exchange.github.io/docs/v5/account/wallet-balance)。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BybitAccountType {
    #[default]
    Unified,
    Contract,
    Spot,
    Fund,
}

impl BybitAccountType {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Unified => "UNIFIED",
            Self::Contract => "CONTRACT",
            Self::Spot => "SPOT",
            Self::Fund => "FUND",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BybitConfig {
    pub credentials: Option<BybitCredentials>,
    pub testnet: bool,
    pub allow_live_writes: bool,
    pub timeout_secs: u64,
    pub qps: u32,
    pub recv_window: String,
    pub base_url_override: Option<String>,
    /// 修复 P1 3.1：Bybit V5 `accountType` 必填，原硬编码 `UNIFIED` 导致 Classic 账户
    /// 用户余额查询返回空。默认仍 `Unified`（多数 2023+ 用户），Classic 账户用户需
    /// 显式设为 `Contract`（衍生品）+ `Spot`（现货）。
    pub account_type: BybitAccountType,
}

impl Default for BybitConfig {
    fn default() -> Self {
        let defaults = VenueId::Bybit.defaults();
        Self {
            credentials: None,
            testnet: false,
            allow_live_writes: false,
            timeout_secs: defaults.timeout_secs,
            qps: defaults.qps,
            recv_window: sign::DEFAULT_RECV_WINDOW.into(),
            base_url_override: None,
            account_type: BybitAccountType::Unified,
        }
    }
}
