//! 现货符号覆盖规划器（backend symbol coverage planner）。
//!
//! 交易所扇出抓取现货行情时，"请求了某符号却没拿到 tick" 可能有多种原因。
//! 直接丢弃会让上层无法解释覆盖率。本模块把**每一个被请求的符号**显式归类，
//! 绝不静默丢弃：
//!
//! - [`SymbolCoverageStatus::Listed`]    交易所上架且本次返回了可用 tick。
//! - [`SymbolCoverageStatus::Unlisted`]  交易所可达、清单已知，但不含该符号。
//! - [`SymbolCoverageStatus::Failed`]    本次抓取失败（网络/解析），上架与否未知。
//! - [`SymbolCoverageStatus::Unsupported`] 符号形态无法解析出受支持的计价资产，
//!   适配器无法为其构造市场（绝不臆测默认计价币）。

use shared_types::SpotTick;

use crate::spot::resolve_pair;

/// 单个请求符号的覆盖判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolCoverageStatus {
    Listed,
    Unlisted,
    Failed,
    Unsupported,
}

/// 一个请求符号及其覆盖判定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolCoverage {
    pub symbol: String,
    pub status: SymbolCoverageStatus,
}

/// 某交易所一次现货扇出抓取的结果，用于驱动覆盖判定。
#[derive(Debug, Clone, Copy)]
pub enum VenueListing<'a> {
    /// 抓取成功，附带本次返回的现货 tick 清单。
    Listed(&'a [SpotTick]),
    /// 抓取失败（网络/解析/限频），无法判断上架情况。
    Failed,
}

/// 某交易所一次现货扇出的完整覆盖规划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveragePlan {
    pub venue: String,
    pub entries: Vec<SymbolCoverage>,
}

impl CoveragePlan {
    pub fn count(&self, status: SymbolCoverageStatus) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.status == status)
            .count()
    }
}

/// 为 `venue` 的一次现货扇出生成逐符号覆盖规划。
///
/// 对 `requested` 中的每个符号都产出恰好一条记录（顺序保留、不去重、不丢弃），
/// 这样调用方可以审计为何某个请求符号没有出现在返回的 tick 中。
pub fn plan_symbol_coverage(
    venue: &str,
    requested: &[String],
    listing: VenueListing<'_>,
) -> CoveragePlan {
    let entries = requested
        .iter()
        .map(|symbol| SymbolCoverage {
            symbol: symbol.clone(),
            status: classify(symbol, listing),
        })
        .collect();
    CoveragePlan {
        venue: venue.to_owned(),
        entries,
    }
}

fn classify(symbol: &str, listing: VenueListing<'_>) -> SymbolCoverageStatus {
    let Some((base, quote)) = resolve_pair(symbol) else {
        // 形态无法解析出受支持的计价资产：即便抓取失败也归为 unsupported，
        // 因为重试也无法让该符号变得可用。
        return SymbolCoverageStatus::Unsupported;
    };
    match listing {
        VenueListing::Failed => SymbolCoverageStatus::Failed,
        VenueListing::Listed(ticks) => {
            if listing_contains(ticks, &base, &quote) {
                SymbolCoverageStatus::Listed
            } else {
                SymbolCoverageStatus::Unlisted
            }
        }
    }
}

fn listing_contains(ticks: &[SpotTick], base: &str, quote: &str) -> bool {
    let want = format!("{base}/{quote}");
    ticks
        .iter()
        .any(|tick| tick.symbol.eq_ignore_ascii_case(&want))
}

#[cfg(test)]
#[path = "spot_coverage_tests.rs"]
mod tests;
