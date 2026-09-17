pub(crate) fn leg_label(exchange: &str, action: &str) -> String {
    let exchange = exchange.trim();
    let action = action.trim();
    if action.is_empty() {
        exchange.to_owned()
    } else if exchange.is_empty() || action_contains_exchange(action, exchange) {
        action.to_owned()
    } else {
        format!("{exchange} · {action}")
    }
}

fn action_contains_exchange(action: &str, exchange: &str) -> bool {
    action
        .to_ascii_lowercase()
        .contains(&exchange.to_ascii_lowercase())
}
