use super::*;
use rust_decimal::Decimal;

pub const STOCK_BATCH_LIMIT: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockBatchRequest {
    pub enabled: bool,
    pub assets: Vec<String>,
    pub budget_usdc: String,
    pub keyed: bool,
    pub interval_secs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockBatchUpdateRequest {
    pub expected_revision: String,
    pub request: StockBatchRequest,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockBatchStatus {
    #[serde(default)]
    pub revision: String,
    pub request: Option<StockBatchRequest>,
    pub running: bool,
    pub waiting_for_viewers: bool,
    pub rows: Vec<StockBatchRow>,
    pub problem: Option<String>,
    pub completed_rounds: u64,
    pub round_started_at_ms: Option<i64>,
    pub last_round_elapsed_ms: Option<u64>,
    pub next_at_ms: Option<i64>,
    pub metadata_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockBatchRow {
    pub security: StockSecurity,
    pub token: Option<StockChainToken>,
    pub issuer_verified: bool,
    pub mint: Option<StockMintEvidence>,
    pub buy: Option<StockDexQuote>,
    pub sell: Option<StockDexQuote>,
    pub books: Vec<StockBookQuote>,
    pub connected: bool,
    pub refreshing: bool,
    pub problem: Option<String>,
    pub checked_at_ms: Option<i64>,
}

impl StockBatchRow {
    /// Only a per-token observation price. This is not a share price or arbitrage profit.
    pub fn token_price(&self, buy: bool, now: i64) -> Option<String> {
        let mint = self.mint.as_ref()?;
        let q = if buy {
            self.buy.as_ref()?
        } else {
            self.sell.as_ref()?
        };
        let token = self.token.as_ref()?;
        if token.contract_address.as_deref() != Some(&mint.address)
            || token.native_decimals != Some(mint.decimals)
            || (buy && (q.input_mint != comparison::SOLANA_USDC || q.output_mint != mint.address))
            || (!buy && (q.input_mint != mint.address || q.output_mint != comparison::SOLANA_USDC))
        {
            return None;
        }
        if !comparison::quote_current(q, now)
            || now < mint.checked_at_ms
            || now - mint.checked_at_ms > 60_000
            || mint.next_change_at_ms.is_some_and(|at| now >= at)
        {
            return None;
        }
        let (usdc, tokens) = if buy {
            (&q.input_raw, &q.minimum_output_raw)
        } else {
            (&q.minimum_output_raw, &q.input_raw)
        };
        let cash = Decimal::from_str_exact(&stock_chain_quantity(usdc, 6)?).ok()?;
        let tokens = Decimal::from_str_exact(&stock_chain_quantity(tokens, mint.decimals)?).ok()?;
        if cash <= Decimal::ZERO || tokens <= Decimal::ZERO {
            return None;
        }
        cash.checked_div(tokens).map(|n| n.normalize().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> StockBatchRow {
        let token: StockChainToken = serde_json::from_value(serde_json::json!({
            "blockchain":"Solana", "contractAddress":"test-mint", "nativeDecimals":6
        }))
        .unwrap();
        let buy = StockDexQuote {
            input_mint: comparison::SOLANA_USDC.into(),
            output_mint: "test-mint".into(),
            input_raw: "100000000".into(),
            output_raw: "2100000".into(),
            minimum_output_raw: "2000000".into(),
            router: "fixture".into(),
            fee_bps: None,
            fee_mint: None,
            requested_at_ms: 1000,
            received_at_ms: 1100,
            expires_at_ms: None,
        };
        let sell = StockDexQuote {
            input_mint: "test-mint".into(),
            output_mint: comparison::SOLANA_USDC.into(),
            input_raw: "2000000".into(),
            output_raw: "97000000".into(),
            minimum_output_raw: "96000000".into(),
            ..buy.clone()
        };
        StockBatchRow {
            security: StockSecurity {
                asset: "TEST.US".into(),
                ticker: "TEST".into(),
                name: "Test".into(),
                cusip: None,
                sessions: vec![],
                order_books: vec![],
                rfq_symbol: "TEST.US_USDC_RFQ".into(),
            },
            token: Some(token),
            issuer_verified: false,
            mint: Some(StockMintEvidence {
                address: "test-mint".into(),
                decimals: 6,
                ui_multiplier: "2".into(),
                slot: 1,
                chain_time_ms: 1000,
                checked_at_ms: 1000,
                next_change_at_ms: None,
                extensions: vec![],
            }),
            buy: Some(buy),
            sell: Some(sell),
            books: vec![],
            connected: false,
            refreshing: false,
            problem: None,
            checked_at_ms: Some(1100),
        }
    }

    #[test]
    fn batch_observation_prices_use_minimum_output_and_raw_token_not_shares() {
        let r = row();
        assert_eq!(r.token_price(true, 1200).as_deref(), Some("50"));
        assert_eq!(r.token_price(false, 1200).as_deref(), Some("48"));
        assert!(!r.issuer_verified);
        assert!(r.token_price(true, 11_001).is_none());
        assert!(r.token_price(false, 999).is_none());
        let mut changed = r.clone();
        changed.mint.as_mut().unwrap().next_change_at_ms = Some(1200);
        assert!(changed.token_price(true, 1200).is_none());
        changed = r.clone();
        changed.token.as_mut().unwrap().native_decimals = Some(9);
        assert!(changed.token_price(true, 1200).is_none());
        changed = r.clone();
        changed.buy.as_mut().unwrap().output_mint = "other".into();
        assert!(changed.token_price(true, 1200).is_none());
        changed = r.clone();
        changed.sell.as_mut().unwrap().minimum_output_raw = "0".into();
        assert!(changed.token_price(false, 1200).is_none());
    }
}
