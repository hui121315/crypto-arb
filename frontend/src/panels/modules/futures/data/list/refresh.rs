use shared_types::StrategyKind;

pub(in crate::panels::modules::futures) fn futures_list_request_is_current(
    request_strategy: StrategyKind,
    request_cursor: Option<&str>,
    current_strategy: StrategyKind,
    current_cursor: Option<&str>,
) -> bool {
    request_strategy == current_strategy && request_cursor == current_cursor
}
