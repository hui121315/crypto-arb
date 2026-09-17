use super::*;

pub(super) fn env_template_panel(
    state: LoadState<EnvTemplateResponse>,
    table: &TableRuntimeHandle<EnvTemplateLine>,
) -> AnyView {
    let (template, template_problem) = match state {
        LoadState::Ready(template) => (template, None),
        LoadState::Stale {
            value: template,
            problem,
        } => (template, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取 .env 模板失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在生成 .env 模板"</div> }.into_any();
        }
    };
    let text = template.text;
    let line_count = template.lines.len();
    let runtime = table.runtime.get();
    let total = table.total;
    let current_page = table.current_page;
    let visible_rows = runtime
        .rows
        .into_iter()
        .map(env_template_row)
        .collect_view();
    view! {
        <>
            <label class="settings-control env-template-box">
                <span>".env 模板"</span>
                <textarea readonly prop:value=text/>
                <em>"不返回任何 secret 明文，只返回变量名。"</em>
            </label>
            {template_problem.map(|problem| view! {
                <em class="settings-message is-error">
                    {diagnostics_stale_problem_message(".env 模板刷新失败", &problem)}
                </em>
            })}
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"交易所"</th>
                            <th>"字段"</th>
                            <th>"环境变量"</th>
                            <th>"状态"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {
                            if line_count == 0 {
                                empty_table_row(4, "暂无 .env 模板字段")
                            } else {
                                visible_rows.into_any()
                            }
                        }
                    </tbody>
                </table>
            </div>
            {move || {
                (total.get() > ENV_TEMPLATE_PAGE_SIZE).then(|| view! {
                    {page_controls(total, current_page, ENV_TEMPLATE_PAGE_SIZE)}
                })
            }}
        </>
    }
    .into_any()
}

pub(super) fn env_template_row(line: EnvTemplateLine) -> impl IntoView {
    let status = if line.configured {
        "字段已填写"
    } else {
        "字段未填写"
    };
    view! {
        <tr>
            <td>{line.venue}</td>
            <td>{line.field_label}</td>
            <td>{line.key}</td>
            <td>{status}</td>
        </tr>
    }
}

pub(super) fn normalized(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

pub(super) fn normalized_search_query(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(super) fn stored_health_query(value: &str) -> Option<String> {
    Some(value.trim().chars().take(120).collect())
}

pub(super) fn stored_health_status_filter(value: &str) -> Option<HealthStatusFilter> {
    Some(HealthStatusFilter::from_key(value))
}

pub(super) fn api_base_apply_confirmed(value: &str) -> bool {
    value.trim() == API_BASE_APPLY_CONFIRMATION
}

pub(super) fn auth_status_label(configured: bool) -> &'static str {
    if configured {
        "Token 已填写"
    } else {
        "Token 未填写"
    }
}

pub(super) fn auth_apply_message(configured: bool) -> String {
    if configured {
        "后端 REST Bearer Token 已应用；WebSocket 将先换取短期 ticket 再订阅。".to_owned()
    } else {
        "后端 REST Bearer Token 已清空；WebSocket 不会发起未鉴权订阅。".to_owned()
    }
}

pub(super) fn empty_table_row(colspan: usize, text: &'static str) -> AnyView {
    view! {
        <tr>
            <td colspan=colspan.to_string()>{text}</td>
        </tr>
    }
    .into_any()
}
