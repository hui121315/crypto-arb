use super::*;

pub const STOCK_WRAPPED_SOL: &str = "So11111111111111111111111111111111111111112";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockNativeValuation {
    pub native_lamports: String,
    pub quote: StockDexQuote,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replenishment: Option<StockNativeReplenishment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockNativeReplenishment {
    pub wallet_address: String,
    pub transaction: crate::OnchainUnsignedTransaction,
    pub transaction_fingerprint: String,
    pub network_fee_lamports: String,
    /// All wallet-funded native outflows; refunds are not credited against this bound.
    pub wallet_outflow_lamports: String,
    pub wallet_required_lamports: String,
    pub minimum_credit_lamports: String,
    pub simulation_slot: u64,
    pub checked_at_ms: i64,
    pub valid_until_ms: i64,
}

impl StockNativeValuation {
    pub fn execution_cost(&self, context: &StockChainCost) -> Result<StockChainCost, String> {
        let proof = self
            .replenishment
            .as_ref()
            .ok_or("SOL 补回缺少模拟与原始交易")?;
        let mut cost = context.clone();
        cost.quote = self.quote.clone();
        cost.transaction = Some(proof.transaction.clone());
        cost.transaction_fingerprint = proof.transaction_fingerprint.clone();
        cost.checked_at_ms = proof.checked_at_ms;
        cost.valid_until_ms = proof.valid_until_ms;
        cost.simulation_slot = Some(proof.simulation_slot);
        cost.network_fee_lamports = Some(proof.network_fee_lamports.clone());
        cost.wallet_debit_lamports = Some(proof.wallet_outflow_lamports.clone());
        cost.wallet_budget_lamports = Some(proof.wallet_outflow_lamports.clone());
        cost.wallet_required_lamports = Some(proof.wallet_required_lamports.clone());
        cost.native_valuation = None;
        cost.provider_fees.clear();
        cost.problems.clear();
        cost.simulation_passed = true;
        Ok(cost)
    }

    pub fn usdc_budget(&self, required: &str, now: i64) -> Option<String> {
        let required = required.parse::<u64>().ok().filter(|n| *n > 0)?;
        let target = self.native_lamports.parse::<u64>().ok()?;
        let received = self.quote.minimum_output_raw.parse::<u64>().ok()?;
        let expected = self.quote.output_raw.parse::<u64>().ok()?;
        let input = self
            .quote
            .input_raw
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0)?;
        if target != required
            || received < required
            || received > expected
            || self.quote.input_mint != comparison::SOLANA_USDC
            || self.quote.output_mint != STOCK_WRAPPED_SOL
            || self.quote.received_at_ms < self.quote.requested_at_ms
            || now < self.quote.received_at_ms
            || !comparison::quote_current(&self.quote, now)
        {
            return None;
        }
        Some(
            (rust_decimal::Decimal::from(input) / rust_decimal::Decimal::from(1_000_000))
                .normalize()
                .to_string(),
        )
    }

    pub fn complete_budget(&self, required: &str, wallet: &str, now: i64) -> Option<String> {
        let budget = self.usdc_budget(required, now)?;
        let proof = self.replenishment.as_ref()?;
        let minimum = self.quote.minimum_output_raw.parse::<u64>().ok()?;
        let outflow = proof.wallet_outflow_lamports.parse::<u64>().ok()?;
        let credit = proof.minimum_credit_lamports.parse::<u64>().ok()?;
        let required = required.parse::<u64>().ok()?;
        let funding = proof.wallet_required_lamports.parse::<u64>().ok()?;
        proof.network_fee_lamports.parse::<u64>().ok()?;
        let crate::OnchainUnsignedTransaction::SolanaVersioned {
            transaction_base64,
            request_id,
            router,
            expire_at_ms,
            ..
        } = &proof.transaction
        else {
            return None;
        };
        if proof.wallet_address != wallet
            || proof.transaction_fingerprint.is_empty()
            || transaction_base64.is_empty()
            || request_id.is_empty()
            || router != &self.quote.router
            || expire_at_ms != &self.quote.expires_at_ms
            || proof.simulation_slot == 0
            || proof.checked_at_ms < self.quote.received_at_ms
            || now < proof.checked_at_ms
            || now >= proof.valid_until_ms
            || proof.valid_until_ms
                > self
                    .quote
                    .requested_at_ms
                    .saturating_add(comparison::STOCK_QUOTE_MAX_AGE_MS)
            || self
                .quote
                .expires_at_ms
                .is_some_and(|t| proof.valid_until_ms > t)
            || funding < outflow
            || minimum.checked_sub(outflow) != Some(credit)
            || credit < required
        {
            return None;
        }
        Some(budget)
    }
}

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockChainDirection {
    Buy,
    Sell,
}

impl StockChainDirection {
    pub fn quote(self, comparison: &StockComparison) -> Option<&StockDexQuote> {
        match self {
            Self::Buy => Some(&comparison.buy),
            Self::Sell => comparison.sell.as_ref(),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Buy => "链买 / Backpack 卖",
            Self::Sell => "Backpack 买 / 链卖",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockChainCostRequest {
    pub asset: String,
    pub wallet_address: String,
    pub direction: StockChainDirection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockNativeFee {
    pub kind: String,
    pub lamports: Option<String>,
    pub payer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockChainCost {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction: Option<crate::OnchainUnsignedTransaction>,
    pub asset: String,
    pub direction: StockChainDirection,
    pub wallet_address: String,
    pub mint: StockMintEvidence,
    pub quote: StockDexQuote,
    pub transaction_fingerprint: String,
    pub checked_at_ms: i64,
    pub valid_until_ms: i64,
    pub provider_fees: Vec<StockNativeFee>,
    pub network_fee_lamports: Option<String>,
    pub wallet_debit_lamports: Option<String>,
    /// Estimate only: retained rent and transient account funding are not guaranteed maxima.
    pub wallet_budget_lamports: Option<String>,
    /// Conservative simulated funding bound, including temporary outflows and wallet rent reserve.
    #[serde(default)]
    pub wallet_required_lamports: Option<String>,
    #[serde(default)]
    pub native_valuation: Option<StockNativeValuation>,
    pub simulation_slot: Option<u64>,
    pub simulation_passed: bool,
    pub problems: Vec<String>,
}

impl StockChainCost {
    pub fn complete_native_usdc_budget(&self, now: i64) -> Option<String> {
        let budget = self.native_usdc_budget(now)?;
        let debit = self.wallet_debit_lamports.as_deref()?;
        if debit == "0" {
            return Some(budget);
        }
        self.native_valuation
            .as_ref()?
            .complete_budget(debit, &self.wallet_address, now)
    }

    /// Main swap first, SOL replenishment second. Keep their temporary funding separate from cost.
    pub fn total_native_required_lamports(&self, now: i64) -> Option<u64> {
        self.complete_native_usdc_budget(now)?;
        let main = self
            .wallet_required_lamports
            .as_deref()?
            .parse::<u64>()
            .ok()?;
        let debit = self.wallet_debit_lamports.as_deref()?.parse::<u64>().ok()?;
        if debit == 0 {
            return Some(main);
        }
        let topup = self
            .native_valuation
            .as_ref()?
            .replenishment
            .as_ref()?
            .wallet_required_lamports
            .parse::<u64>()
            .ok()?;
        Some(main.max(debit.checked_add(topup)?))
    }

    pub fn native_usdc_budget(&self, now: i64) -> Option<String> {
        if !self.simulation_passed || now < self.checked_at_ms || now >= self.valid_until_ms {
            return None;
        }
        let native = self.wallet_debit_lamports.as_deref()?;
        if native == "0" {
            return Some("0".into());
        }
        let valuation = self.native_valuation.as_ref()?;
        if valuation.replenishment.is_some() {
            valuation.complete_budget(native, &self.wallet_address, now)
        } else {
            valuation.usdc_budget(native, now)
        }
    }

    pub fn current(&self, snapshot: &StockMarketSnapshot, wallet: &str, now: i64) -> bool {
        self.wallet_address == wallet
            && now >= self.checked_at_ms
            && now < self.valid_until_ms
            && comparison::quote_current(&self.quote, now)
            && snapshot
                .security
                .as_ref()
                .is_some_and(|s| s.asset == self.asset)
            && snapshot.comparison.as_ref().is_some_and(|c| {
                c.asset == self.asset
                    && c.mint == self.mint
                    && now >= c.mint.checked_at_ms
                    && now - c.mint.checked_at_ms <= 60_000
                    && c.mint.next_change_at_ms.is_none_or(|t| now < t)
                    && self.direction.quote(c) == Some(&self.quote)
            })
    }
}
