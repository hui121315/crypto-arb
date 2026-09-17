//! High-risk user action run contract.

use crate::live_trading::ProtectedPositionFingerprint;
use crate::portfolio::AutoProfitCloseConfig;
use crate::problem::ApiProblem;
use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Serialize};

#[path = "actions/evidence.rs"]
mod evidence;
mod state;

pub use evidence::{ActionEvidence, ActionEvidenceSource};
pub use state::ActionState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionRunKind {
    TradingRiskConfigUpdate,
    TradingAdapterSelect,
    TradingKillSwitch,
    TradingFeeSnapshotUpsert,
    TradingOrderSubmit,
    TradingOrderCancel,
    TradingOrderReconcile,
    AutomationConfigUpdate,
    AutomationControl,
    AutomationLiveUnlock,
    HedgeConfirm,
    WebhookConfigUpdate,
    MarketSubscriptionsUpdate,
    GateCrossExModeUpdate,
    VenueCredentialsUpdate,
    VenueCredentialsClear,
    VenueCredentialsMigrate,
    OnchainProviderCredentialsUpdate,
    OnchainProviderCredentialsClear,
    PortfolioClosePosition,
    PortfolioClosePair,
    PortfolioCloseAll,
    PortfolioCloseCompensation,
    PortfolioCloseManualTerminal,
}

impl ActionRunKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TradingRiskConfigUpdate => "trading_risk_config_update",
            Self::TradingAdapterSelect => "trading_adapter_select",
            Self::TradingKillSwitch => "trading_kill_switch",
            Self::TradingFeeSnapshotUpsert => "trading_fee_snapshot_upsert",
            Self::TradingOrderSubmit => "trading_order_submit",
            Self::TradingOrderCancel => "trading_order_cancel",
            Self::TradingOrderReconcile => "trading_order_reconcile",
            Self::AutomationConfigUpdate => "automation_config_update",
            Self::AutomationControl => "automation_control",
            Self::AutomationLiveUnlock => "automation_live_unlock",
            Self::HedgeConfirm => "hedge_confirm",
            Self::WebhookConfigUpdate => "webhook_config_update",
            Self::MarketSubscriptionsUpdate => "market_subscriptions_update",
            Self::GateCrossExModeUpdate => "gate_crossex_mode_update",
            Self::VenueCredentialsUpdate => "venue_credentials_update",
            Self::VenueCredentialsClear => "venue_credentials_clear",
            Self::VenueCredentialsMigrate => "venue_credentials_migrate",
            Self::OnchainProviderCredentialsUpdate => "onchain_provider_credentials_update",
            Self::OnchainProviderCredentialsClear => "onchain_provider_credentials_clear",
            Self::PortfolioClosePosition => "portfolio_close_position",
            Self::PortfolioClosePair => "portfolio_close_pair",
            Self::PortfolioCloseAll => "portfolio_close_all",
            Self::PortfolioCloseCompensation => "portfolio_close_compensation",
            Self::PortfolioCloseManualTerminal => "portfolio_close_manual_terminal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionRunStatus {
    Accepted,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "field", rename_all = "snake_case")]
pub enum ActionMutationChange {
    Adapter {
        before: String,
        after: String,
    },
    ExecutionEnvironment {
        before: String,
        after: String,
    },
    LiveTradingEnabled {
        before: bool,
        after: bool,
    },
    KillSwitchActive {
        before: bool,
        after: bool,
    },
    MaxOrderNotional {
        before: f64,
        after: f64,
    },
    MaxOpenOrders {
        before: usize,
        after: usize,
    },
    MaxHedgeImbalancePct {
        before: f64,
        after: f64,
    },
    LiquidationWarnPct {
        before: f64,
        after: f64,
    },
    LiquidationDangerPct {
        before: f64,
        after: f64,
    },
    AllowedExchanges {
        before: Vec<String>,
        after: Vec<String>,
    },
    AllowedSymbols {
        before: Vec<String>,
        after: Vec<String>,
    },
    ProtectedPositions {
        before: Vec<ProtectedPositionFingerprint>,
        after: Vec<ProtectedPositionFingerprint>,
    },
    AutoProfitClose {
        before: AutoProfitCloseConfig,
        after: AutoProfitCloseConfig,
    },
}

#[derive(Deserialize)]
struct ActionMutationWire {
    field: String,
    before: serde_json::Value,
    after: serde_json::Value,
}

impl<'de> Deserialize<'de> for ActionMutationChange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ActionMutationWire::deserialize(deserializer)?;
        let field = wire.field.as_str();
        let before = wire.before;
        let after = wire.after;
        match field {
            "adapter" => decode_mutation_pair(before, after)
                .map(|(before, after)| Self::Adapter { before, after }),
            "execution_environment" => decode_mutation_pair(before, after)
                .map(|(before, after)| Self::ExecutionEnvironment { before, after }),
            "live_trading_enabled" => decode_mutation_pair(before, after)
                .map(|(before, after)| Self::LiveTradingEnabled { before, after }),
            "kill_switch_active" => decode_mutation_pair(before, after)
                .map(|(before, after)| Self::KillSwitchActive { before, after }),
            "max_order_notional" => decode_mutation_pair(before, after)
                .map(|(before, after)| Self::MaxOrderNotional { before, after }),
            "max_open_orders" => decode_mutation_pair(before, after)
                .map(|(before, after)| Self::MaxOpenOrders { before, after }),
            "max_hedge_imbalance_pct" => decode_mutation_pair(before, after)
                .map(|(before, after)| Self::MaxHedgeImbalancePct { before, after }),
            "liquidation_warn_pct" => decode_mutation_pair(before, after)
                .map(|(before, after)| Self::LiquidationWarnPct { before, after }),
            "liquidation_danger_pct" => decode_mutation_pair(before, after)
                .map(|(before, after)| Self::LiquidationDangerPct { before, after }),
            "allowed_exchanges" => decode_mutation_pair(before, after)
                .map(|(before, after)| Self::AllowedExchanges { before, after }),
            "allowed_symbols" => decode_mutation_pair(before, after)
                .map(|(before, after)| Self::AllowedSymbols { before, after }),
            "protected_positions" => decode_mutation_pair(before, after)
                .map(|(before, after)| Self::ProtectedPositions { before, after }),
            "auto_profit_close" => decode_mutation_pair(before, after)
                .map(|(before, after)| Self::AutoProfitClose { before, after }),
            unknown => Err(D::Error::unknown_variant(
                unknown,
                &[
                    "adapter",
                    "execution_environment",
                    "live_trading_enabled",
                    "kill_switch_active",
                    "max_order_notional",
                    "max_open_orders",
                    "max_hedge_imbalance_pct",
                    "liquidation_warn_pct",
                    "liquidation_danger_pct",
                    "allowed_exchanges",
                    "allowed_symbols",
                    "protected_positions",
                    "auto_profit_close",
                ],
            )),
        }
    }
}

fn decode_mutation_pair<T, E>(
    before: serde_json::Value,
    after: serde_json::Value,
) -> Result<(T, T), E>
where
    T: DeserializeOwned,
    E: serde::de::Error,
{
    let before = serde_json::from_value(before).map_err(E::custom)?;
    let after = serde_json::from_value(after).map_err(E::custom)?;
    Ok((before, after))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionMutationDiff {
    pub effective_at_ms: i64,
    pub changes: Vec<ActionMutationChange>,
}

impl ActionMutationDiff {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRun {
    pub id: String,
    pub kind: ActionRunKind,
    pub status: ActionRunStatus,
    pub actor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutation: Option<ActionMutationDiff>,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
}

pub type ActionRunEnvelope = crate::ResourceEnvelope<Vec<ActionRun>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_run_uses_camel_case_fields() {
        let run = ActionRun {
            id: "act-1".to_owned(),
            kind: ActionRunKind::TradingKillSwitch,
            status: ActionRunStatus::Accepted,
            actor: "127.0.0.1".to_owned(),
            target: Some("trading".to_owned()),
            request_id: Some("req-1".to_owned()),
            idempotency_key: None,
            message: "accepted".to_owned(),
            problem: None,
            result: None,
            mutation: None,
            started_at_ms: 1,
            updated_at_ms: 1,
        };

        let text = serde_json::to_string(&run).expect("serialize action run");

        assert!(text.contains("\"requestId\":\"req-1\""));
        assert!(text.contains("\"startedAtMs\":1"));
        assert!(text.contains("\"trading_kill_switch\""));
    }

    #[test]
    fn action_mutation_diff_is_typed_and_legacy_optional() {
        let diff = ActionMutationDiff {
            effective_at_ms: 42,
            changes: vec![ActionMutationChange::KillSwitchActive {
                before: false,
                after: true,
            }],
        };
        let encoded = serde_json::to_value(&diff).expect("mutation diff encodes");

        assert_eq!(encoded["effectiveAtMs"], 42);
        assert_eq!(encoded["changes"][0]["field"], "kill_switch_active");
        assert_eq!(encoded["changes"][0]["before"], false);
        assert_eq!(encoded["changes"][0]["after"], true);

        let legacy: ActionRun = serde_json::from_value(serde_json::json!({
            "id": "act-legacy",
            "kind": "trading_kill_switch",
            "status": "succeeded",
            "actor": "system",
            "message": "done",
            "startedAtMs": 1,
            "updatedAtMs": 2
        }))
        .expect("legacy action run decodes");
        assert!(legacy.mutation.is_none());
    }

    #[test]
    fn action_mutation_variants_keep_the_wire_contract() {
        let auto_close = AutoProfitCloseConfig::default();
        let changes = vec![
            ActionMutationChange::Adapter {
                before: "a".into(),
                after: "b".into(),
            },
            ActionMutationChange::ExecutionEnvironment {
                before: "paper".into(),
                after: "live".into(),
            },
            ActionMutationChange::LiveTradingEnabled {
                before: false,
                after: true,
            },
            ActionMutationChange::KillSwitchActive {
                before: false,
                after: true,
            },
            ActionMutationChange::MaxOrderNotional {
                before: 1.0,
                after: 2.0,
            },
            ActionMutationChange::MaxOpenOrders {
                before: 1,
                after: 2,
            },
            ActionMutationChange::MaxHedgeImbalancePct {
                before: 1.0,
                after: 2.0,
            },
            ActionMutationChange::LiquidationWarnPct {
                before: 1.0,
                after: 2.0,
            },
            ActionMutationChange::LiquidationDangerPct {
                before: 1.0,
                after: 2.0,
            },
            ActionMutationChange::AllowedExchanges {
                before: vec!["a".into()],
                after: vec!["b".into()],
            },
            ActionMutationChange::AllowedSymbols {
                before: vec!["A".into()],
                after: vec!["B".into()],
            },
            ActionMutationChange::ProtectedPositions {
                before: Vec::new(),
                after: vec![ProtectedPositionFingerprint {
                    venue: "binance".into(),
                    canonical_symbol: "btc".into(),
                    native_symbol: "btcusdt".into(),
                    side: "long".into(),
                    quantity: 0.232,
                    entry_price: 64_456.2,
                    position_mode: Some("both".into()),
                    opening_identity: "preexisting-binance-btc-long".into(),
                    source: "account_position_runtime".into(),
                    captured_at_ms: 42,
                }],
            },
            ActionMutationChange::AutoProfitClose {
                before: auto_close.clone(),
                after: auto_close,
            },
        ];

        for change in changes {
            let encoded = serde_json::to_value(&change).expect("mutation serializes");
            let decoded: ActionMutationChange =
                serde_json::from_value(encoded).expect("mutation deserializes");
            assert_eq!(decoded, change);
        }
    }
}
