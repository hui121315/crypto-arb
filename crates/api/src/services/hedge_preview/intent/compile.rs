use shared_types::{
    venue_family, ExecutionMode, FeeProduct, OrderIntent, OrderPayloadPricePolicy, OrderType,
    TimeInForce, VenueMarketOrderStyle, VenueOrderKind,
};

pub(super) struct CompileSpec {
    pub(super) effective_order_type: OrderType,
    pub(super) effective_time_in_force: TimeInForce,
    pub(super) venue_order_kind: VenueOrderKind,
    pub(super) payload_price_policy: OrderPayloadPricePolicy,
    pub(super) summary: String,
    pub(super) blockers: Vec<String>,
}

impl CompileSpec {
    pub(super) fn payload_price(&self, protection_price: Option<f64>) -> Option<f64> {
        match self.payload_price_policy {
            OrderPayloadPricePolicy::LimitPrice | OrderPayloadPricePolicy::ProtectionPrice => {
                protection_price
            }
            OrderPayloadPricePolicy::ZeroPrice => Some(0.0),
            OrderPayloadPricePolicy::Omit | OrderPayloadPricePolicy::MarketLikeNoPrice => None,
        }
    }
}

pub(super) fn compile_spec(
    intent: &OrderIntent,
    market_order_style: Option<VenueMarketOrderStyle>,
    product: FeeProduct,
) -> CompileSpec {
    let mut spec = match intent.order_type {
        OrderType::Limit => compile_limit_spec(intent),
        OrderType::PostOnly => compile_post_only_spec(intent),
        OrderType::Market => compile_market_spec(intent, market_order_style, product),
    };
    append_kucoin_live_position_side_blocker(intent, &mut spec);
    spec
}

pub(super) fn compiler_time_in_force_options(
    intent: &OrderIntent,
    product: FeeProduct,
) -> Vec<TimeInForce> {
    exchange::static_venue_capability_matrix_for_product(&intent.exchange, product)
        .and_then(|matrix| {
            matrix
                .order(intent.order_type)
                .map(|capability| capability.time_in_force.clone())
        })
        .unwrap_or_default()
}

fn compile_limit_spec(intent: &OrderIntent) -> CompileSpec {
    if venue_family(&intent.exchange) == "gate" && matches!(intent.time_in_force, TimeInForce::Gtx)
    {
        return CompileSpec {
            effective_order_type: OrderType::Limit,
            effective_time_in_force: intent.time_in_force,
            venue_order_kind: VenueOrderKind::Limit,
            payload_price_policy: OrderPayloadPricePolicy::LimitPrice,
            summary: "Gate futures 限价单不支持 GTX；post-only 需按官方 poc 编译".into(),
            blockers: vec![
                "Gate futures 官方 TIF 为 gtc/ioc/fok/poc；GTX 需切换为 Post-only".into(),
            ],
        };
    }

    CompileSpec {
        effective_order_type: OrderType::Limit,
        effective_time_in_force: intent.time_in_force,
        venue_order_kind: VenueOrderKind::Limit,
        payload_price_policy: OrderPayloadPricePolicy::LimitPrice,
        summary: "限价单，payload 使用票据保护价".into(),
        blockers: Vec::new(),
    }
}

fn compile_post_only_spec(intent: &OrderIntent) -> CompileSpec {
    CompileSpec {
        effective_order_type: OrderType::PostOnly,
        effective_time_in_force: intent.time_in_force,
        venue_order_kind: VenueOrderKind::PostOnly,
        payload_price_policy: OrderPayloadPricePolicy::LimitPrice,
        summary: "Post-only 限价单，payload 使用票据保护价".into(),
        blockers: Vec::new(),
    }
}

fn compile_market_spec(
    intent: &OrderIntent,
    _market_order_style: Option<VenueMarketOrderStyle>,
    product: FeeProduct,
) -> CompileSpec {
    match venue_family(&intent.exchange) {
        "hyperliquid" => CompileSpec {
            effective_order_type: OrderType::Limit,
            effective_time_in_force: TimeInForce::Ioc,
            venue_order_kind: VenueOrderKind::ProtectedIoc,
            payload_price_policy: OrderPayloadPricePolicy::ProtectionPrice,
            summary: "Hyperliquid 市价按官方 protected IOC limit 编译，payload 保留保护价".into(),
            blockers: Vec::new(),
        },
        "gate" if product == FeeProduct::Perp => CompileSpec {
            effective_order_type: OrderType::Market,
            effective_time_in_force: TimeInForce::Ioc,
            venue_order_kind: VenueOrderKind::PriceZeroIoc,
            payload_price_policy: OrderPayloadPricePolicy::ZeroPrice,
            summary: "Gate futures 市价按官方 price=0 + IOC 编译".into(),
            blockers: Vec::new(),
        },
        "bybit" => compile_bybit_market_spec(intent),
        "binance" | "okx" | "bitget" | "gate_crossex" | "kraken" | "kucoin" => CompileSpec {
            effective_order_type: OrderType::Market,
            effective_time_in_force: intent.time_in_force,
            venue_order_kind: VenueOrderKind::NativeMarket,
            payload_price_policy: OrderPayloadPricePolicy::Omit,
            summary: "原生市价单，payload 不发送价格字段".into(),
            blockers: Vec::new(),
        },
        _ => CompileSpec {
            effective_order_type: OrderType::Market,
            effective_time_in_force: intent.time_in_force,
            venue_order_kind: VenueOrderKind::MarketLikeRequired,
            payload_price_policy: OrderPayloadPricePolicy::MarketLikeNoPrice,
            summary: "未知 venue 市价语义，需官方文档确认后编译".into(),
            blockers: vec!["未知交易所市价 payload 语义，禁止按全局默认值实盘".into()],
        },
    }
}

fn append_kucoin_live_position_side_blocker(intent: &OrderIntent, spec: &mut CompileSpec) {
    if intent.mode != ExecutionMode::Live || venue_family(&intent.exchange) != "kucoin" {
        return;
    }
    if intent.reduce_only {
        spec.blockers.push(
            "KuCoin hedge-mode reduce-only 需要明确目标仓位 side；未接当前持仓 positionSide 证据前禁止提交"
                .into(),
        );
    }
}

pub(super) fn order_plan_blockers(
    mut blockers: Vec<String>,
    policy: &shared_types::ClientOrderIdPolicy,
) -> Vec<String> {
    blockers.extend(policy.blockers.iter().cloned());
    blockers
}

fn compile_bybit_market_spec(intent: &OrderIntent) -> CompileSpec {
    match bybit_market_slippage_percent(intent.slippage_tolerance_bps) {
        Ok(percent) => CompileSpec {
            effective_order_type: OrderType::Market,
            effective_time_in_force: TimeInForce::Ioc,
            venue_order_kind: VenueOrderKind::NativeMarket,
            payload_price_policy: OrderPayloadPricePolicy::Omit,
            summary: format!("Bybit 市价按官方 Percent slippageTolerance={percent} 编译"),
            blockers: Vec::new(),
        },
        Err(reason) => CompileSpec {
            effective_order_type: OrderType::Market,
            effective_time_in_force: TimeInForce::Ioc,
            venue_order_kind: VenueOrderKind::NativeMarket,
            payload_price_policy: OrderPayloadPricePolicy::Omit,
            summary: "Bybit 市价需携带官方 slippageToleranceType/slippageTolerance 证据后再实盘"
                .into(),
            blockers: vec![reason],
        },
    }
}

fn bybit_market_slippage_percent(value: Option<f64>) -> Result<String, String> {
    let bps = value.ok_or_else(|| {
        "Bybit 市价单缺官方 slippageTolerance 证据；请先设置正数滑点容忍度".to_owned()
    })?;
    if !bps.is_finite() || !(1.0..=1_000.0).contains(&bps) {
        return Err("Bybit 市价单官方 Percent slippageTolerance 只支持 1..=1000 bps".into());
    }
    if (bps.fract()).abs() > 1e-9 {
        return Err(
            "Bybit 市价单官方 Percent slippageTolerance 最多两位小数，内部需使用整数 bps".into(),
        );
    }
    Ok(format_bybit_percent_slippage(bps))
}

fn format_bybit_percent_slippage(bps: f64) -> String {
    let text = format!("{:.2}", bps / 100.0);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ExecutionMode, MarginMode, OrderSide, OrderSource};

    fn market_intent(exchange: &str) -> OrderIntent {
        OrderIntent {
            id: "intent-1".to_owned(),
            source: OrderSource::ArbitragePreview,
            strategy: None,
            mode: ExecutionMode::Live,
            exchange: exchange.to_owned(),
            symbol: "BTC".to_owned(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            quantity: 0.01,
            price: Some(64_000.0),
            slippage_tolerance_bps: Some(10.0),
            reduce_only: false,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "xl-order-1".to_owned(),
            client_order_id_policy: None,
            created_at_ms: 1,
        }
    }

    #[test]
    fn kraken_and_crossex_use_their_verified_native_market_compilers() {
        for exchange in ["kraken", "gate_crossex:okx"] {
            let spec = compile_market_spec(&market_intent(exchange), None, FeeProduct::Perp);
            assert!(spec.blockers.is_empty(), "{exchange}: {:?}", spec.blockers);
            assert_eq!(spec.venue_order_kind, VenueOrderKind::NativeMarket);
            assert_eq!(spec.payload_price_policy, OrderPayloadPricePolicy::Omit);
        }
    }
}
