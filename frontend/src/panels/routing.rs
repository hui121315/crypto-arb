use crate::panels::workstation::ModuleId;
use gloo_storage::Storage;
use leptos::prelude::{on_cleanup, window_event_listener_untyped, GetUntracked, RwSignal};
use shared_types::StrategyKind;

const ACTIVE_MODULE_STORAGE_KEY: &str = "crossline.activeModule";
const MAX_ROUTE_TOKEN_CHARS: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceRouteOrigin {
    Position,
}

impl WorkspaceRouteOrigin {
    fn from_query_value(value: &str) -> Option<Self> {
        match value {
            "position" => Some(Self::Position),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceRoute {
    pub module: ModuleId,
    pub symbol: Option<String>,
    pub strategy: Option<StrategyKind>,
    pub origin: Option<WorkspaceRouteOrigin>,
    pub page: Option<String>,
    pub opportunity_id: Option<String>,
    pub run_id: Option<String>,
    pub ticket_id: Option<String>,
}

impl WorkspaceRoute {
    fn for_module(module: ModuleId) -> Self {
        Self {
            module,
            symbol: None,
            strategy: None,
            origin: None,
            page: None,
            opportunity_id: None,
            run_id: None,
            ticket_id: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunRouteContext {
    pub run_id: String,
    pub ticket_id: Option<String>,
    pub opportunity_id: Option<String>,
}

impl RunRouteContext {
    pub(crate) fn from_route(route: &WorkspaceRoute) -> Option<Self> {
        Some(Self {
            run_id: route.run_id.clone()?,
            ticket_id: route.ticket_id.clone(),
            opportunity_id: route.opportunity_id.clone(),
        })
    }

    pub(crate) fn matches_position(&self, row: &shared_types::PositionRow) -> bool {
        row.pair_evidence.as_ref().is_some_and(|pair| {
            pair.run_id == self.run_id
                && self
                    .ticket_id
                    .as_ref()
                    .is_none_or(|id| *id == pair.ticket_id)
                && self
                    .opportunity_id
                    .as_ref()
                    .is_none_or(|id| *id == pair.opportunity_id)
                && pair.venue == row.venue
                && pair.side == row.side
        })
    }
}

pub(crate) fn execution_run_href(module: ModuleId, run: &shared_types::ExecutionRun) -> String {
    let params = web_sys::UrlSearchParams::new().expect("empty query parameters");
    params.append("run", &run.run_id);
    params.append("ticket", &run.ticket_id);
    params.append("opp", &run.opportunity_id);
    format!("#{}?{}", module.slug(), params.to_string())
}

impl ModuleId {
    pub(crate) const fn slug(self) -> &'static str {
        match self {
            Self::Positions => "positions",
            Self::Futures => "futures",
            Self::Opportunities => "opportunities",
            Self::GateCrossEx => "crossex",
            Self::Onchain => "onchain",
            Self::Stocks => "stocks",
            Self::Automation => "automation",
            Self::Execution => "execution",
            Self::Review => "review",
            Self::Settings => "settings",
        }
    }

    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Positions => "持仓/风控",
            Self::Futures => "期货套利",
            Self::Opportunities => "机会扫描",
            Self::GateCrossEx => "CrossEx",
            Self::Onchain => "链上套利",
            Self::Stocks => "股票套利",
            Self::Automation => "自动化",
            Self::Execution => "对冲执行",
            Self::Review => "复盘",
            Self::Settings => "设置",
        }
    }

    pub(crate) fn from_slug(slug: &str) -> Option<Self> {
        Some(match slug {
            "positions" => Self::Positions,
            "futures" => Self::Futures,
            "opportunities" => Self::Opportunities,
            "crossex" => Self::GateCrossEx,
            "onchain" => Self::Onchain,
            "stocks" => Self::Stocks,
            "automation" => Self::Automation,
            "execution" => Self::Execution,
            "review" => Self::Review,
            "settings" => Self::Settings,
            _ => return None,
        })
    }
}

pub(crate) fn initial_workspace_route() -> WorkspaceRoute {
    let fallback = stored_module().unwrap_or(ModuleId::Positions);
    current_workspace_route(fallback)
}

pub(crate) fn bind_workspace_route_listener(
    active_module: RwSignal<ModuleId>,
    apply: impl Fn(WorkspaceRoute) + Copy + 'static,
) {
    let hash_handle = window_event_listener_untyped("hashchange", move |_| {
        let route = current_workspace_route(active_module.get_untracked());
        apply(route);
    });
    let popstate_handle = window_event_listener_untyped("popstate", move |_| {
        let route = current_workspace_route(active_module.get_untracked());
        apply(route);
    });
    on_cleanup(move || {
        hash_handle.remove();
        popstate_handle.remove();
    });
}

pub(crate) fn sync_module_hash(module: ModuleId) {
    store_module(module);

    let Some(window) = web_sys::window() else {
        return;
    };
    let location = window.location();
    let hash = location.hash().unwrap_or_default();
    if hash_module(&hash) == Some(module) {
        return;
    }
    let _ = location.set_hash(module.slug());
}

fn current_workspace_route(fallback: ModuleId) -> WorkspaceRoute {
    let Some(window) = web_sys::window() else {
        return WorkspaceRoute::for_module(fallback);
    };
    let location = window.location();
    parse_workspace_route_parts(
        &location.hash().unwrap_or_default(),
        &location.search().unwrap_or_default(),
        fallback,
    )
}

fn parse_workspace_route_parts(hash: &str, search: &str, fallback: ModuleId) -> WorkspaceRoute {
    let fragment = hash.trim().trim_start_matches('#');
    let (fragment_module, fragment_query) = fragment
        .split_once('?')
        .map_or((fragment, ""), |(module, query)| (module, query));
    let fragment_params = RouteParams::parse(fragment_query);
    let search_params = RouteParams::parse(search.trim().trim_start_matches('?'));
    let value = |key| {
        fragment_params
            .get(key)
            .or_else(|| search_params.get(key))
            .and_then(clean_route_token)
    };
    let module = ModuleId::from_slug(fragment_module.trim().trim_start_matches('/'))
        .or_else(|| value("module").as_deref().and_then(ModuleId::from_slug))
        .unwrap_or(fallback);
    WorkspaceRoute {
        module,
        symbol: value("symbol"),
        strategy: value("strategy")
            .as_deref()
            .and_then(StrategyKind::from_query_value),
        origin: value("origin")
            .as_deref()
            .and_then(WorkspaceRouteOrigin::from_query_value),
        page: value("page"),
        opportunity_id: value("opp"),
        run_id: value("run"),
        ticket_id: value("ticket"),
    }
}

fn hash_module(hash: &str) -> Option<ModuleId> {
    let fragment = hash.trim().trim_start_matches('#');
    let module = fragment
        .split_once('?')
        .map_or(fragment, |(module, _)| module);
    ModuleId::from_slug(module.trim().trim_start_matches('/'))
}

fn clean_route_token(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.chars().count() <= MAX_ROUTE_TOKEN_CHARS
        && !value.chars().any(char::is_control))
    .then(|| value.to_owned())
}

#[derive(Default)]
struct RouteParams {
    module: Option<String>,
    symbol: Option<String>,
    strategy: Option<String>,
    origin: Option<String>,
    page: Option<String>,
    opportunity_id: Option<String>,
    run_id: Option<String>,
    ticket_id: Option<String>,
}

impl RouteParams {
    #[cfg(target_arch = "wasm32")]
    fn parse(query: &str) -> Self {
        let Ok(params) = web_sys::UrlSearchParams::new_with_str(query) else {
            return Self::default();
        };
        Self {
            module: params.get("module"),
            symbol: params.get("symbol"),
            strategy: params.get("strategy"),
            origin: params.get("origin"),
            page: params.get("page"),
            opportunity_id: params.get("opp"),
            run_id: params.get("run"),
            ticket_id: params.get("ticket"),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn parse(query: &str) -> Self {
        let mut parsed = Self::default();
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            parsed.set(key, value.to_owned());
        }
        parsed
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn set(&mut self, key: &str, value: String) {
        let target = match key {
            "module" => &mut self.module,
            "symbol" => &mut self.symbol,
            "strategy" => &mut self.strategy,
            "origin" => &mut self.origin,
            "page" => &mut self.page,
            "opp" => &mut self.opportunity_id,
            "run" => &mut self.run_id,
            "ticket" => &mut self.ticket_id,
            _ => return,
        };
        *target = Some(value);
    }

    fn get(&self, key: &str) -> Option<&str> {
        match key {
            "module" => self.module.as_deref(),
            "symbol" => self.symbol.as_deref(),
            "strategy" => self.strategy.as_deref(),
            "origin" => self.origin.as_deref(),
            "page" => self.page.as_deref(),
            "opp" => self.opportunity_id.as_deref(),
            "run" => self.run_id.as_deref(),
            "ticket" => self.ticket_id.as_deref(),
            _ => None,
        }
    }
}

fn stored_module() -> Option<ModuleId> {
    let raw: String = gloo_storage::LocalStorage::get(ACTIVE_MODULE_STORAGE_KEY).ok()?;
    hash_module(&raw)
}

fn store_module(module: ModuleId) {
    let _ = gloo_storage::LocalStorage::set(ACTIVE_MODULE_STORAGE_KEY, module.slug());
}

#[cfg(test)]
#[path = "routing_tests.rs"]
mod tests;
