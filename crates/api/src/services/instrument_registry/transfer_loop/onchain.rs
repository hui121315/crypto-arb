use super::super::{InstrumentRegistry, TransferProbeState, TRANSFER_SUPPORTED_VENUES};
use super::TransferLoopIndex;
use exchange::{canonical_network_id, contracts_compatible, TRANSFER_NETWORK_FRESHNESS_MS};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use shared_types::{
    OnchainTransferDirection, OnchainTransferEvidence, OnchainTransferStatus,
    EVM_NATIVE_TOKEN_ADDRESS,
};

struct TransferQuery<'a> {
    venue: &'a str,
    asset: &'a str,
    chain: &'a str,
    contract_address: Option<&'a str>,
    direction: OnchainTransferDirection,
    amount: Decimal,
    asset_decimals: u8,
    now_ms: i64,
}

impl InstrumentRegistry {
    pub(crate) fn onchain_transfer_evidence(
        &self,
        venue: &str,
        asset: &str,
        chain: &str,
        contract_address: Option<&str>,
        direction: OnchainTransferDirection,
        amount: Decimal,
        asset_decimals: u8,
        now_ms: i64,
    ) -> OnchainTransferEvidence {
        project(
            self,
            TransferQuery {
                venue,
                asset,
                chain,
                contract_address,
                direction,
                amount,
                asset_decimals,
                now_ms,
            },
        )
    }
}

fn project(registry: &InstrumentRegistry, query: TransferQuery<'_>) -> OnchainTransferEvidence {
    let family = shared_types::venue_family(query.venue);
    if !TRANSFER_SUPPORTED_VENUES.contains(&family) {
        return evidence(
            &query,
            OnchainTransferStatus::Unsupported,
            None,
            Some(format!(
                "{} 尚无可核验的官方充提接口",
                query.venue.to_ascii_uppercase()
            )),
        );
    }
    match registry.transfer_probe_state(query.venue) {
        Some(TransferProbeState::Refreshing { .. }) => {
            return evidence(
                &query,
                OnchainTransferStatus::Refreshing,
                None,
                Some("正在按当前候选读取官方充提网络".to_owned()),
            );
        }
        Some(TransferProbeState::Success { checked_at_ms })
            if query.now_ms >= checked_at_ms
                && query.now_ms.saturating_sub(checked_at_ms) < TRANSFER_NETWORK_FRESHNESS_MS => {}
        Some(TransferProbeState::Success { .. }) => {
            return evidence(
                &query,
                OnchainTransferStatus::Unknown,
                None,
                Some("官方充提网络证据已过期，等待候选触发刷新".to_owned()),
            );
        }
        Some(TransferProbeState::Unavailable { problem, .. }) => {
            return evidence(
                &query,
                OnchainTransferStatus::Unsupported,
                None,
                Some(problem.message),
            );
        }
        Some(TransferProbeState::Failed { problem, .. }) => {
            return evidence(
                &query,
                OnchainTransferStatus::Unknown,
                None,
                Some(format!("官方充提网络刷新失败：{}", problem.message)),
            );
        }
        Some(TransferProbeState::Unsupported { problem, .. }) => {
            return evidence(
                &query,
                OnchainTransferStatus::Unsupported,
                None,
                Some(problem.message),
            );
        }
        None => {
            return evidence(
                &query,
                OnchainTransferStatus::Unknown,
                None,
                Some("尚未取得官方充提网络状态".to_owned()),
            );
        }
    }
    if query.amount <= Decimal::ZERO || query.asset_decimals > 28 {
        return evidence(
            &query,
            OnchainTransferStatus::Blocked,
            None,
            Some("补仓数量无效".to_owned()),
        );
    }

    let canonical_chain = canonical_network_id(query.chain);
    let index = TransferLoopIndex::snapshot(registry);
    let candidates = index
        .transfer_rows(query.venue, query.asset)
        .iter()
        .filter(|row| row.is_fresh_at(query.now_ms))
        .filter(|row| row.canonical_network == canonical_chain)
        .filter(|row| {
            if canonical_chain == "solana" && row.contract_address.is_some() && query.contract_address.is_some() {
                contract_verified(row.contract_address.as_deref(), &query)
            } else {
                contracts_compatible(row.contract_address.as_deref(), query.contract_address)
            }
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return evidence(
            &query,
            OnchainTransferStatus::Blocked,
            None,
            Some(format!(
                "{} {} 没有与 {} 合约身份兼容的新鲜官方网络",
                query.venue.to_ascii_uppercase(),
                query.asset.to_ascii_uppercase(),
                canonical_chain
            )),
        );
    }

    let enabled = candidates
        .into_iter()
        .filter(|row| match query.direction {
            OnchainTransferDirection::WithdrawToChain => row.withdraw_enabled,
            OnchainTransferDirection::DepositToCex => row.deposit_enabled,
        })
        .collect::<Vec<_>>();
    if enabled.is_empty() {
        let action = match query.direction {
            OnchainTransferDirection::WithdrawToChain => "提币",
            OnchainTransferDirection::DepositToCex => "充值",
        };
        return evidence(
            &query,
            OnchainTransferStatus::Blocked,
            None,
            Some(format!("{canonical_chain} 当前未开放{action}")),
        );
    }

    let mut eligible = enabled
        .into_iter()
        .filter_map(|row| {
            let minimum = match query.direction {
                OnchainTransferDirection::WithdrawToChain => row.min_withdraw,
                OnchainTransferDirection::DepositToCex => row.min_deposit,
            };
            let amount = transfer_amount(&query, minimum, row.withdrawal_step)?;
            let fee = match query.direction {
                OnchainTransferDirection::WithdrawToChain => {
                    let fixed = row.withdrawal_fee?;
                    let rate = row.withdrawal_fee_rate?;
                    let variable = rate.checked_mul(amount)?;
                    // Bybit feeType=0 requests a net amount; its percentage fee is grossed up.
                    // https://bybit-exchange.github.io/docs/v5/asset/withdraw
                    let variable = if shared_types::venue_family(&row.venue) == "bybit" {
                        if rate >= Decimal::ONE {
                            return None;
                        }
                        variable.checked_div(Decimal::ONE.checked_sub(rate)?)?
                    } else {
                        variable
                    };
                    fixed.checked_add(variable)?
                }
                OnchainTransferDirection::DepositToCex => Decimal::ZERO,
            };
            Some((row, minimum, fee, amount))
        })
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        return evidence(
            &query,
            OnchainTransferStatus::Unknown,
            None,
            Some("网络已开放，但手续费、最小数量或精确数量步长证据不足".to_owned()),
        );
    }
    eligible.sort_by(|left, right| {
        let left_contract = contract_verified(left.0.contract_address.as_deref(), &query);
        let right_contract = contract_verified(right.0.contract_address.as_deref(), &query);
        right_contract
            .cmp(&left_contract)
            .then_with(|| left.2.cmp(&right.2))
    });
    let (row, minimum_exact, fee_exact, amount_exact) = eligible[0];
    let minimum = minimum_exact.and_then(|value| value.to_f64());
    let fee = fee_exact.to_f64();
    let exact_contract = contract_verified(row.contract_address.as_deref(), &query);
    let status = if exact_contract {
        OnchainTransferStatus::Ready
    } else {
        OnchainTransferStatus::Unknown
    };
    OnchainTransferEvidence {
        direction: query.direction,
        venue: query.venue.to_owned(),
        asset: query.asset.to_ascii_uppercase(),
        chain: canonical_chain,
        network: Some(row.network.clone()),
        amount: amount_exact.to_f64().expect("Decimal fits f64 range"),
        amount_exact: Some(amount_exact.normalize().to_string()),
        status,
        fee,
        fee_exact: Some(fee_exact.normalize().to_string()),
        minimum,
        minimum_exact: minimum_exact.map(|value| value.normalize().to_string()),
        amount_step: transfer_step(&query, row.withdrawal_step).map(|step| step.normalize().to_string()),
        requires_tag: row.requires_tag,
        contract_verified: exact_contract,
        credit_confirmations: row.credit_confirmations,
        unlock_confirmations: row.unlock_confirmations,
        network_status: row.network_status.clone(),
        source: Some(row.source_url.clone()),
        observed_at_ms: Some(row.checked_at_ms),
        problem: (!exact_contract)
            .then(|| "网络已开放，但官方响应未提供可与当前合约精确核对的地址".to_owned()),
    }
}

fn transfer_amount(
    query: &TransferQuery<'_>,
    minimum: Option<Decimal>,
    withdrawal_step: Option<Decimal>,
) -> Option<Decimal> {
    let amount = query.amount.max(minimum?);
    let step = transfer_step(query, withdrawal_step)?;
    // Remainders avoid dividing/rounding away a fractional atomic unit before ceil.
    let remainder = amount.checked_rem(step)?;
    if remainder.is_zero() {
        Some(amount)
    } else {
        amount.checked_add(step.checked_sub(remainder)?)
    }
}

fn transfer_step(query: &TransferQuery<'_>, withdrawal_step: Option<Decimal>) -> Option<Decimal> {
    let atomic_step = Decimal::try_new(1, u32::from(query.asset_decimals)).ok()?;
    let step = match query.direction {
        OnchainTransferDirection::WithdrawToChain => {
            let venue_step = withdrawal_step?;
            if venue_step > Decimal::ZERO && atomic_step.checked_rem(venue_step)?.is_zero() {
                atomic_step
            } else {
                venue_step
            }
        }
        OnchainTransferDirection::DepositToCex => atomic_step,
    };
    if step <= Decimal::ZERO || !step.checked_rem(atomic_step)?.is_zero() {
        return None;
    }
    Some(step)
}

fn contract_verified(official: Option<&str>, query: &TransferQuery<'_>) -> bool {
    let requested = query
        .contract_address
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let native =
        requested.is_some_and(|value| value.eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS));
    match (
        official.map(str::trim).filter(|value| !value.is_empty()),
        requested,
    ) {
        (Some(official), Some(requested)) if canonical_network_id(query.chain) == "solana" => {
            official == requested
        }
        (Some(official), Some(requested)) => official.eq_ignore_ascii_case(requested),
        (None, Some(_)) if native => true,
        (None, None) => false,
        _ => false,
    }
}

fn evidence(
    query: &TransferQuery<'_>,
    status: OnchainTransferStatus,
    observed_at_ms: Option<i64>,
    problem: Option<String>,
) -> OnchainTransferEvidence {
    OnchainTransferEvidence {
        direction: query.direction,
        venue: query.venue.to_owned(),
        asset: query.asset.to_ascii_uppercase(),
        chain: canonical_network_id(query.chain),
        network: None,
        amount: query.amount.to_f64().expect("Decimal fits f64 range"),
        amount_exact: None,
        status,
        fee: None,
        fee_exact: None,
        minimum: None,
        minimum_exact: None,
        amount_step: None,
        requires_tag: false,
        contract_verified: false,
        credit_confirmations: None,
        unlock_confirmations: None,
        network_status: None,
        source: None,
        observed_at_ms,
        problem,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exchange::CurrencyTransferNetwork;
    use rust_decimal::Decimal;

    #[test]
    fn exact_chain_and_contract_prove_candidate_withdrawal() {
        let registry = InstrumentRegistry::default();
        registry.replace_transfer_venue("binance", vec![network(Some("0xabc"), true, true)]);

        let result = registry.onchain_transfer_evidence(
            "binance",
            "USDC",
            "base",
            Some("0xAbC"),
            OnchainTransferDirection::WithdrawToChain,
            Decimal::from(100),
            6,
            1_100,
        );

        assert_eq!(result.status, OnchainTransferStatus::Ready);
        assert_eq!(result.network.as_deref(), Some("BASE"));
        assert_eq!(result.fee, Some(0.1));
        assert_eq!(result.credit_confirmations, Some(12));
        assert_eq!(result.unlock_confirmations, Some(24));
        assert!(result.contract_verified);
    }

    #[test]
    fn open_network_without_contract_identity_stays_unknown() {
        let registry = InstrumentRegistry::default();
        registry.replace_transfer_venue("binance", vec![network(None, true, true)]);

        let result = registry.onchain_transfer_evidence(
            "binance",
            "USDC",
            "base",
            Some("0xabc"),
            OnchainTransferDirection::DepositToCex,
            Decimal::from(100),
            6,
            1_100,
        );

        assert_eq!(result.status, OnchainTransferStatus::Unknown);
        assert!(!result.contract_verified);
        assert!(result
            .problem
            .as_deref()
            .is_some_and(|problem| problem.contains("合约")));
    }

    #[test]
    fn replenishment_minimum_and_step_are_applied_before_percentage_fee() {
        let registry = InstrumentRegistry::default();
        let mut row = network(Some("0xabc"), true, true);
        row.min_withdraw = Some(Decimal::new(10003, 3));
        row.withdrawal_step = Some(Decimal::new(1, 2));
        row.withdrawal_fee_rate = Some(Decimal::new(1, 2));
        registry.replace_transfer_venue("binance", vec![row]);

        let result = registry.onchain_transfer_evidence(
            "binance",
            "USDC",
            "base",
            Some("0xabc"),
            OnchainTransferDirection::WithdrawToChain,
            Decimal::new(3001, 4),
            6,
            1_100,
        );
        assert_eq!(result.status, OnchainTransferStatus::Ready);
        assert_eq!(result.amount_exact.as_deref(), Some("10.01"));
        assert_eq!(result.amount, 10.01);
        assert_eq!(result.fee_exact.as_deref(), Some("0.2001"));
        assert_eq!(result.fee, Some(0.2001));
    }

    #[test]
    fn replenishment_deposits_round_up_and_support_high_decimal_assets() {
        let registry = InstrumentRegistry::default();
        registry.replace_transfer_venue("binance", vec![network(Some("0xabc"), true, true)]);
        for (requested, decimals, expected) in [
            ("20.0000001", 6, "20.000001"),
            ("1.2", 0, "2"),
            (
                "0.000000000000000000000001",
                24,
                "0.000000000000000000000001",
            ),
        ] {
            let result = registry.onchain_transfer_evidence(
                "binance",
                "USDC",
                "base",
                Some("0xabc"),
                OnchainTransferDirection::DepositToCex,
                requested.parse().unwrap(),
                decimals,
                1_100,
            );
            assert_eq!(result.status, OnchainTransferStatus::Ready);
            assert_eq!(result.amount_exact.as_deref(), Some(expected));
        }
    }

    #[test]
    fn replenishment_cannot_invent_missing_or_incompatible_quantity_rules() {
        for (minimum, step, decimals) in [
            (None, Some(Decimal::ONE), 6),
            (Some(Decimal::ZERO), None, 6),
            // Neither grid divides the other; do not assume one is sufficient.
            (Some(Decimal::ZERO), Some(Decimal::new(3, 7)), 6),
            (Some(Decimal::ZERO), Some(Decimal::ONE), 29),
        ] {
            let registry = InstrumentRegistry::default();
            let mut row = network(Some("0xabc"), true, true);
            row.min_withdraw = minimum;
            row.withdrawal_step = step;
            registry.replace_transfer_venue("binance", vec![row]);
            let result = registry.onchain_transfer_evidence(
                "binance",
                "USDC",
                "base",
                Some("0xabc"),
                OnchainTransferDirection::WithdrawToChain,
                Decimal::ONE,
                decimals,
                1_100,
            );
            assert_ne!(result.status, OnchainTransferStatus::Ready);
            assert!(result.amount_exact.is_none());
        }
    }

    #[test]
    fn replenishment_bybit_net_amount_uses_official_fee_type_zero_formula() {
        let registry = InstrumentRegistry::default();
        let mut row = network(Some("0xabc"), true, true);
        row.venue = "bybit".to_owned();
        row.withdrawal_fee_rate = Some(Decimal::new(1, 2));
        registry.replace_transfer_venue("bybit", vec![row]);
        let result = registry.onchain_transfer_evidence(
            "bybit",
            "USDC",
            "base",
            Some("0xabc"),
            OnchainTransferDirection::WithdrawToChain,
            Decimal::from(99),
            6,
            1_100,
        );
        assert_eq!(result.fee_exact.as_deref(), Some("1.1"));
    }

    #[test]
    fn missing_contract_on_both_sides_does_not_prove_identity() {
        let query = TransferQuery {
            venue: "binance",
            asset: "UNKNOWN",
            chain: "base",
            contract_address: Some(""),
            direction: OnchainTransferDirection::WithdrawToChain,
            amount: Decimal::ONE,
            asset_decimals: 6,
            now_ms: 1,
        };

        assert!(!contract_verified(None, &query));
    }

    #[test]
    fn kraken_deposit_solana_contract_identity_is_case_sensitive() {
        let query = TransferQuery {
            venue: "kraken", asset: "USDC", chain: "solana", contract_address: Some("CaseSensitiveMint"),
            direction: OnchainTransferDirection::DepositToCex, amount: Decimal::ONE, asset_decimals: 6, now_ms: 1,
        };
        assert!(contract_verified(Some("CaseSensitiveMint"), &query));
        assert!(!contract_verified(Some("casesensitivemint"), &query));
    }

    #[test]
    fn kraken_withdrawal_account_precision_is_combined_with_chain_atomic_units() {
        let query = TransferQuery {
            venue: "kraken", asset: "USDC", chain: "solana", contract_address: Some("CaseSensitiveMint"),
            direction: OnchainTransferDirection::WithdrawToChain,
            amount: "12.5000000001".parse().unwrap(), asset_decimals: 6, now_ms: 1,
        };
        let step = Some(Decimal::new(1, 10));
        assert_eq!(transfer_step(&query, step), Some(Decimal::new(1, 6)));
        assert_eq!(transfer_amount(&query, Some(Decimal::ONE), step), Some("12.500001".parse().unwrap()));
    }

    #[test]
    fn scoped_refresh_preserves_other_candidate_currencies() {
        let registry = InstrumentRegistry::default();
        let mut usdt = network(Some("0xusdt"), true, true);
        usdt.currency = "USDT".to_owned();
        registry.replace_transfer_venue("binance", vec![network(Some("0xusdc"), true, true), usdt]);

        let mut refreshed_usdc = network(Some("0xusdc"), true, true);
        refreshed_usdc.withdrawal_fee = Some(Decimal::new(2, 1));
        assert_eq!(
            registry.replace_transfer_venue_scope(
                "binance",
                &["USDC".to_owned()],
                vec![refreshed_usdc],
            ),
            1
        );

        let index = TransferLoopIndex::snapshot(&registry);
        assert_eq!(index.transfer_rows("binance", "USDC").len(), 1);
        assert_eq!(index.transfer_rows("binance", "USDT").len(), 1);
        assert_eq!(registry.transfer_len(), 2);
        assert!(!registry.begin_transfer_refresh_for("binance", &["USDT".to_owned()], 1_100,));
        assert!(registry.begin_transfer_refresh_for("binance", &["ETH".to_owned()], 1_100,));
    }

    #[test]
    fn unavailable_candidate_currency_is_negatively_cached() {
        let registry = InstrumentRegistry::default();
        let now_ms = common::time::now_ms();
        registry.record_transfer_unavailable(
            "bybit",
            &["PUPS".to_owned()],
            "official endpoint returned no networks for PUPS",
        );

        assert!(!registry.begin_transfer_refresh_for("bybit", &["PUPS".to_owned()], now_ms));
        assert!(registry.begin_transfer_refresh_for("bybit", &["ETH".to_owned()], now_ms));
    }

    fn network(
        contract_address: Option<&str>,
        deposit_enabled: bool,
        withdraw_enabled: bool,
    ) -> CurrencyTransferNetwork {
        CurrencyTransferNetwork {
            venue: "binance".to_owned(),
            currency: "USDC".to_owned(),
            network: "BASE".to_owned(),
            canonical_network: "base".to_owned(),
            contract_address: contract_address.map(str::to_owned),
            deposit_enabled,
            withdraw_enabled,
            withdrawal_fee: Some(Decimal::new(1, 1)),
            withdrawal_fee_rate: Some(Decimal::ZERO),
            withdrawal_step: Some(Decimal::new(1, 6)),
            min_withdraw: Some(Decimal::ONE),
            min_deposit: Some(Decimal::ZERO),
            requires_tag: false,
            credit_confirmations: Some(12),
            unlock_confirmations: Some(24),
            network_status: None,
            checked_at_ms: 1_000,
            source_url: "https://developers.binance.com/docs/wallet/capital/all-coins-info"
                .to_owned(),
        }
    }
}
