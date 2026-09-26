use super::*;

mod fixtures;
mod connection;
use fixtures::{preview, seed_spot_book, seed_spot_book_with_quantity, test_state};

#[tokio::test]
async fn dry_run_confirmation_refreshes_selected_books_and_fills_both_legs() -> anyhow::Result<()> {
    let state = test_state().await?;
    let now_ms = common::time::now_ms();
    seed_spot_book(&state, "BTC-LONG", 100.0, now_ms);
    seed_spot_book(&state, "BTC-SHORT", 101.0, now_ms);

    let mut preview = preview(now_ms)?;
    assert!(
        crate::services::hedge_recheck::pre_submit_rejection(&state, &mut preview)
            .await
            .is_none()
    );

    preview.execution_binding = Some(crate::services::hedge_preview::runtime::capture(&state).await);
    let engine = crate::services::hedge_preview::runtime::bind_engine(&state, &preview).await?;
    let response = confirm_preview(&state, preview, "dry-run-pair".to_owned(), &engine).await;
    let run = response
        .execution_run
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("execution run missing"))?;
    let long = response
        .long_record
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("long record missing"))?;
    let short = response
        .short_record
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("short record missing"))?;

    assert_eq!(
        response.status,
        HedgeConfirmStatus::Submitted,
        "status={:?}; error={:?}; problem={:?}; partial={:?}",
        response.status,
        response.error,
        response.problem,
        response.partial_outcome
    );
    assert_eq!(run.state, ExecutionRunState::Hedged);
    assert_eq!(long.state, LiveOrderState::Filled);
    assert_eq!(short.state, LiveOrderState::Filled);
    assert_eq!(long.intent.mode, ExecutionMode::DryRun);
    assert_eq!(short.intent.mode, ExecutionMode::DryRun);
    assert_eq!(long.intent.exchange, "mock");
    assert_eq!(short.intent.exchange, "mock");
    assert!(run.net_exposure_usd.abs() < 1e-9);
    Ok(())
}

#[tokio::test]
async fn dry_run_market_confirmation_binds_one_refreshed_ticket_fact() -> anyhow::Result<()> {
    let state = test_state().await?;
    let now_ms = common::time::now_ms();
    seed_spot_book(&state, "BTC-LONG", 99.5, now_ms);
    seed_spot_book(&state, "BTC-SHORT", 101.5, now_ms);

    let mut preview = preview(now_ms)?;
    preview.long_leg.order_type = OrderType::Market;
    preview.short_leg.order_type = OrderType::Market;
    let plans = preview
        .ticket_order_plans
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("ticket plans missing"))?;
    for plan in [&mut plans.long.compile_plan, &mut plans.short.compile_plan] {
        plan.requested_order_type = OrderType::Market;
        plan.effective_order_type = OrderType::Market;
        plan.venue_order_kind = shared_types::VenueOrderKind::NativeMarket;
        plan.payload_price_policy = shared_types::OrderPayloadPricePolicy::Omit;
        plan.payload_price = None;
    }

    assert!(
        crate::services::hedge_recheck::pre_submit_rejection(&state, &mut preview)
            .await
            .is_none()
    );
    assert_eq!(preview.long_leg.price, Some(99.5));
    assert_eq!(preview.short_leg.price, Some(101.5));
    assert!(preview.ticket.market_checked_at_ms >= now_ms);
    assert!(preview.ticket.guards.iter().any(|guard| {
        guard.key == "profit_lock" && guard.passed && guard.detail.contains("class=locked")
    }));

    preview.execution_binding = Some(crate::services::hedge_preview::runtime::capture(&state).await);
    let engine = crate::services::hedge_preview::runtime::bind_engine(&state, &preview).await?;
    let response = confirm_preview(&state, preview, "dry-run-market-pair".to_owned(), &engine).await;
    let long = response
        .long_record
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("long record missing"))?;
    let short = response
        .short_record
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("short record missing"))?;

    assert_eq!(response.status, HedgeConfirmStatus::Submitted);
    assert_eq!(long.filled_price, Some(99.5));
    assert_eq!(short.filled_price, Some(101.5));
    Ok(())
}

#[tokio::test]
async fn dry_run_short_first_uses_only_long_book_for_second_leg_refresh() -> anyhow::Result<()> {
    let state = test_state().await?;
    let now_ms = common::time::now_ms();
    seed_spot_book(&state, "BTC-LONG", 100.0, now_ms);
    seed_spot_book_with_quantity(&state, "BTC-SHORT", 101.0, 1.0, now_ms);

    let mut preview = preview(now_ms)?;
    assert!(
        crate::services::hedge_recheck::pre_submit_rejection(&state, &mut preview)
            .await
            .is_none()
    );
    assert_eq!(
        crate::services::hedge_ticket::execution_order(&preview.ticket).first,
        HedgeLegRole::Short
    );
    seed_spot_book_with_quantity(&state, "BTC-SHORT", 101.0, 1.0, now_ms - 31_000);

    preview.execution_binding = Some(crate::services::hedge_preview::runtime::capture(&state).await);
    let engine = crate::services::hedge_preview::runtime::bind_engine(&state, &preview).await?;
    let response = confirm_preview(&state, preview, "dry-run-short-first".to_owned(), &engine).await;
    let run = response
        .execution_run
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("execution run missing"))?;

    assert_eq!(
        response.status,
        HedgeConfirmStatus::Submitted,
        "status={:?}; error={:?}; problem={:?}; partial={:?}",
        response.status,
        response.error,
        response.problem,
        response.partial_outcome
    );
    assert_eq!(run.state, ExecutionRunState::Hedged);
    assert_eq!(
        response
            .short_record
            .as_ref()
            .map(|record| record.intent.id.as_str()),
        Some("dry-run-short")
    );
    assert_eq!(
        response
            .long_record
            .as_ref()
            .map(|record| record.intent.id.as_str()),
        Some("dry-run-long")
    );
    Ok(())
}
