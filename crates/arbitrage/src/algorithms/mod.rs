//! 套利引擎算法子模块。

pub mod cost_model;
pub mod cross_exchange;
pub mod cross_spot_perp;
pub mod fee_evidence;
pub mod funding_carry;
pub(crate) mod funding_timeline;
pub mod futures_fields;
pub(crate) mod leg_market_evidence;
pub(crate) mod market;
pub(crate) mod market_index;
pub mod normalizer;
pub mod options_perp_basis;
pub(crate) mod perp_cross_policy;
pub mod perp_price_spread;
pub mod position_optimizer;
pub(crate) mod price_spread_history;
pub(crate) mod quote_conversion;
pub mod spot_cross;
pub mod spot_perp;
pub mod time_value;
pub mod triangular;
