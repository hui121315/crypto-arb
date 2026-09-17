//! Official venue currency-network evidence used to prove cross-venue spot
//! rebalancing loops. This is cold metadata; callers must cache it outside the
//! market-data hot path.

use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDirection {
    WithdrawToChain,
    DepositToVenue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferDestinationRequest {
    pub currency: String,
    pub network: String,
    pub direction: TransferDirection,
    pub expected_address: Option<String>,
    pub expected_tag: Option<String>,
    /// Exact transfer amount for destination-side funding limits.
    pub amount: Option<Decimal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDestinationStatus {
    Verified,
    Missing,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferDestinationEvidence {
    pub venue: String,
    pub currency: String,
    pub network: String,
    pub direction: TransferDirection,
    pub address: Option<String>,
    pub tag: Option<String>,
    pub status: TransferDestinationStatus,
    pub allowlisted: Option<bool>,
    pub checked_at_ms: i64,
    pub source_url: String,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalSubmitRequest {
    pub venue: String,
    pub currency: String,
    pub network: String,
    pub address: String,
    pub tag: Option<String>,
    pub amount: Decimal,
    pub client_withdrawal_id: String,
    pub wallet_type: WithdrawalWalletType,
    /// Approved fee ceiling; venues supporting fee pinning must check it before sending.
    pub max_fee: Option<Decimal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawalWalletType {
    Spot,
    Funding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalSourceBalanceRequest {
    pub venue: String,
    pub currency: String,
    pub wallet_type: WithdrawalWalletType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalSourceBalance {
    pub venue: String,
    pub currency: String,
    pub wallet_type: WithdrawalWalletType,
    pub available: Decimal,
    pub checked_at_ms: i64,
    pub source_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalSubmission {
    pub venue: String,
    pub provider_withdrawal_id: String,
    pub client_withdrawal_id: String,
    pub submitted_at_ms: i64,
    pub source_url: String,
    /// Preserve a known receipt even if the acknowledgement's amounts are inconsistent.
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalStatusRequest {
    pub venue: String,
    pub currency: String,
    pub network: String,
    pub address: String,
    pub tag: Option<String>,
    pub client_withdrawal_id: String,
    pub provider_withdrawal_id: Option<String>,
    pub submitted_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawalStatus {
    Pending,
    Completed,
    Cancelled,
    Rejected,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalStatusEvidence {
    pub venue: String,
    pub provider_withdrawal_id: String,
    pub client_withdrawal_id: String,
    pub currency: String,
    pub network: String,
    pub address: String,
    pub amount: Decimal,
    pub transaction_fee: Decimal,
    pub status: WithdrawalStatus,
    pub transaction_id: Option<String>,
    pub confirmations: Option<u64>,
    pub checked_at_ms: i64,
    pub source_url: String,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositStatusRequest {
    pub venue: String,
    pub currency: String,
    pub network: String,
    pub address: String,
    pub tag: Option<String>,
    pub transaction_id: String,
    /// Planned credit target, not a filter for locating the deposit record.
    pub amount: Decimal,
    pub submitted_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepositStatus {
    Pending,
    /// Credited to the venue account and usable for trading, but not yet
    /// unlocked for a later withdrawal.
    CreditedLocked,
    Completed,
    /// The venue requires manual compliance or account action before funds
    /// can be credited. Automated reconciliation must pause.
    Blocked,
    /// The venue reports a terminal deposit failure or rollback.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositStatusEvidence {
    pub venue: String,
    pub currency: String,
    pub network: String,
    pub address: String,
    pub tag: Option<String>,
    /// Venue-reported amount. It may differ from the planned transfer amount.
    pub amount: Decimal,
    /// Reported separately by the venue. Do not subtract unless its amount
    /// field is explicitly documented as gross of this fee.
    pub deposit_fee: Option<Decimal>,
    pub status: DepositStatus,
    pub transaction_id: String,
    pub confirmations: Option<u64>,
    pub checked_at_ms: i64,
    pub source_url: String,
    pub problem: Option<String>,
}

pub const TRANSFER_NETWORK_FRESHNESS_MS: i64 = 30 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq)]
pub struct CurrencyTransferNetwork {
    pub venue: String,
    pub currency: String,
    pub network: String,
    pub canonical_network: String,
    pub contract_address: Option<String>,
    pub deposit_enabled: bool,
    pub withdraw_enabled: bool,
    pub withdrawal_fee: Option<Decimal>,
    pub withdrawal_fee_rate: Option<Decimal>,
    pub withdrawal_step: Option<Decimal>,
    pub min_withdraw: Option<Decimal>,
    pub min_deposit: Option<Decimal>,
    pub requires_tag: bool,
    /// Confirmations after which a deposit can be used for trading.
    pub credit_confirmations: Option<u64>,
    /// Confirmations after which deposited funds are fully unlocked for withdrawal.
    pub unlock_confirmations: Option<u64>,
    /// Provider status such as congestion, maintenance, or delayed withdrawal.
    pub network_status: Option<String>,
    pub checked_at_ms: i64,
    pub source_url: String,
}

impl CurrencyTransferNetwork {
    pub fn is_structurally_valid(&self) -> bool {
        !self.venue.trim().is_empty()
            && !self.currency.trim().is_empty()
            && !self.network.trim().is_empty()
            && !self.canonical_network.trim().is_empty()
            && self.checked_at_ms > 0
            && self
                .withdrawal_fee
                .is_none_or(|value| value >= Decimal::ZERO)
            && self
                .withdrawal_fee_rate
                .is_none_or(|value| value >= Decimal::ZERO)
            && self
                .withdrawal_step
                .is_none_or(|value| value > Decimal::ZERO)
            && self.min_withdraw.is_none_or(|value| value >= Decimal::ZERO)
            && self.min_deposit.is_none_or(|value| value >= Decimal::ZERO)
            && !self.source_url.trim().is_empty()
    }

    pub fn has_cost_evidence(&self) -> bool {
        self.withdrawal_fee.is_some()
            && self.withdrawal_fee_rate.is_some()
            && self.min_withdraw.is_some()
    }

    pub fn is_fresh_at(&self, now_ms: i64) -> bool {
        now_ms >= self.checked_at_ms
            && now_ms.saturating_sub(self.checked_at_ms) < TRANSFER_NETWORK_FRESHNESS_MS
    }
}

pub fn canonical_network_id(value: &str) -> String {
    let compact = value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>();
    match compact.as_str() {
        "eth" | "ethereum" | "erc20" | "ethereumerc20" => "ethereum",
        "trx" | "tron" | "trc20" | "trontrc20" => "tron",
        "bsc" | "bep20" | "bnbsmartchain" | "bnbsmartchainbep20" => "bsc",
        "arb" | "arbitrum" | "arbitrumone" => "arbitrum",
        "op" | "optimism" | "optimismethereum" => "optimism",
        "base" | "baseethereum" => "base",
        "matic" | "polygon" | "polygonpos" => "polygon",
        "avaxc" | "avalanchec" | "avalanchecchain" => "avalanche-c",
        "sol" | "solana" => "solana",
        "btc" | "bitcoin" => "bitcoin",
        "btcln" | "lightning" | "lightningnetwork" | "bitcoinlightning" => "bitcoin-lightning",
        "bch" | "bitcoincash" => "bitcoin-cash",
        "ltc" | "litecoin" => "litecoin",
        "doge" | "dogecoin" => "dogecoin",
        "xrp" | "ripple" => "xrp",
        "xlm" | "stellar" => "stellar",
        "ton" | "theopennetwork" => "ton",
        "dot" | "polkadot" => "polkadot",
        "atom" | "cosmos" => "cosmos",
        "ada" | "cardano" => "cardano",
        "near" => "near",
        "apt" | "aptos" => "aptos",
        "sui" => "sui",
        "kcc" => "kcc",
        other => other,
    }
    .to_owned()
}

pub fn normalized_contract_address(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
}

pub fn contracts_compatible(left: Option<&str>, right: Option<&str>) -> bool {
    match (
        normalized_contract_address(left),
        normalized_contract_address(right),
    ) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

pub fn parse_optional_decimal(value: Option<&str>) -> Option<Decimal> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_network_aliases_match_cross_venue_names() {
        assert_eq!(canonical_network_id("ERC20"), "ethereum");
        assert_eq!(canonical_network_id("ETH"), "ethereum");
        assert_eq!(canonical_network_id("TRC20"), "tron");
        assert_eq!(canonical_network_id("Arbitrum One"), "arbitrum");
        assert_eq!(
            canonical_network_id("Lightning Network"),
            "bitcoin-lightning"
        );
    }

    #[test]
    fn conflicting_contracts_do_not_share_a_route() {
        assert!(contracts_compatible(Some("0xAB"), Some("0xab")));
        assert!(!contracts_compatible(Some("0x01"), Some("0x02")));
        assert!(contracts_compatible(Some("0x01"), None));
    }
}
