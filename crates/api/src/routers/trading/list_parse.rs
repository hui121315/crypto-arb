use super::*;

pub(super) fn execution_ledger_limit(
    requested: Option<usize>,
    problems: &mut Vec<ApiProblem>,
) -> usize {
    match requested {
        None => EXECUTION_LEDGER_LIST_DEFAULT_LIMIT,
        Some(0) => {
            problems.push(execution_ledger_problem(
                codes::LIST_LIMIT_CLAMPED,
                "execution ledger limit was raised to minimum",
                serde_json::json!({ "requested": 0, "applied": 1 }),
            ));
            1
        }
        Some(limit) if limit > EXECUTION_LEDGER_LIST_MAX_LIMIT => {
            problems.push(execution_ledger_problem(
                codes::LIST_LIMIT_CLAMPED,
                "execution ledger limit was clamped to maximum",
                serde_json::json!({
                    "requested": limit,
                    "applied": EXECUTION_LEDGER_LIST_MAX_LIMIT,
                }),
            ));
            EXECUTION_LEDGER_LIST_MAX_LIMIT
        }
        Some(limit) => limit,
    }
}

pub(super) fn execution_ledger_timestamp(
    value: Option<&str>,
    field: &'static str,
    problems: &mut Vec<ApiProblem>,
) -> Option<i64> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    match value.parse::<i64>() {
        Ok(timestamp) if timestamp >= 0 => Some(timestamp),
        _ => {
            problems.push(execution_ledger_invalid_filter(field, value));
            None
        }
    }
}

pub(super) fn clean_query_token(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn execution_ledger_leg_role(
    value: Option<&str>,
    problems: &mut Vec<ApiProblem>,
) -> Option<HedgeLegRole> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let token = value.replace('-', "_").to_ascii_lowercase();
    if let Ok(role) = serde_json::from_value::<HedgeLegRole>(serde_json::Value::String(token)) {
        return Some(role);
    }
    problems.push(execution_ledger_invalid_filter("legRole", value));
    None
}

pub(super) fn execution_ledger_invalid_filter(filter: &'static str, value: &str) -> ApiProblem {
    execution_ledger_problem(
        codes::LIST_FILTER_INVALID,
        "execution ledger filter was invalid",
        serde_json::json!({ "filter": filter, "value": value }),
    )
}

pub(super) fn execution_ledger_problem(
    code: &'static str,
    message: &'static str,
    details: serde_json::Value,
) -> ApiProblem {
    let mut problem = ApiProblem::new(code, message).with_source(EXECUTION_LEDGER_LIST_SOURCE);
    problem.details = Some(details);
    problem
}

pub(super) fn order_list_limit(requested: Option<usize>, problems: &mut Vec<ApiProblem>) -> usize {
    match requested {
        None => ORDER_LIST_DEFAULT_LIMIT,
        Some(0) => {
            problems.push(order_list_problem(
                codes::LIST_LIMIT_CLAMPED,
                "order list limit was raised to minimum",
                serde_json::json!({ "requested": 0, "applied": 1 }),
            ));
            1
        }
        Some(limit) if limit > ORDER_LIST_MAX_LIMIT => {
            problems.push(order_list_problem(
                codes::LIST_LIMIT_CLAMPED,
                "order list limit was clamped to maximum",
                serde_json::json!({
                    "requested": limit,
                    "applied": ORDER_LIST_MAX_LIMIT,
                }),
            ));
            ORDER_LIST_MAX_LIMIT
        }
        Some(limit) => limit,
    }
}

pub(super) fn order_list_offset(cursor: Option<&str>, problems: &mut Vec<ApiProblem>) -> usize {
    let Some(cursor) = cursor.filter(|value| !value.trim().is_empty()) else {
        return 0;
    };
    if let Ok(offset) = cursor.parse::<usize>() {
        return offset;
    }
    problems.push(order_list_problem(
        codes::LIST_CURSOR_INVALID,
        "order list cursor was invalid",
        serde_json::json!({ "cursor": cursor, "applied": 0 }),
    ));
    0
}

pub(super) fn order_list_state(
    value: Option<&str>,
    problems: &mut Vec<ApiProblem>,
) -> Option<LiveOrderState> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let Some(token) = normalized_order_state_token(value) else {
        problems.push(order_list_invalid_filter("state", value));
        return None;
    };
    if let Ok(state) = serde_json::from_value::<LiveOrderState>(serde_json::Value::String(token)) {
        Some(state)
    } else {
        problems.push(order_list_invalid_filter("state", value));
        None
    }
}

pub(super) fn order_list_since_ms(
    value: Option<&str>,
    problems: &mut Vec<ApiProblem>,
) -> Option<i64> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    match value.parse::<i64>() {
        Ok(timestamp) if timestamp >= 0 => Some(timestamp),
        _ => {
            problems.push(order_list_invalid_filter("sinceMs", value));
            None
        }
    }
}

pub(super) fn normalized_order_state_token(value: &str) -> Option<String> {
    let mut token = String::with_capacity(value.len());
    for (index, ch) in value.chars().enumerate() {
        match ch {
            '-' | ' ' => token.push('_'),
            '_' => token.push('_'),
            ch if ch.is_ascii_uppercase() => {
                if index > 0 && !token.ends_with('_') {
                    token.push('_');
                }
                token.push(ch.to_ascii_lowercase());
            }
            ch if ch.is_ascii_lowercase() || ch.is_ascii_digit() => token.push(ch),
            _ => return None,
        }
    }
    (!token.is_empty()).then_some(token)
}

pub(super) fn order_list_invalid_filter(filter: &'static str, value: &str) -> ApiProblem {
    order_list_problem(
        codes::LIST_FILTER_INVALID,
        "order list filter was invalid",
        serde_json::json!({ "filter": filter, "value": value }),
    )
}

pub(super) fn order_list_problem(
    code: &'static str,
    message: &'static str,
    details: serde_json::Value,
) -> ApiProblem {
    let mut problem = ApiProblem::new(code, message).with_source(ORDER_LIST_SOURCE);
    problem.details = Some(details);
    problem
}
