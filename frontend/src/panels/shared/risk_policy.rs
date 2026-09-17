pub const KILL_SWITCH_POLICY_LABEL: &str =
    "总闸开启后：阻止非 reduce-only 新订单；保留 reduce-only 平仓与撤单；不会自动撤销现有挂单";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_switch_policy_names_open_close_cancel_and_existing_order_boundaries() {
        assert!(KILL_SWITCH_POLICY_LABEL.contains("非 reduce-only 新订单"));
        assert!(KILL_SWITCH_POLICY_LABEL.contains("reduce-only 平仓与撤单"));
        assert!(KILL_SWITCH_POLICY_LABEL.contains("不会自动撤销现有挂单"));
    }
}
