use super::*;
use shared_types::{
    FeeProduct, FeeScheduleRegistryResponse, FeeScheduleRegistryRow, FeeScheduleVenue, VenueId,
};

pub(super) fn fee_schedule_panel(
    state: LoadState<FeeScheduleRegistryResponse>,
    venue_id: &str,
) -> AnyView {
    let (response, stale_problem) = match state {
        LoadState::Ready(response) => (response, None),
        LoadState::Stale { value, problem } => (value, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取费率表证据失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取费率表证据"</div> }.into_any();
        }
    };
    let stale_message = fee_schedule_stale_message(stale_problem.as_ref());
    let registry_version = response.schema.version.clone();
    let registry_fingerprint = response.schema.fingerprint.clone();
    let Some(row) = fee_schedule_venue_from_response(response, venue_id) else {
        return view! {
            <>
                {stale_message.map(|message| view! {
                    <em class="settings-message is-error">{message}</em>
                })}
                <div class="empty-cell">"未找到该交易所费率表证据"</div>
            </>
        }
        .into_any();
    };
    let count = row.schedules.len();
    view! {
        <>
            {stale_message.map(|message| view! {
                <em class="settings-message is-error">{message}</em>
            })}
            <div class="ws-venue-panel">
                <div class="ws-venue-head">
                    <div>
                        <strong>"Fee schedule fixture 注册表 · " {registry_version}</strong>
                        <em>"官方费率表 fixture；账户/VIP/折扣费率仍以账户 API 快照为准。"</em>
                    </div>
                    <span class="num" title=registry_fingerprint>{count} " schedules"</span>
                </div>
                <div class="table-wrap">
                    <table class="clean-table settings-table" data-table-budget="bounded-small">
                        <thead>
                            <tr>
                                <th>"产品"</th>
                                <th>"Maker / Taker"</th>
                                <th>"版本 / tier"</th>
                                <th>"fixture"</th>
                                <th>"范围"</th>
                            </tr>
                        </thead>
                        <tbody>
                            {row.schedules.into_iter().map(fee_schedule_row).collect_view()}
                        </tbody>
                    </table>
                </div>
            </div>
        </>
    }
    .into_any()
}

fn fee_schedule_row(row: FeeScheduleRegistryRow) -> AnyView {
    let evidence = row.evidence;
    let doc_url = evidence.source_url.clone();
    let schedule = evidence.schedule_version.unwrap_or_else(|| "-".to_owned());
    let tier = evidence.tier.unwrap_or_else(|| "-".to_owned());
    let scope = evidence.scope.unwrap_or_else(|| "-".to_owned());
    let fixture = format!("{} · {}", row.fixture_id, row.fixture_symbol);
    let fee_values = format!("{:.4} / {:.4}", row.maker_fee_bps, row.taker_fee_bps);
    view! {
        <tr>
            <td>{fee_product_label(row.product)}</td>
            <td>{fee_values}</td>
            <td>
                <a href=doc_url target="_blank" rel="noreferrer">{schedule}</a>
                <br/>
                <span title=evidence.evidence_id>{tier}</span>
            </td>
            <td>{fixture}</td>
            <td>{scope}</td>
        </tr>
    }
    .into_any()
}

pub(super) fn fee_schedule_venue_from_response(
    response: FeeScheduleRegistryResponse,
    venue_id: &str,
) -> Option<FeeScheduleVenue> {
    let family = VenueId::from_exchange_name(venue_id)?;
    response
        .venues
        .into_iter()
        .find(|row| row.venue == family.as_str())
}

pub(super) fn fee_schedule_stale_message(problem: Option<&ApiProblem>) -> Option<String> {
    problem.map(|problem| problem_message("费率表证据刷新失败，显示上次结果", problem))
}

fn fee_product_label(product: FeeProduct) -> &'static str {
    match product {
        FeeProduct::Spot => "Spot",
        FeeProduct::Perp => "Perp",
        FeeProduct::Margin => "Margin",
        FeeProduct::Unknown => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_with(venue: &str) -> FeeScheduleRegistryResponse {
        FeeScheduleRegistryResponse {
            schema: Default::default(),
            venues: vec![FeeScheduleVenue {
                venue: venue.to_owned(),
                schedules: Vec::new(),
            }],
        }
    }

    #[test]
    fn fee_schedule_venue_matches_family_for_scoped_venue_names() {
        assert!(fee_schedule_venue_from_response(
            response_with("hyperliquid"),
            "hyperliquid:builder"
        )
        .is_some());
        assert!(fee_schedule_venue_from_response(response_with("kucoin"), "kucoin").is_some());
    }

    #[test]
    fn fee_schedule_venue_missing_or_unknown_returns_none() {
        assert!(fee_schedule_venue_from_response(response_with("okx"), "binance").is_none());
        assert!(fee_schedule_venue_from_response(response_with("okx"), "not-a-venue").is_none());
    }
}
