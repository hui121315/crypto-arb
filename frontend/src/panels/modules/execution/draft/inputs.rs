//! 执行草案的输入信号、localStorage 持久化/恢复与默认值同步效应，以及名义金额/价格纯派生。
//! `ExecutionDraft` 结构与组装见父模块 `draft.rs`。

use leptos::prelude::*;

use crate::state::module_runtime::{store_choice, stored_choice};

use super::super::data;
use super::super::selection::ExecutionSelection;
use super::ExecutionPreview;

#[path = "inputs/selection_key.rs"]
mod selection_key;
use selection_key::execution_selection_key;

#[cfg(test)]
#[path = "inputs/tests.rs"]
mod tests;

const DRAFT_SELECTION_KEY: &str = "crossline.execution.draft.selection";
const DRAFT_CAPITAL_KEY: &str = "crossline.execution.draft.capital";
const DRAFT_LEVERAGE_KEY: &str = "crossline.execution.draft.leverage";
const DRAFT_ORDER_TYPE_KEY: &str = "crossline.execution.draft.orderType";
const DRAFT_LIMIT_OFFSET_KEY: &str = "crossline.execution.draft.limitOffsetBps";
const DRAFT_LONG_PRICE_KEY: &str = "crossline.execution.draft.longPrice";
const DRAFT_SHORT_PRICE_KEY: &str = "crossline.execution.draft.shortPrice";
const DRAFT_LONG_NOTIONAL_KEY: &str = "crossline.execution.draft.longNotional";
const DRAFT_SHORT_NOTIONAL_KEY: &str = "crossline.execution.draft.shortNotional";
const DRAFT_TIME_IN_FORCE_KEY: &str = "crossline.execution.draft.timeInForce";
const DRAFT_MARGIN_MODE_KEY: &str = "crossline.execution.draft.marginMode";

#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct DraftInputs {
    selection_key: RwSignal<String>,
    last_margin_key: RwSignal<String>,
    pub(super) capital_usd: RwSignal<String>,
    pub(super) leverage: RwSignal<String>,
    pub(super) order_type: RwSignal<String>,
    pub(super) limit_offset_bps: RwSignal<String>,
    pub(super) long_price: RwSignal<String>,
    pub(super) short_price: RwSignal<String>,
    pub(super) long_notional_usd: RwSignal<String>,
    pub(super) short_notional_usd: RwSignal<String>,
    pub(super) time_in_force: RwSignal<String>,
    pub(super) margin_mode: RwSignal<String>,
}

impl DraftInputs {
    pub(in crate::panels::modules::execution) fn new(selection: &ExecutionSelection) -> Self {
        let selection_key = execution_selection_key(selection);
        let restore =
            can_restore_draft(stored_text(DRAFT_SELECTION_KEY).as_deref(), &selection_key);
        let capital = draft_value(
            restore,
            DRAFT_CAPITAL_KEY,
            data::default_capital_text(selection),
        );
        let leverage = draft_value(
            restore,
            DRAFT_LEVERAGE_KEY,
            data::default_leverage_text(selection),
        );
        let notional = if restore {
            draft_value(
                true,
                DRAFT_LONG_NOTIONAL_KEY,
                default_notional_text(selection),
            )
        } else {
            default_notional_text(selection)
        };
        Self {
            selection_key: RwSignal::new(selection_key),
            last_margin_key: RwSignal::new(margin_key(&capital, &leverage)),
            capital_usd: RwSignal::new(capital),
            leverage: RwSignal::new(leverage),
            order_type: RwSignal::new(draft_value(restore, DRAFT_ORDER_TYPE_KEY, "Limit")),
            limit_offset_bps: RwSignal::new(draft_value(
                restore,
                DRAFT_LIMIT_OFFSET_KEY,
                data::default_limit_offset_text(selection),
            )),
            long_price: RwSignal::new(draft_value(
                restore,
                DRAFT_LONG_PRICE_KEY,
                selection.long_price_label.clone(),
            )),
            short_price: RwSignal::new(draft_value(
                restore,
                DRAFT_SHORT_PRICE_KEY,
                selection.short_price_label.clone(),
            )),
            long_notional_usd: RwSignal::new(notional.clone()),
            short_notional_usd: RwSignal::new(draft_value(
                restore,
                DRAFT_SHORT_NOTIONAL_KEY,
                notional,
            )),
            time_in_force: RwSignal::new(draft_value(restore, DRAFT_TIME_IN_FORCE_KEY, "IOC")),
            margin_mode: RwSignal::new(draft_value(restore, DRAFT_MARGIN_MODE_KEY, "Cross")),
        }
    }

    pub(in crate::panels::modules::execution) fn apply_selection(
        self,
        selection: &ExecutionSelection,
    ) {
        let next_key = execution_selection_key(selection);
        if self.selection_key.get_untracked() == next_key {
            return;
        }
        let restore = can_restore_draft(stored_text(DRAFT_SELECTION_KEY).as_deref(), &next_key);
        let capital = draft_value(
            restore,
            DRAFT_CAPITAL_KEY,
            data::default_capital_text(selection),
        );
        let leverage = draft_value(
            restore,
            DRAFT_LEVERAGE_KEY,
            data::default_leverage_text(selection),
        );
        let notional = draft_value(
            restore,
            DRAFT_LONG_NOTIONAL_KEY,
            default_notional_text(selection),
        );
        self.selection_key.set(next_key);
        self.capital_usd.set(capital.clone());
        self.leverage.set(leverage.clone());
        self.limit_offset_bps.set(draft_value(
            restore,
            DRAFT_LIMIT_OFFSET_KEY,
            data::default_limit_offset_text(selection),
        ));
        self.long_price.set(draft_value(
            restore,
            DRAFT_LONG_PRICE_KEY,
            selection.long_price_label.clone(),
        ));
        self.short_price.set(draft_value(
            restore,
            DRAFT_SHORT_PRICE_KEY,
            selection.short_price_label.clone(),
        ));
        self.long_notional_usd.set(notional.clone());
        self.short_notional_usd
            .set(draft_value(restore, DRAFT_SHORT_NOTIONAL_KEY, notional));
        if restore {
            self.order_type
                .set(draft_value(true, DRAFT_ORDER_TYPE_KEY, "Limit"));
            self.time_in_force
                .set(draft_value(true, DRAFT_TIME_IN_FORCE_KEY, "IOC"));
            self.margin_mode
                .set(draft_value(true, DRAFT_MARGIN_MODE_KEY, "Cross"));
        } else {
            set_if_empty(self.time_in_force, "IOC");
            set_if_empty(self.order_type, "Limit");
            set_if_empty(self.margin_mode, "Cross");
        }
        self.last_margin_key.set(margin_key(&capital, &leverage));
    }
}

pub(super) fn sync_defaults(selection: Memo<ExecutionSelection>, inputs: DraftInputs) {
    Effect::new(move |_| {
        let selection = selection.get();
        inputs.apply_selection(&selection);
    });
}

pub(super) fn persist_inputs(inputs: DraftInputs) {
    Effect::new(move |_| {
        store_choice(DRAFT_SELECTION_KEY, &inputs.selection_key.get());
        store_choice(DRAFT_CAPITAL_KEY, &inputs.capital_usd.get());
        store_choice(DRAFT_LEVERAGE_KEY, &inputs.leverage.get());
        store_choice(DRAFT_ORDER_TYPE_KEY, &inputs.order_type.get());
        store_choice(DRAFT_LIMIT_OFFSET_KEY, &inputs.limit_offset_bps.get());
        store_choice(DRAFT_LONG_PRICE_KEY, &inputs.long_price.get());
        store_choice(DRAFT_SHORT_PRICE_KEY, &inputs.short_price.get());
        store_choice(DRAFT_LONG_NOTIONAL_KEY, &inputs.long_notional_usd.get());
        store_choice(DRAFT_SHORT_NOTIONAL_KEY, &inputs.short_notional_usd.get());
        store_choice(DRAFT_TIME_IN_FORCE_KEY, &inputs.time_in_force.get());
        store_choice(DRAFT_MARGIN_MODE_KEY, &inputs.margin_mode.get());
    });
}

fn set_if_empty(signal: RwSignal<String>, value: &str) {
    if signal.get_untracked().trim().is_empty() {
        signal.set(value.to_owned());
    }
}

pub(super) fn sync_reference_prices(preview: Memo<ExecutionPreview>, inputs: DraftInputs) {
    Effect::new(move |_| {
        let preview = preview.get();
        fill_price_if_missing(inputs.long_price, preview.long_reference_price);
        fill_price_if_missing(inputs.short_price, preview.short_reference_price);
    });
}

fn fill_price_if_missing(signal: RwSignal<String>, value: Option<f64>) {
    let Some(value) = value.filter(|value| value.is_finite() && *value > f64::EPSILON) else {
        return;
    };
    if has_positive_price(&signal.get_untracked()) {
        return;
    }
    signal.set(format_price(value));
}

fn has_positive_price(value: &str) -> bool {
    value
        .trim()
        .parse::<f64>()
        .is_ok_and(|price| price.is_finite() && price > f64::EPSILON)
}

pub(crate) fn format_price(value: f64) -> String {
    if value >= 100.0 {
        format!("{value:.2}")
    } else if value >= 1.0 {
        format!("{value:.4}")
    } else {
        format!("{value:.8}")
    }
}

fn default_notional_text(selection: &ExecutionSelection) -> String {
    let capital = data::default_capital_text(selection)
        .parse::<f64>()
        .unwrap_or(1.0);
    let leverage = data::default_leverage_text(selection)
        .parse::<f64>()
        .unwrap_or(1.0);
    format!("{:.0}", (capital * leverage).max(1.0))
}

pub(super) fn sync_notional_from_margin(inputs: DraftInputs) {
    Effect::new(move |_| {
        let capital = inputs.capital_usd.get();
        let leverage = inputs.leverage.get();
        let next_key = margin_key(&capital, &leverage);
        if inputs.last_margin_key.get_untracked() == next_key {
            return;
        }
        inputs.last_margin_key.set(next_key);
        let notional = notional_text(&capital, &leverage);
        inputs.long_notional_usd.set(notional.clone());
        inputs.short_notional_usd.set(notional);
    });
}

fn margin_key(capital: &str, leverage: &str) -> String {
    format!("{}:{}", capital.trim(), leverage.trim())
}

fn draft_value(restore: bool, key: &str, fallback: impl Into<String>) -> String {
    if restore {
        if let Some(value) = stored_text(key) {
            return value;
        }
    }
    fallback.into()
}

fn stored_text(key: &str) -> Option<String> {
    stored_choice(key, |value| Some(value.to_owned()))
}

fn can_restore_draft(stored_key: Option<&str>, current_key: &str) -> bool {
    stored_key.is_some_and(|stored| stored == current_key)
}

fn notional_text(capital: &str, leverage: &str) -> String {
    let capital = positive_number(capital).unwrap_or(1.0);
    let leverage = positive_number(leverage).unwrap_or(1.0);
    format!("{:.0}", (capital * leverage).max(1.0))
}

fn positive_number(value: &str) -> Option<f64> {
    let value = value.trim().parse::<f64>().ok()?;
    (value.is_finite() && value > f64::EPSILON).then_some(value)
}
