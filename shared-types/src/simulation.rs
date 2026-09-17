//! Legacy simulation API contracts.
//!
//! These contracts intentionally keep the `Simulation` prefix even inside this
//! module so searches for production portfolio types cannot confuse the two
//! surfaces.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationRuntimeMeta {
    pub source: String,
    pub persistence: SimulationPersistenceMode,
    pub volatile: bool,
    pub observed_at_ms: i64,
    pub freshness_ms: u64,
    pub model: String,
    pub warning: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulationPersistenceMode {
    MemoryOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationPortfolioSummary {
    pub initial_capital: f64,
    pub cash: f64,
    pub total_equity: f64,
    pub total_unrealized_pnl: f64,
    pub total_realized_pnl: f64,
    pub return_pct: f64,
    pub position_count: usize,
    pub trade_count: usize,
    pub positions: Vec<SimulationPosition>,
    pub runtime: SimulationRuntimeMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationPosition {
    pub id: String,
    pub symbol: String,
    pub exchange: String,
    pub side: String,
    pub quantity: f64,
    pub entry_price: f64,
    pub current_price: f64,
    pub leverage: f64,
    pub opened_at: String,
    pub fees_paid: f64,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationOpenRequest {
    pub symbol: String,
    pub exchange: String,
    pub side: String,
    pub quantity: f64,
    pub entry_price: f64,
    #[serde(default = "simulation_default_leverage")]
    pub leverage: f64,
    #[serde(default)]
    pub fees: f64,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationOpenResponse {
    pub position: SimulationPosition,
    pub runtime: SimulationRuntimeMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationCloseResponse {
    pub realized_pnl: f64,
    pub new_cash: f64,
    pub new_total_equity: f64,
    pub runtime: SimulationRuntimeMeta,
}

pub fn simulation_default_leverage() -> f64 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_fixture() -> SimulationRuntimeMeta {
        SimulationRuntimeMeta {
            source: "simulation".to_owned(),
            persistence: SimulationPersistenceMode::MemoryOnly,
            volatile: true,
            observed_at_ms: 1_000,
            freshness_ms: 50,
            model: "legacy".to_owned(),
            warning: "内存态，不代表实盘".to_owned(),
        }
    }

    fn position_fixture() -> SimulationPosition {
        SimulationPosition {
            id: "sim-1".to_owned(),
            symbol: "BTCUSDT".to_owned(),
            exchange: "binance".to_owned(),
            side: "long".to_owned(),
            quantity: 0.5,
            entry_price: 50_000.0,
            current_price: 50_100.0,
            leverage: 2.0,
            opened_at: "2026-06-30T00:00:00Z".to_owned(),
            fees_paid: 1.2,
            note: String::new(),
        }
    }

    #[test]
    fn open_response_serializes_camel_case_contract() {
        let response = SimulationOpenResponse {
            position: position_fixture(),
            runtime: runtime_fixture(),
        };

        let json = serde_json::to_value(&response).expect("serialize open response");

        assert_eq!(json["position"]["entryPrice"], 50_000.0);
        assert_eq!(json["position"]["feesPaid"], 1.2);
        assert_eq!(json["runtime"]["persistence"], "memory_only");
        assert_eq!(json["runtime"]["observedAtMs"], 1_000);
    }

    #[test]
    fn close_response_serializes_camel_case_contract() {
        let response = SimulationCloseResponse {
            realized_pnl: 12.5,
            new_cash: 1_012.5,
            new_total_equity: 1_020.0,
            runtime: runtime_fixture(),
        };

        let json = serde_json::to_value(&response).expect("serialize close response");

        assert_eq!(json["realizedPnl"], 12.5);
        assert_eq!(json["newCash"], 1_012.5);
        assert_eq!(json["newTotalEquity"], 1_020.0);
        assert_eq!(json["runtime"]["freshnessMs"], 50);
    }

    #[test]
    fn open_request_defaults_apply_on_minimal_payload() {
        let request: SimulationOpenRequest = serde_json::from_str(
            r#"{"symbol":"ETHUSDT","exchange":"okx","side":"short","quantity":1.0,"entryPrice":2500.0}"#,
        )
        .expect("deserialize minimal open request");

        assert_eq!(request.leverage, 1.0);
        assert_eq!(request.fees, 0.0);
        assert!(request.note.is_empty());
    }

    #[test]
    fn portfolio_summary_round_trips() {
        let summary = SimulationPortfolioSummary {
            initial_capital: 1_000.0,
            cash: 900.0,
            total_equity: 1_010.0,
            total_unrealized_pnl: 10.0,
            total_realized_pnl: 0.0,
            return_pct: 1.0,
            position_count: 1,
            trade_count: 3,
            positions: vec![position_fixture()],
            runtime: runtime_fixture(),
        };

        let text = serde_json::to_string(&summary).expect("serialize summary");
        let parsed: SimulationPortfolioSummary =
            serde_json::from_str(&text).expect("deserialize summary");

        assert_eq!(parsed.position_count, 1);
        assert_eq!(parsed.positions[0].symbol, "BTCUSDT");
        assert_eq!(parsed.runtime.model, "legacy");
    }

    #[test]
    fn contract_types_remain_in_the_simulation_namespace() {
        let names = [
            std::any::type_name::<SimulationRuntimeMeta>(),
            std::any::type_name::<SimulationPersistenceMode>(),
            std::any::type_name::<SimulationPortfolioSummary>(),
            std::any::type_name::<SimulationPosition>(),
            std::any::type_name::<SimulationOpenRequest>(),
            std::any::type_name::<SimulationOpenResponse>(),
            std::any::type_name::<SimulationCloseResponse>(),
        ];

        assert!(names
            .iter()
            .all(|name| name.contains("shared_types::simulation::Simulation")));
    }
}
