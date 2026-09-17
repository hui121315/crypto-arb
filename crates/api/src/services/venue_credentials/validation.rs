#[cfg(test)]
use shared_types::VenueCredentialPermissionStatus;
#[cfg(not(test))]
use shared_types::VenueId;
use shared_types::{
    BalanceInfo, VenueAccountModeInfo, VenueCredentialPermission, VenueCredentialProbe,
    VenueCredentialProbeStatus, VenueCredentialValidationEvidence, VenueCredentialValidationStatus,
    VenueCredentialValue,
};

use super::specs::VenueSpec;
use super::CredentialUpdateError;
#[cfg(not(test))]
use super::{env_key_for, secret};

#[cfg(not(test))]
use exchange::{
    Binance, BinanceConfig, BinanceCredentials, Bitget, BitgetConfig, BitgetCredentials, Bybit,
    BybitConfig, BybitCredentials, Gate, GateConfig, GateCredentials, GateCrossEx,
    GateCrossExConfig, GateCrossExCredentials, Hyperliquid, HyperliquidConfig,
    HyperliquidCredentials, Kraken, KrakenConfig, KrakenCredentials, KrakenFuturesCredentials,
    KrakenSpotCredentials, Kucoin, KucoinConfig, KucoinCredentials, LiveTradingAdapter, Okx,
    OkxConfig, OkxCredentials,
};

#[cfg(not(test))]
const HTTP_TIMEOUT_SECS: u64 = 4;
#[cfg(not(test))]
const VALIDATION_TIMEOUT_SECS: u64 = 12;
#[cfg(not(test))]
const OPTIONAL_READ_PROBE_TIMEOUT_SECS: u64 = 3;

mod account_mode;
#[cfg(not(test))]
mod fields;
mod hyperliquid_probes;
#[cfg(not(test))]
mod okx_profile;
mod order_permission;
#[cfg(test)]
mod order_permission_denial_tests;
#[cfg(test)]
mod order_permission_matrix_tests;
#[cfg(test)]
mod order_permission_readiness_tests;
#[cfg(test)]
mod order_permission_tests;
mod probes;
mod requests;
mod safe_order_permission;
mod safe_order_permission_error;
#[cfg(test)]
mod tests;
#[cfg(not(test))]
mod venues;

use account_mode::*;
#[cfg(not(test))]
use fields::*;
use hyperliquid_probes::*;
#[cfg(not(test))]
use okx_profile::*;
use order_permission::*;
use probes::*;
use requests::*;
use safe_order_permission::*;
#[cfg(not(test))]
use venues::*;

#[cfg(test)]
pub(super) async fn validate(
    spec: &VenueSpec,
    fields: &[VenueCredentialValue],
) -> Result<VenueCredentialValidationEvidence, CredentialUpdateError> {
    let _ = fields;
    Ok(evidence(
        VenueCredentialValidationStatus::Unknown,
        vec![
            probe(
                "field_completeness",
                VenueCredentialProbeStatus::Unknown,
                spec.venue,
                "test",
                "test build does not call external credential probes",
            ),
            probe(
                "order_permission",
                VenueCredentialProbeStatus::Unknown,
                &format!("{}_place_cancel_order_stream", spec.venue),
                &format!("credential_save.order_permission_unproven.{}", spec.venue),
                "order placement, cancellation, private order stream, and order finality are not proven by credential save",
            ),
            probe(
                "positions_read",
                VenueCredentialProbeStatus::Unknown,
                "private_read.positions",
                "test",
                "test build does not call external positions probe",
            ),
            probe(
                "open_orders_read",
                VenueCredentialProbeStatus::Unknown,
                "private_read.open_orders",
                "test",
                "test build does not call external open-orders probe",
            ),
        ],
    ))
}

#[cfg(not(test))]
pub(super) async fn validate(
    spec: &VenueSpec,
    fields: &[VenueCredentialValue],
) -> Result<VenueCredentialValidationEvidence, CredentialUpdateError> {
    let values = FieldValues { spec, fields };
    match spec.id {
        VenueId::Binance => validate_binance(&values).await,
        VenueId::Okx => validate_okx(&values).await,
        VenueId::Bybit => validate_bybit(&values).await,
        VenueId::Bitget => validate_bitget(&values).await,
        VenueId::Gate => validate_gate(&values).await,
        VenueId::GateCrossEx => validate_gate_crossex(&values).await,
        VenueId::Htx => Err(CredentialUpdateError::UnknownVenue("htx".to_owned())),
        VenueId::Kucoin => validate_kucoin(&values).await,
        VenueId::Hyperliquid => validate_hyperliquid(&values).await,
        VenueId::Kraken => validate_kraken(&values).await,
    }
}
