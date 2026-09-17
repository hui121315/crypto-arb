use shared_types::{PositionRow, PositionSide};

/// 按方向计算**有符号**强平距离：多头 `(mark - liq)/mark`、空头 `(liq - mark)/mark`。
/// 负值表示 mark 已越过强平价——此前用 `abs()` 会把"已越界"折叠成一个小的
/// 正距离，看起来像还有安全边际，LiquidationGuard 与 severity 分级会低估风险。
pub fn annotate_liquidation_distance(rows: &mut [PositionRow]) {
    for row in rows {
        row.liquidation_distance_pct = match row.liquidation_price {
            Some(liq) if liq > 0.0 && row.mark_price > 0.0 => {
                let signed = match row.side {
                    PositionSide::Long => row.mark_price - liq,
                    PositionSide::Short => liq - row.mark_price,
                };
                Some((signed / row.mark_price) * 100.0)
            }
            _ => None,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::PositionSide;

    #[test]
    fn annotates_distance_when_prices_are_valid() {
        let mut rows = vec![PositionRow {
            venue: "OKX".into(),
            symbol: "BTC".into(),
            origin: Default::default(),
            side: PositionSide::Long,
            quantity: 1.0,
            entry_price: 100.0,
            mark_price: 100.0,
            leverage: 2.0,
            unrealized_pnl_usd: 0.0,
            liquidation_price: Some(80.0),
            liquidation_distance_pct: None,
            next_funding_ms: None,
            funding_rate_8h: 0.0,
            funding_rate_verified: true,
            maintenance_margin_ratio: 0.05,
            pair_evidence: None,
            paired_with: None,
            margin_usd: 50.0,
            severity: shared_types::PositionSeverity::Ok,
            seconds_until_funding: None,
        }];

        annotate_liquidation_distance(&mut rows);

        assert_eq!(rows[0].liquidation_distance_pct, Some(20.0));

        // 多头 mark 已跌破强平价：距离为负（已越界），不得被 abs 折叠成正边际。
        rows[0].mark_price = 75.0;
        annotate_liquidation_distance(&mut rows);
        let breached = rows[0].liquidation_distance_pct.expect("distance");
        assert!(breached < 0.0, "breached long must be negative: {breached}");

        // 空头方向相反：liq 100，mark 80 → 正距离 25%。
        rows[0].side = PositionSide::Short;
        rows[0].mark_price = 80.0;
        rows[0].liquidation_price = Some(100.0);
        annotate_liquidation_distance(&mut rows);
        assert_eq!(rows[0].liquidation_distance_pct, Some(25.0));
    }
}
