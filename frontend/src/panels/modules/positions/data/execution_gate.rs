use crate::state::load_state::LoadState;
use shared_types::{ExecutionEnvironment, TradingStatusResponse};
use shared_types::{PositionOrigin, PositionRow};

pub(in crate::panels::modules::positions) fn close_selection_requires_live(
    row: &PositionRow,
    rows: &[PositionRow],
    paired: bool,
) -> Result<bool, &'static str> {
    let private = row.origin == PositionOrigin::AccountPrivate;
    if !paired {
        return Ok(private);
    }
    let pair = row.pair_evidence.as_ref().ok_or("当前仓位缺少配对数据依据")?;
    let same = |target: &PositionRow, venue: &str, symbol: &str, side| {
        shared_types::venue_names_equal(&target.venue, venue)
            && target.symbol.trim().eq_ignore_ascii_case(symbol.trim())
            && target.side == side
    };
    let mut matches = rows.iter().filter(|other| {
        same(
            other,
            &pair.partner_venue,
            &pair.partner_symbol,
            pair.partner_side,
        )
    });
    let partner = matches
        .next()
        .ok_or("另一条配对腿尚未读取，不能提交配对平仓")?;
    if matches.next().is_some() || same(partner, &row.venue, &row.symbol, row.side) {
        return Err("配对腿身份不唯一，不能提交配对平仓");
    }
    let reciprocal = partner.pair_evidence.as_ref().is_some_and(|other| {
        !pair.run_id.trim().is_empty()
            && other.run_id == pair.run_id
            && same(
                row,
                &other.partner_venue,
                &other.partner_symbol,
                other.partner_side,
            )
    });
    if !reciprocal {
        return Err("两边持仓的配对信息不一致，请等待更新后再确认");
    }
    Ok(private || partner.origin == PositionOrigin::AccountPrivate)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::panels::modules::positions) enum CloseExecutionGate {
    Unknown,
    Paper { adapter: String },
    Live { adapter: String, enabled: bool },
}

impl CloseExecutionGate {
    pub(in crate::panels::modules::positions) fn from_status(
        state: &LoadState<TradingStatusResponse>,
    ) -> Self {
        let LoadState::Ready(status) = state else {
            return Self::Unknown;
        };
        if status.adapter.trim().is_empty() {
            return Self::Unknown;
        }
        match status.environment {
            ExecutionEnvironment::Paper => Self::Paper {
                adapter: status.adapter.clone(),
            },
            ExecutionEnvironment::Live => Self::Live {
                adapter: status.adapter.clone(),
                enabled: status.risk.live_trading_enabled,
            },
        }
    }

    pub(in crate::panels::modules::positions) fn blocked_label(
        &self,
        requires_live: bool,
    ) -> Option<&'static str> {
        match self {
            Self::Unknown => Some("环境待确认"),
            Self::Paper { .. } if requires_live => Some("需实盘"),
            Self::Live { enabled: false, .. } => Some("实盘未启用"),
            _ => None,
        }
    }

    pub(in crate::panels::modules::positions) fn blocked_reason(
        &self,
        requires_live: bool,
    ) -> Option<&'static str> {
        match self {
            Self::Unknown => Some("执行环境状态未就绪或已过期，等待刷新后再确认平仓"),
            Self::Paper { .. } if requires_live => {
                Some("当前为模拟模式，不能关闭交易所真实仓位；请先在设置的执行环境中两步启用实盘")
            }
            Self::Live { enabled: false, .. } => {
                Some("当前实盘写入未启用，请先确认设置中的实盘开关")
            }
            _ => None,
        }
    }

    pub(in crate::panels::modules::positions) fn environment_label(&self) -> &'static str {
        match self {
            Self::Unknown => "环境待确认",
            Self::Paper { .. } => "模拟",
            Self::Live { .. } => "实盘",
        }
    }
}
