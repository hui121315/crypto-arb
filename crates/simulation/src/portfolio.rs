//! 模拟投资组合：现金 + 当前仓位 + 历史交易。

use crate::types::{ClosedTrade, OpenRequest, PositionSide, SimPosition};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum SimError {
    #[error("position not found: {0}")]
    NotFound(String),
    #[error("insufficient cash: needed {needed}, have {available}")]
    InsufficientCash { needed: f64, available: f64 },
    #[error("invalid quantity: {0}")]
    InvalidQuantity(f64),
    #[error("invalid price: {0}")]
    InvalidPrice(f64),
    #[error("invalid leverage: {0}")]
    InvalidLeverage(f64),
}

pub type SimResult<T> = Result<T, SimError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimPortfolio {
    pub initial_capital: f64,
    pub cash: f64,
    /// 当前持仓，按 position id 索引。
    pub positions: BTreeMap<String, SimPosition>,
    /// 已平仓历史。
    pub history: Vec<ClosedTrade>,
}

impl SimPortfolio {
    pub fn new(initial_capital: f64) -> Self {
        Self {
            initial_capital,
            cash: initial_capital,
            positions: BTreeMap::new(),
            history: Vec::new(),
        }
    }

    /// 开仓。扣除保证金 + 手续费。
    ///
    /// 保证金 = 名义价值 / leverage。
    pub fn open(&mut self, req: OpenRequest) -> SimResult<SimPosition> {
        if req.quantity <= 0.0 {
            return Err(SimError::InvalidQuantity(req.quantity));
        }
        if req.entry_price <= 0.0 {
            return Err(SimError::InvalidPrice(req.entry_price));
        }
        if !req.leverage.is_finite() || req.leverage < 1.0 {
            return Err(SimError::InvalidLeverage(req.leverage));
        }
        let margin = req.entry_price * req.quantity / req.leverage;
        let total_cost = margin + req.fees;
        if total_cost > self.cash + 1e-9 {
            return Err(SimError::InsufficientCash {
                needed: total_cost,
                available: self.cash,
            });
        }
        self.cash -= total_cost;

        let id = Uuid::new_v4().simple().to_string();
        let position = SimPosition {
            id: id.clone(),
            symbol: req.symbol,
            exchange: req.exchange,
            side: req.side,
            quantity: req.quantity,
            entry_price: req.entry_price,
            current_price: req.entry_price,
            leverage: req.leverage,
            opened_at: Utc::now(),
            fees_paid: req.fees,
            note: req.note,
        };
        self.positions.insert(id, position.clone());
        Ok(position)
    }

    /// 平仓。返还保证金 + PnL - 手续费。
    pub fn close(&mut self, id: &str, close_price: f64, fees: f64) -> SimResult<ClosedTrade> {
        if close_price <= 0.0 {
            return Err(SimError::InvalidPrice(close_price));
        }
        let mut position = self
            .positions
            .remove(id)
            .ok_or_else(|| SimError::NotFound(id.into()))?;
        position.current_price = close_price;
        let pnl_gross = match position.side {
            PositionSide::Long => (close_price - position.entry_price) * position.quantity,
            PositionSide::Short => (position.entry_price - close_price) * position.quantity,
        };
        let realized = pnl_gross - position.fees_paid - fees;

        let margin_returned = position.margin_used();
        self.cash += margin_returned + realized;
        // realized 已含开仓 + 平仓手续费；margin_returned 返还入场保证金

        let trade = ClosedTrade {
            position,
            close_price,
            closed_at: Utc::now(),
            realized_pnl: realized,
            close_fees: fees,
        };
        self.history.push(trade.clone());
        Ok(trade)
    }

    /// 用 `{symbol: price}` 字典批量更新所有持仓的 `current_price`。
    /// 缺失 symbol 的仓位保持原值。
    pub fn update_marks(&mut self, prices: &HashMap<String, f64>) {
        for pos in self.positions.values_mut() {
            if let Some(&p) = prices.get(&pos.symbol) {
                if p > 0.0 {
                    pos.current_price = p;
                }
            }
        }
    }

    /// 给定单个 symbol 的最新价格，单独更新该 symbol 所有仓位。
    pub fn update_mark(&mut self, symbol: &str, price: f64) {
        if price <= 0.0 {
            return;
        }
        for pos in self.positions.values_mut() {
            if pos.symbol == symbol {
                pos.current_price = price;
            }
        }
    }

    /// 所有持仓的未实现盈亏总和。
    pub fn total_unrealized_pnl(&self) -> f64 {
        self.positions.values().map(|p| p.unrealized_pnl()).sum()
    }

    fn total_mark_pnl(&self) -> f64 {
        self.positions.values().map(SimPosition::mark_pnl).sum()
    }

    /// 所有已平仓交易的实现盈亏总和。
    pub fn total_realized_pnl(&self) -> f64 {
        self.history.iter().map(|t| t.realized_pnl).sum()
    }

    /// 当前持仓名义价值总和（按 `current_price`）。
    pub fn total_positions_value(&self) -> f64 {
        self.positions
            .values()
            .map(SimPosition::notional_value)
            .sum()
    }

    /// 当前仓位占用保证金总和。
    pub fn total_margin_used(&self) -> f64 {
        self.positions.values().map(SimPosition::margin_used).sum()
    }

    /// 账户总权益 = 可用现金 + 持仓保证金 + 未实现盈亏。
    pub fn total_equity(&self) -> f64 {
        self.cash + self.total_margin_used() + self.total_mark_pnl()
    }

    /// 当前回报率（相对初始资本）。
    pub fn return_pct(&self) -> f64 {
        if self.initial_capital <= 0.0 {
            return 0.0;
        }
        (self.total_equity() - self.initial_capital) / self.initial_capital
    }

    pub fn position_count(&self) -> usize {
        self.positions.len()
    }

    pub fn trade_count(&self) -> usize {
        self.history.len()
    }
}

#[cfg(test)]
mod tests;
