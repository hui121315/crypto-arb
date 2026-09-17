use super::*;

#[test]
fn restored_run_is_labeled_as_recent_without_an_active_selection() {
    let idle = ActionState::Idle;
    let run = run();

    assert_eq!(
        contextual_run_label(&idle, Some(&run), false),
        "最近执行 · 第二腿已提交，等待成交确认"
    );
    assert_eq!(
        contextual_run_label(&idle, Some(&run), true),
        "第二腿已提交，等待成交确认"
    );
    assert_eq!(contextual_run_label(&idle, None, false), "草案待提交");
    assert_eq!(
        contextual_run_label(&ActionState::succeeded("配对平仓已完成"), None, false),
        "最近执行 · 配对平仓已完成"
    );
}

#[test]
fn action_detail_does_not_render_placeholder_pair() {
    assert_eq!(action_detail(&ActionState::Idle, "-", None), "草案可编辑");
    assert_eq!(
        action_detail(&ActionState::succeeded("配对平仓已完成"), "-", None),
        ""
    );
}
