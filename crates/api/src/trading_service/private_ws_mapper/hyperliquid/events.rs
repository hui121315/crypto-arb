use super::*;

pub(super) fn hyperliquid_fill_event(row: hyperliquid_ws_user::HyperliquidFill) -> PrivateWsEvent {
    let transport_metadata =
        OrderTransportMetadata::default().with_venue_fill_evidence(VenueFillTransportEvidence {
            venue_closed_pnl: decimal_evidence(row.closed_pnl),
            liquidation: row.liquidation.as_ref().map(|liquidation| {
                VenueLiquidationTransportEvidence {
                    liquidated_user: liquidation.liquidated_user.clone(),
                    mark_price: decimal_evidence(liquidation.mark_price),
                    method: match liquidation.method {
                        hyperliquid_ws_user::HyperliquidLiquidationMethod::Market => {
                            VenueLiquidationMethod::Market
                        }
                        hyperliquid_ws_user::HyperliquidLiquidationMethod::Backstop => {
                            VenueLiquidationMethod::Backstop
                        }
                    },
                }
            }),
        });
    PrivateWsEvent::FillWithEvidence(Box::new(PrivateFillWithEvidenceDelta {
        fill: hyperliquid_fill_delta(row),
        transport_metadata,
    }))
}

fn decimal_evidence(value: f64) -> String {
    let text = format!("{value:.12}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn hyperliquid_fill_delta(row: hyperliquid_ws_user::HyperliquidFill) -> PrivateFillDelta {
    let venue_event_id = hyperliquid_fill_event_id(&row);
    PrivateFillDelta {
        venue: row.venue.clone(),
        exchange_order_id: row.order_id,
        client_order_id: None,
        symbol: non_empty_text(&row.coin),
        side: order_side_from_text(&row.side),
        venue_event_id,
        quantity: row.size,
        price: row.price,
        fee_amount: Some(row.fee),
        fee_currency: Some(row.fee_token),
        occurred_at_ms: row.time_ms,
    }
}

fn hyperliquid_fill_event_id(row: &hyperliquid_ws_user::HyperliquidFill) -> String {
    if let Some(trade_id) = row.trade_id {
        return format!(
            "hyperliquid_fill:{}:{}:{}:{}:{}",
            row.venue, row.order_id, row.coin, row.time_ms, trade_id
        );
    }
    let hash = row.tx_hash.trim();
    if hash.is_empty() {
        format!(
            "hyperliquid_fill:{}:{}:{}:{:.12}:{:.12}:{:.12}",
            row.venue, row.order_id, row.time_ms, row.size, row.price, row.fee
        )
    } else {
        format!("hyperliquid_fill:{}:{}:{hash}", row.venue, row.order_id)
    }
}

pub(super) fn hyperliquid_funding_delta(
    row: hyperliquid_ws_user::HyperliquidFunding,
) -> PrivateFundingDelta {
    let venue_event_id = format!(
        "hyperliquid_funding:{}:{}:{}",
        row.venue, row.coin, row.time_ms
    );
    PrivateFundingDelta {
        venue: row.venue,
        venue_event_id,
        coin: row.coin,
        amount: row.usdc,
        currency: "USDC".to_owned(),
        occurred_at_ms: row.time_ms,
    }
}

pub(super) fn hyperliquid_liquidation_delta(
    row: hyperliquid_ws_user::HyperliquidLiquidation,
) -> PrivateLiquidationDelta {
    PrivateLiquidationDelta {
        venue: "hyperliquid".to_owned(),
        venue_event_id: format!("hyperliquid_liquidation:{}", row.id),
        liquidator: row.liquidator,
        liquidated_user: row.liquidated_user,
        notional_position: row.notional_position,
        account_value: row.account_value,
        occurred_at_ms: common::time::now_ms(),
    }
}

pub(super) fn hyperliquid_non_user_cancel_delta(
    row: hyperliquid_ws_user::HyperliquidNonUserCancel,
) -> PrivateNonUserCancelDelta {
    let venue_event_id = format!(
        "hyperliquid_non_user_cancel:{}:{}:{}",
        row.venue, row.coin, row.order_id
    );
    PrivateNonUserCancelDelta {
        venue: row.venue,
        venue_event_id,
        exchange_order_id: row.order_id,
        coin: row.coin,
        occurred_at_ms: common::time::now_ms(),
    }
}
