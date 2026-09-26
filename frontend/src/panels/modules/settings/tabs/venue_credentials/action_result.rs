use super::*;

#[derive(Clone, PartialEq, Eq)]
struct CredentialActionPresentation {
    tone: &'static str,
    status: String,
    title: &'static str,
    detail: String,
    technical: String,
}

pub(super) fn credential_save_result(
    selected: RwSignal<String>,
    selected_credential: Memo<Option<VenueCredentialStatus>>,
    state: RwSignal<ActionState>,
    idle_message: RwSignal<String>,
) -> impl IntoView {
    let presentation =
        Memo::new(move |_| credential_action_presentation(&state.get(), &idle_message.get()));
    let venue_label = Memo::new(move |_| {
        selected_credential
            .get()
            .map(|status| status.label)
            .unwrap_or_else(|| normalized_venue_name(&selected.get()))
    });

    view! {
        <section
            class=move || format!(
                "credential-action-result {}",
                presentation.get().tone,
            )
        >
            <div
                class="credential-action-result-copy"
                role="status"
                aria-live="polite"
                aria-atomic="true"
            >
                <span>{move || format!("{} · 本次保存", venue_label.get())}</span>
                <strong>{move || presentation.get().title}</strong>
                <em>{move || presentation.get().detail}</em>
            </div>
            <b>{move || presentation.get().status}</b>
            {move || credential_action_technical_view(&presentation.get().technical)}
        </section>
    }
}

fn credential_action_presentation(
    state: &ActionState,
    idle_message: &str,
) -> CredentialActionPresentation {
    match state {
        ActionState::Idle => CredentialActionPresentation {
            tone: "is-idle",
            status: "未提交".to_owned(),
            title: "等待保存",
            detail: idle_message.to_owned(),
            technical: String::new(),
        },
        ActionState::Pending { label, .. } => CredentialActionPresentation {
            tone: "is-pending",
            status: "进行中".to_owned(),
            title: "正在保存",
            detail: label.clone(),
            technical: state.message("凭证保存中"),
        },
        ActionState::Accepted { label, .. } => CredentialActionPresentation {
            tone: "is-accepted",
            status: "已接收".to_owned(),
            title: "等待保存最终结果",
            detail: label.clone(),
            technical: state.message("凭证保存已接收"),
        },
        ActionState::Succeeded { .. } => CredentialActionPresentation {
            tone: "is-succeeded",
            status: "成功".to_owned(),
            title: "保存完成",
            detail: "字段已保存；实盘可用性仍以上方保存期验证和下方运行状态数据依据为准。".to_owned(),
            technical: state.message("凭证保存完成"),
        },
        ActionState::Failed { label, problem, .. }
            if label == "保存结果待确认" || problem.code == "SETTINGS_RESULT_UNKNOWN" =>
        {
            CredentialActionPresentation {
                tone: "is-accepted",
                status: "待核对".into(),
                title: "保存结果待核对",
                detail: problem.message.clone(),
                technical: state.message("凭证保存结果未知"),
            }
        }
        ActionState::Failed { problem, .. } => CredentialActionPresentation {
            tone: "is-failed",
            status: if problem.code.trim().is_empty() {
                "失败".to_owned()
            } else {
                format!("失败 · {}", problem.code)
            },
            title: "保存失败",
            detail: problem.message.clone(),
            technical: state.message("凭证保存失败"),
        },
    }
}

fn credential_action_technical_view(technical: &str) -> AnyView {
    if technical.trim().is_empty() {
        return ().into_any();
    }
    view! {
        <details class="credential-action-evidence">
            <summary>"技术数据依据"</summary>
            <code>{technical.to_owned()}</code>
        </details>
    }
    .into_any()
}
