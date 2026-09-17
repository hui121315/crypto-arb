//! Bybit V5 account-mode and API-key permission evidence.
//!
//! Official references:
//! - <https://bybit-exchange.github.io/docs/v5/account/account-info>
//! - <https://bybit-exchange.github.io/docs/v5/user/apikey-info>

use crate::error::{ExchangeError, ExchangeResult};
use common::time::now_ms;
use serde::Deserialize;
use shared_types::VenueAccountModeInfo;

const EXCHANGE: &str = "bybit";
const ACCOUNT_INFO_SOURCE: &str = "bybit.GET /v5/account/info";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BybitAccountInfo {
    margin_mode: String,
    unified_margin_status: u8,
}

impl BybitAccountInfo {
    pub(super) fn into_mode_info(self) -> ExchangeResult<VenueAccountModeInfo> {
        let account_scope = unified_account_scope(self.unified_margin_status)?;
        let margin_mode = official_margin_mode(&self.margin_mode)?;
        Ok(VenueAccountModeInfo {
            venue: EXCHANGE.to_owned(),
            mode: format!("{account_scope}; marginMode={margin_mode}"),
            source: ACCOUNT_INFO_SOURCE.to_owned(),
            checked_at_ms: now_ms(),
            freshness_ms: Some(0),
            account_scope: Some(account_scope.to_owned()),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BybitApiKeyInfo {
    read_only: u8,
    #[serde(default)]
    permissions: BybitApiPermissions,
}

impl BybitApiKeyInfo {
    pub(super) fn validate_linear_order_permission(&self) -> ExchangeResult<()> {
        match self.read_only {
            0 => {}
            1 => {
                return Err(permission_error(
                    "Bybit API key is read-only; enable Contract Trade Order permission",
                ));
            }
            value => {
                return Err(ExchangeError::Parse(format!(
                    "bybit query-api returned unknown readOnly={value}"
                )));
            }
        }
        if self.permissions.contract_trade_has("Order") {
            Ok(())
        } else {
            Err(permission_error(
                "Bybit API key lacks ContractTrade.Order permission for linear contracts",
            ))
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct BybitApiPermissions {
    #[serde(default, rename = "ContractTrade")]
    contract_trade: Vec<String>,
}

impl BybitApiPermissions {
    fn contract_trade_has(&self, permission: &str) -> bool {
        self.contract_trade
            .iter()
            .any(|value| value.eq_ignore_ascii_case(permission))
    }
}

fn unified_account_scope(status: u8) -> ExchangeResult<&'static str> {
    match status {
        1 => Ok("classic"),
        3 => Ok("uta1"),
        4 => Ok("uta1_pro"),
        5 => Ok("uta2"),
        6 => Ok("uta2_pro"),
        value => Err(ExchangeError::Parse(format!(
            "bybit account info returned unknown unifiedMarginStatus={value}"
        ))),
    }
}

fn official_margin_mode(value: &str) -> ExchangeResult<&str> {
    match value.trim() {
        "ISOLATED_MARGIN" => Ok("ISOLATED_MARGIN"),
        "REGULAR_MARGIN" => Ok("REGULAR_MARGIN"),
        "PORTFOLIO_MARGIN" => Ok("PORTFOLIO_MARGIN"),
        other => Err(ExchangeError::Parse(format!(
            "bybit account info returned unknown marginMode={other}"
        ))),
    }
}

fn permission_error(message: &str) -> ExchangeError {
    ExchangeError::Api {
        exchange: EXCHANGE.to_owned(),
        code: "api_trading_disabled".to_owned(),
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_info_maps_official_uta2_fixture() {
        let fixture: BybitAccountInfo =
            serde_json::from_str(include_str!("../../fixtures/bybit/account_info_uta2.json"))
                .expect("account info fixture");
        let info = fixture.into_mode_info().expect("official account mode");

        assert_eq!(info.mode, "uta2; marginMode=REGULAR_MARGIN");
        assert_eq!(info.account_scope.as_deref(), Some("uta2"));
        assert_eq!(info.source, ACCOUNT_INFO_SOURCE);
    }

    #[test]
    fn api_key_info_accepts_contract_order_permission() {
        let fixture: BybitApiKeyInfo =
            serde_json::from_str(include_str!("../../fixtures/bybit/api_key_info_order.json"))
                .expect("api key fixture");

        fixture
            .validate_linear_order_permission()
            .expect("linear order permission");
    }

    #[test]
    fn api_key_info_rejects_read_only_or_missing_order_permission() {
        let read_only: BybitApiKeyInfo = serde_json::from_value(serde_json::json!({
            "readOnly": 1,
            "permissions": {"ContractTrade": ["Order", "Position"]}
        }))
        .expect("read-only response");
        let missing_order: BybitApiKeyInfo = serde_json::from_value(serde_json::json!({
            "readOnly": 0,
            "permissions": {"ContractTrade": ["Position"]}
        }))
        .expect("missing-order response");

        assert!(read_only
            .validate_linear_order_permission()
            .expect_err("read-only key")
            .to_string()
            .contains("read-only"));
        assert!(missing_order
            .validate_linear_order_permission()
            .expect_err("missing order permission")
            .to_string()
            .contains("ContractTrade.Order"));
    }
}
