use super::*;

#[test]
fn stock_exchange_conversion_ui_original_plan_pending_fees_and_actual_credit() {
    let capture: serde_json::Value = std::env::var("STOCK_EXCHANGE_CONVERSION_CAPTURE_PATH")
        .ok()
        .map(|p| serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap())
        .unwrap_or_else(fixture);
    Owner::new().with(||{
        let d=super::super::super::data::PreflightData::fixture().conversion;
        let busy=Memo::new(|_|false);
        for state in ["ready","pending","missing","completed"] {
            let s:StockMarketSnapshot=serde_json::from_value(capture[state].clone()).unwrap();let p=s.exchange_conversions[0].clone();
            let html=row(p.clone(),d,busy,RwSignal::new(p.updated_at_ms)).to_html();
            assert!(html.contains("10 USDT"));assert!(html.contains("0.9997"));assert!(html.contains("原报价费后"));
            if state=="ready" {
                assert!(html.contains("确认本次 Backpack 实盘兑换"));assert!(!html.contains("checked"));assert!(html.contains("取消预留"));
                let expired=row(p.clone(),d,busy,RwSignal::new(p.terms.valid_until_ms)).to_html();assert!(expired.contains("报价已过期"));assert!(!expired.contains("提交兑换"));
            } else {
                assert!(!html.contains("提交兑换") &&!html.contains("取消预留"));
                if state=="completed" {assert!(html.contains("实际净入账"));assert!(html.contains("9.987003"));assert!(!html.contains("核对原订单"));}
                else {assert!(html.contains("核对原订单"));assert!(!html.contains("实际净入账"));}
            }
            if let Ok(root)=std::env::var("STOCK_EXCHANGE_CONVERSION_RENDER_DIR") {
                let path=std::path::Path::new(&root).join(format!("stocks-market-exchange-conversion-{state}.html"));
                super::super::tests::write_stock_html(path.to_str().unwrap(),&format!("<main class=\"stock-arbitrage-page stock-main\"><section class=\"stock-section stock-stablecoin\"><header><h3>Backpack 账户兑换</h3><span>USDT → USDC · 账户内</span></header>{html}</section></main>"));
            }
        }
    });
}
fn fixture() -> serde_json::Value {
    let now = 1000;
    let p:StockExchangeConversionPlan=serde_json::from_value(serde_json::json!({
        "planId":"local-exchange-conversion","request":{"requestId":"local-conversion-fixture","inputUsdt":"10","minimumUsdc":"9.98"},
        "terms":{"accountFingerprint":"local","market":{"symbol":"USDT_USDC","baseSymbol":"USDT","quoteSymbol":"USDC","marketType":"SPOT","orderBookState":"Open","minQuantity":"1","stepSize":"1","tickSize":"0.0001","checkedAtMs":now},
            "book":{"symbol":"USDT_USDC","bid":"0.9997","bidQuantity":"500","ask":"0.9998","askQuantity":"600","updateId":1,"sourceAtMs":now,"receivedAtMs":now},
            "availableUsdt":"20","balanceAtMs":now,"takerFeeBps":"10","feesAtMs":now,"feeBudgetUsdc":"0.009997","minimumNetUsdc":"9.987003",
            "instruction":{"kind":"order_book","clientId":1,"symbol":"USDT_USDC","side":"Ask","quantity":"10","limitPrice":"0.9997"},"createdAtMs":now,"validUntilMs":11000},
        "revision":1,"updatedAtMs":now,"cancelledAtMs":null,"order":null
    })).unwrap();
    let mut pending = p.clone();
    pending.revision = 2;
    pending.updated_at_ms = 1200;
    pending.order = Some(StockCexOrder::intent(1200));
    let mut missing = pending.clone();
    missing.revision = 3;
    let o = missing.order.as_mut().unwrap();
    o.phase = StockCexOrderPhase::Filled;
    o.order_id = Some("123".into());
    o.executed_quantity = Some("10".into());
    o.executed_quote_quantity = Some("9.997".into());
    o.fills.push(StockCexFill {
        trade_id: "456".into(),
        quantity: "10".into(),
        price: "0.9997".into(),
        fee: None,
    });
    let mut completed = missing.clone();
    completed.revision = 4;
    completed.order.as_mut().unwrap().fills[0].fee = Some(StockTradeFee {
        asset: "USDC".into(),
        quantity: "0.009997".into(),
    });
    serde_json::json!({"ready":StockMarketSnapshot{exchange_conversions:vec![p],..Default::default()},"pending":StockMarketSnapshot{exchange_conversions:vec![pending],..Default::default()},"missing":StockMarketSnapshot{exchange_conversions:vec![missing],..Default::default()},"completed":StockMarketSnapshot{exchange_conversions:vec![completed],..Default::default()}})
}
