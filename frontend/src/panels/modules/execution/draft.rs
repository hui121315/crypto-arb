use leptos::prelude::*;
use shared_types::{ApiProblem, ExecutionRun, HedgeTicketView, OrderRecord};

use crate::api::ws::WsChannelState;
use crate::state::load_state::LoadState;

use super::data::{
    self, all_orders_memo, order_seed_problem_memo, order_stream_problem_memo, orders_for_run_memo,
    use_order_queue, ConfirmActionRuntime, ExecutionPreview, ExecutionRuntime, PreviewSignals,
    WorkflowViewSource,
};
use crate::panels::modules::ExecutionSelection;

#[path = "draft/inputs.rs"]
pub(in crate::panels::modules::execution) mod inputs;
pub(super) use inputs::format_price;
use inputs::{persist_inputs, sync_defaults, sync_notional_from_margin, sync_reference_prices};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct ExecutionDraft {
    pub capital_usd: RwSignal<String>,
    pub leverage: RwSignal<String>,
    pub order_type: RwSignal<String>,
    pub limit_offset_bps: RwSignal<String>,
    pub long_price: RwSignal<String>,
    pub short_price: RwSignal<String>,
    pub long_notional_usd: RwSignal<String>,
    pub short_notional_usd: RwSignal<String>,
    pub time_in_force: RwSignal<String>,
    pub margin_mode: RwSignal<String>,
    pub preview_nonce: RwSignal<u64>,
    pub runtime_refresh_nonce: RwSignal<u64>,
    pub preview_state: RwSignal<LoadState<ExecutionPreview>>,
    pub preview: Memo<ExecutionPreview>,
    pub orders: Memo<Vec<OrderRecord>>,
    pub all_orders: Memo<Vec<OrderRecord>>,
    pub order_seed_problem: Memo<Option<ApiProblem>>,
    pub order_seed_ready: Memo<bool>,
    pub order_details: data::OrderDetails,
    pub order_stream_problem: Memo<Option<ApiProblem>>,
    pub order_channel_state: RwSignal<WsChannelState>,
    pub execution_run: RwSignal<Option<ExecutionRun>>,
    pub execution_run_seed_problem: RwSignal<Option<ApiProblem>>,
    pub execution_run_stream_problem: RwSignal<Option<ApiProblem>>,
    pub execution_run_channel_state: RwSignal<WsChannelState>,
    pub workflow_view: RwSignal<Option<HedgeTicketView>>,
    pub workflow_provenance: RwSignal<WorkflowViewSource>,
    pub confirm: ConfirmActionRuntime,
    pub order_queue: RwSignal<super::data::OrderQueue>,
}

impl ExecutionDraft {
    pub(super) fn new(selection: Memo<ExecutionSelection>, runtime: ExecutionRuntime) -> Self {
        let inputs = runtime.draft_inputs(&selection.get_untracked());
        let preview_nonce = runtime.preview_nonce;
        let runtime_refresh_nonce = runtime.runtime_refresh_nonce;
        persist_inputs(inputs);
        sync_defaults(selection, inputs);
        sync_notional_from_margin(inputs);
        let signals = PreviewSignals {
            selection,
            selection_state: runtime.selection(),
            capital_usd: inputs.capital_usd,
            leverage: inputs.leverage,
            order_type: inputs.order_type,
            limit_offset_bps: inputs.limit_offset_bps,
            long_price: inputs.long_price,
            short_price: inputs.short_price,
            long_notional_usd: inputs.long_notional_usd,
            short_notional_usd: inputs.short_notional_usd,
            margin_mode: inputs.margin_mode,
            time_in_force: inputs.time_in_force,
        };
        let (preview_state, preview) = data::use_preview(
            signals,
            runtime.workflow,
            preview_nonce,
            runtime.preview_state,
        );
        sync_reference_prices(preview, inputs);
        let order_queue = use_order_queue(
            runtime.order_queue,
            runtime.order_channel_state,
            runtime_refresh_nonce,
        );
        let order_seed_problem = order_seed_problem_memo(order_queue);
        let order_seed_ready = data::order_seed_ready_memo(order_queue);
        let order_details = data::use_run_order_details(runtime.run.run, order_queue, order_seed_ready);
        let order_stream_problem = order_stream_problem_memo(order_queue);
        let execution_run_feed = data::use_execution_run_updates(
            selection,
            runtime.run,
            runtime.workflow,
            runtime_refresh_nonce,
            runtime.confirm.recovery,
            runtime.confirm.state,
        );
        let orders = orders_for_run_memo(order_queue, execution_run_feed.run);
        let all_orders = all_orders_memo(order_queue);
        Self {
            capital_usd: inputs.capital_usd,
            leverage: inputs.leverage,
            order_type: inputs.order_type,
            limit_offset_bps: inputs.limit_offset_bps,
            long_price: inputs.long_price,
            short_price: inputs.short_price,
            long_notional_usd: inputs.long_notional_usd,
            short_notional_usd: inputs.short_notional_usd,
            time_in_force: inputs.time_in_force,
            margin_mode: inputs.margin_mode,
            preview_nonce,
            runtime_refresh_nonce,
            preview_state,
            preview,
            orders,
            all_orders,
            order_seed_problem,
            order_seed_ready,
            order_details,
            order_stream_problem,
            order_channel_state: runtime.order_channel_state,
            execution_run: execution_run_feed.run,
            execution_run_seed_problem: execution_run_feed.seed_problem,
            execution_run_stream_problem: execution_run_feed.stream_problem,
            execution_run_channel_state: execution_run_feed.channel_state,
            workflow_view: runtime.workflow.view,
            workflow_provenance: runtime.workflow.provenance,
            confirm: runtime.confirm,
            order_queue,
        }
    }
}
