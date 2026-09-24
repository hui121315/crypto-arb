//! Spot v1 手动调试 hook：按 symbol 手动查询 `/api/v1/spot/ticks` envelope。
//!
//! 只读诊断用途，不进入交易/收益/排序；`spot_v1` gate off 时会显示 typed 404
//! problem，而不是伪装成空数据。

use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{MarketDataEnvelope, SpotTicksPage};

pub(in crate::panels::modules::settings) type SpotTicksEnvelope = MarketDataEnvelope<SpotTicksPage>;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct SpotDebugQuery {
    /// `None` = 尚未发起查询；`Some(LoadState)` = 最近一次查询的落态。
    pub state: RwSignal<Option<LoadState<SpotTicksEnvelope>>>,
    pub submit: Callback<String>,
}

pub(in crate::panels::modules::settings) fn use_spot_debug_query() -> SpotDebugQuery {
    let client = use_global().client;
    let state = RwSignal::new(None::<LoadState<SpotTicksEnvelope>>);
    let version = StoredValue::new(0_u64);
    let submit = Callback::new(move |symbol: String| {
        let request_version = version.with_value(|value| value.wrapping_add(1));
        version.set_value(request_version);
        state.set(Some(LoadState::Loading));
        let client = client.clone();
        spawn_local(async move {
            let outcome = client.spot_ticks(&symbol).await;
            if state.is_disposed() || version.get_value() != request_version {
                return;
            }
            state.set(Some(match outcome {
                Ok(envelope) => LoadState::Ready(envelope),
                Err(error) => LoadState::Error(error.problem),
            }));
        });
    });
    SpotDebugQuery { state, submit }
}
