use super::*;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OpportunitiesParams {
    #[serde(default)]
    pub(super) limit: Option<usize>,
    #[serde(default)]
    pub(super) min_yield: Option<f64>,
    #[serde(default)]
    pub(super) strategy: Option<String>,
    #[serde(default)]
    pub(super) symbol: Option<String>,
    /// 请求后台刷新；响应仍只读当前快照，避免 HTTP 热路径等待扫描。
    #[serde(default)]
    pub(super) fresh: bool,
    /// 快速模式只读热快照/轻量兜底，不在请求路径同步打交易所聚合接口。
    #[serde(default)]
    pub(super) fast: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OpportunityListParams {
    #[serde(default)]
    pub(super) page_size: Option<usize>,
    #[serde(default)]
    pub(super) limit: Option<usize>,
    #[serde(default)]
    pub(super) cursor: Option<String>,
    #[serde(default)]
    pub(super) sort_key: Option<String>,
    #[serde(default)]
    pub(super) min_yield: Option<f64>,
    #[serde(default)]
    pub(super) strategy: Option<String>,
    #[serde(default)]
    pub(super) symbol: Option<String>,
    #[serde(default)]
    pub(super) fresh: bool,
    #[serde(default)]
    pub(super) fast: bool,
}

#[derive(Debug)]
pub(super) struct OpportunityFilters {
    pub(super) limit: Option<usize>,
    pub(super) min_yield: Option<f64>,
    pub(super) strategy_kinds: Vec<StrategyKind>,
    pub(super) symbol: Option<String>,
}

impl OpportunityFilters {
    #[cfg(test)]
    pub(super) fn parse(params: &OpportunitiesParams) -> Self {
        let limit = opportunity::wide_limit(params.limit);
        Self::parse_with_limit(params, limit.value())
    }

    pub(super) fn parse_with_limit(params: &OpportunitiesParams, limit: usize) -> Self {
        Self {
            limit: Some(limit),
            min_yield: params.min_yield,
            strategy_kinds: Self::strategy_kinds(params.strategy.as_deref()),
            symbol: Self::symbol(params.symbol.as_deref()),
        }
    }

    pub(super) fn parse_list(params: &OpportunityListParams) -> Self {
        Self {
            limit: params.page_size.or(params.limit),
            min_yield: params.min_yield,
            strategy_kinds: Self::strategy_kinds(params.strategy.as_deref()),
            symbol: Self::symbol(params.symbol.as_deref()),
        }
    }

    pub(super) fn apply(&self, list: &mut Vec<ArbitrageOpportunityDto>) {
        list.retain(|row| self.matches(row));
    }

    pub(super) fn matches(&self, row: &ArbitrageOpportunityDto) -> bool {
        let min_yield_ok = match self.min_yield {
            Some(min) => row.net_single_yield >= min,
            None => true,
        };
        min_yield_ok && self.matches_symbol(row) && self.matches_strategy(row)
    }

    pub(super) fn matches_product_row(&self, row: &ArbitrageOpportunityDto, now_ms: i64) -> bool {
        self.matches(row) && opportunity::is_product_visible_row(row, now_ms)
    }

    pub(super) fn matches_strategy(&self, row: &ArbitrageOpportunityDto) -> bool {
        row.strategy_kind
            .is_some_and(|kind| self.strategy_kinds.contains(&kind))
    }

    pub(super) fn matches_symbol(&self, row: &ArbitrageOpportunityDto) -> bool {
        match self.symbol.as_deref() {
            Some(symbol) => opportunity_symbol_matches(row, symbol),
            None => true,
        }
    }

    pub(super) fn strategy_kinds(value: Option<&str>) -> Vec<StrategyKind> {
        let Some(value) = value.map(str::trim) else {
            return default_p0_strategy_kinds();
        };
        if value.is_empty() {
            return default_p0_strategy_kinds();
        }
        value.split(',').filter_map(strategy_kind).collect()
    }

    pub(super) fn symbol(value: Option<&str>) -> Option<String> {
        let raw = value?.trim();
        if raw.is_empty() {
            return None;
        }
        let base = raw.split_once(':').map_or(raw, |(_, symbol)| symbol);
        let symbol = exchange::strip_common_suffixes(base);
        (!symbol.is_empty()).then_some(symbol)
    }

    pub(super) fn scope(&self) -> OpportunityEnvelopeScope {
        if !self.strategy_kinds.is_empty()
            && self
                .strategy_kinds
                .iter()
                .all(|kind| is_p0_executable_strategy(*kind))
        {
            OpportunityEnvelopeScope::MainP0
        } else {
            OpportunityEnvelopeScope::Custom
        }
    }

    pub(super) fn query_key(&self, params: &OpportunitiesParams) -> String {
        let strategies = self
            .strategy_kinds
            .iter()
            .map(|kind| kind.as_query_value())
            .collect::<Vec<_>>()
            .join(",");
        let symbol = self.symbol.as_deref().unwrap_or("*");
        let min_yield = self
            .min_yield
            .map(|value| value.to_string())
            .unwrap_or_else(|| "*".into());
        let limit = self
            .limit
            .map(|value| value.to_string())
            .unwrap_or_else(|| "*".into());
        format!(
            "scope={};strategy={strategies};symbol={symbol};minYield={min_yield};limit={limit};fast={};fresh={}",
            scope_key(self.scope()),
            params.fast,
            params.fresh
        )
    }

    pub(super) fn list_query_key(
        &self,
        params: &OpportunityListParams,
        window: opportunity::OpportunityListWindow,
    ) -> String {
        let filter_key = self.list_filter_key();
        format!(
            "{filter_key};pageSize={};cursor={};sortKey={:?};fast={};fresh={}",
            window.page_size(),
            window.offset(),
            window.sort_key(),
            params.fast,
            params.fresh
        )
    }

    pub(super) fn list_request_meta(
        &self,
        params: &OpportunityListParams,
        window: opportunity::OpportunityListWindow,
    ) -> shared_types::arbitrage::OpportunityListRequestMeta {
        shared_types::arbitrage::OpportunityListRequestMeta {
            fast: params.fast,
            fresh: params.fresh,
            filter: shared_types::arbitrage::OpportunityListFilterMeta {
                scope: self.scope(),
                strategy_kinds: self.strategy_kinds.clone(),
                symbol: self.symbol.clone(),
                min_yield: self.min_yield,
            },
            sort_key: window.sort_key(),
            requested_page_size: window.requested_page_size(),
            applied_page_size: window.page_size(),
            max_page_size: window.max_page_size(),
        }
    }

    pub(super) fn list_filter_key(&self) -> String {
        let strategies = self
            .strategy_kinds
            .iter()
            .map(|kind| kind.as_query_value())
            .collect::<Vec<_>>()
            .join(",");
        let symbol = self.symbol.as_deref().unwrap_or("*");
        let min_yield = self
            .min_yield
            .map(|value| value.to_string())
            .unwrap_or_else(|| "*".into());
        format!(
            "scope={};strategy={strategies};symbol={symbol};minYield={min_yield}",
            scope_key(self.scope()),
        )
    }
}

pub(super) fn opportunity_symbol_matches(opp: &ArbitrageOpportunityDto, symbol: &str) -> bool {
    exchange::strip_common_suffixes(&opp.symbol) == symbol
}

fn strategy_kind(value: &str) -> Option<StrategyKind> {
    opportunity::p0_strategy_kind(value)
}

pub(super) fn default_p0_strategy_kinds() -> Vec<StrategyKind> {
    P0_EXECUTABLE_STRATEGY_KINDS.to_vec()
}

fn scope_key(scope: OpportunityEnvelopeScope) -> &'static str {
    match scope {
        OpportunityEnvelopeScope::MainP0 => "main_p0",
        OpportunityEnvelopeScope::RegistrySnapshot => "registry_snapshot",
        OpportunityEnvelopeScope::Custom => "custom",
    }
}
