//! Spot v1 手动调试 hook：按 symbol 手动查询 `/api/v1/spot/ticks` envelope。
//!
//! 只读诊断用途，不进入交易/收益/排序；`spot_v1` gate off 时会显示 typed 404
//! problem，而不是伪装成空数据。

use super::resources::settings_read;
use crate::state::load_state::LoadState;
use crate::state::read_scope::ReadScope;
use leptos::prelude::*;
use shared_types::{MarketDataEnvelope, SpotTicksPage};

pub(in crate::panels::modules::settings) type SpotTicksEnvelope = MarketDataEnvelope<SpotTicksPage>;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct SpotDebugQuery {
    /// `None` = 尚未发起查询；`Some(LoadState)` = 最近一次查询的落态。
    pub state: RwSignal<Option<LoadState<SpotTicksEnvelope>>>,
    pub submit: Callback<String>,
}

pub(in crate::panels::modules::settings) fn use_spot_debug_query() -> SpotDebugQuery {
    let state = RwSignal::new(None::<LoadState<SpotTicksEnvelope>>);
    let scope = ReadScope::new(move || state.set(None));
    let request = scope.request();
    Effect::new(move |_| {
        scope.track();
        request.cancel();
    });
    let submit = Callback::new(move |symbol: String| {
        state.set(Some(LoadState::Loading));
        request.run(move |client| async move {
            settings_read(client.spot_ticks(&symbol)).await
        }, move |outcome| {
            state.set(Some(match outcome {
                Ok(envelope) => LoadState::Ready(envelope),
                Err(error) => LoadState::Error(error.problem),
            }));
        });
    });
    SpotDebugQuery { state, submit }
}
