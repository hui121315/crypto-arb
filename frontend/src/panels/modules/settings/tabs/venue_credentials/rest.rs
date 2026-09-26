use super::*;
use shared_types::{RestEndpointVenue, RestEndpointsResponse, VenueId};

pub(super) fn rest_panel(state: LoadState<RestEndpointsResponse>, venue_id: &str) -> AnyView {
    let (response, stale_problem) = match state {
        LoadState::Ready(response) => (response, None),
        LoadState::Stale { value, problem } => (value, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取 REST endpoint 数据依据失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取 REST endpoint 数据依据"</div> }
                .into_any();
        }
    };
    let stale_message = rest_stale_message(stale_problem.as_ref());
    let Some(row) = rest_venue_from_response(response, venue_id) else {
        return view! {
            <>
                {stale_message.map(|message| view! {
                    <em class="settings-message is-error">{message}</em>
                })}
                <div class="empty-cell">"未找到该交易所 REST endpoint 数据依据"</div>
            </>
        }
        .into_any();
    };
    let count = row.endpoints.len();
    view! {
        <>
            {stale_message.map(|message| view! {
                <em class="settings-message is-error">{message}</em>
            })}
            <div class="ws-venue-panel">
                <div class="ws-venue-head">
                    <div>
                        <strong>"REST endpoint 数据依据注册表"</strong>
                        <em>"官方文档 doc_version/checked_at 与 schema fixture/测试绑定，非运行时探测。"</em>
                    </div>
                    <span class="num">{count} " endpoints"</span>
                </div>
                <div class="table-wrap">
                <table class="clean-table settings-table" data-table-budget="bounded-small">
                    <thead>
                        <tr>
                            <th>"Endpoint"</th>
                            <th>"文档版本"</th>
                            <th>"核对日期"</th>
                            <th>"schema fixture / 测试"</th>
                            <th>"用途 / 限频"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {row.endpoints.into_iter().map(rest_endpoint_row).collect_view()}
                    </tbody>
                </table>
                </div>
            </div>
        </>
    }
    .into_any()
}

fn rest_endpoint_row(row: shared_types::RestEndpointRow) -> AnyView {
    let doc_url = row.doc_urls.first().cloned().unwrap_or_default();
    let fixture = format!(
        "{} · {} / {}",
        row.fixture_id, row.parser_test, row.request_builder_test
    );
    let scope = format!(
        "{} · {} · 权重 {}",
        row.use_cases.join("/"),
        row.rate_scopes.join("/"),
        row.weight
    );
    view! {
        <tr>
            <td>{row.method} " " {row.path}</td>
            <td>
                <a href=doc_url target="_blank" rel="noreferrer">{row.doc_version}</a>
            </td>
            <td>{row.checked_at}</td>
            <td title=row.schema_hash>{fixture}</td>
            <td>{scope}</td>
        </tr>
    }
    .into_any()
}

pub(super) fn rest_venue_from_response(
    response: RestEndpointsResponse,
    venue_id: &str,
) -> Option<RestEndpointVenue> {
    let family = VenueId::from_exchange_name(venue_id)?;
    response
        .venues
        .into_iter()
        .find(|row| row.venue == family.as_str())
}

pub(super) fn rest_stale_message(problem: Option<&ApiProblem>) -> Option<String> {
    problem.map(|problem| problem_message("REST endpoint 数据依据刷新失败，显示上次结果", problem))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_with(venue: &str) -> RestEndpointsResponse {
        RestEndpointsResponse {
            venues: vec![RestEndpointVenue {
                venue: venue.to_owned(),
                endpoints: Vec::new(),
            }],
        }
    }

    #[test]
    fn rest_venue_matches_family_for_scoped_venue_names() {
        assert!(rest_venue_from_response(response_with("okx"), "okx-live").is_some());
        assert!(rest_venue_from_response(response_with("binance"), "binance").is_some());
    }

    #[test]
    fn rest_venue_missing_or_unknown_returns_none() {
        assert!(rest_venue_from_response(response_with("okx"), "binance").is_none());
        assert!(rest_venue_from_response(response_with("okx"), "not-a-venue").is_none());
    }
}
