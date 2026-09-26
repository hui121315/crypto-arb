use super::*;

pub(super) fn refresh_inventory(data: StockData) -> impl IntoView {
    let selected = Memo::new(move |_| {
        data.market.with(|m| {
            m.value()
                .and_then(|s| s.security.as_ref().map(|s| s.asset.clone()))
        })
    });
    view! {<div class="stock-stablecoin-actions" aria-label="到账后库存复查">
        <button type="button" class="row-action" title="读取当前股票与钱包的新余额和充提状态；不会下单、兑换或转账"
            disabled=move ||data.pending.get() ||data.preflight.pending.get()
                ||selected.get().is_none() ||data.preflight.wallet.get().trim().is_empty()
            on:click=move |_|{
                if data.pending.get_untracked() ||data.preflight.pending.get_untracked()
                    ||data.preflight.wallet.get_untracked().trim().is_empty(){return;}
                if let Some(asset)=selected.get_untracked(){
                    data.preflight.read.run(asset);
                    #[cfg(target_arch="wasm32")]
                    if let Some(el)=web_sys::window().and_then(|w|w.document()).and_then(|d|d.get_element_by_id("stock-inventory")){el.scroll_into_view_with_bool(true);}
                }
            }>"重新检查库存与补库"</button>
        <p class="stock-rfq-note">"按当前股票与钱包重新读取；本笔到账不代替账户余额，兑换和转账费用仍保留在原记录中。"</p>
    </div>}
}

pub(super) fn needs(
    row: StockFundingDirection,
    data: StockData,
    asset: String,
    checked_at_ms: i64,
    source_plan: Option<StockInventorySource>,
) -> impl IntoView {
    let current_asset = asset.clone();
    let current_inventory = move || {
        data.market.with(|m| {
            m.value().is_some_and(|s| {
                inventory_current(
                    s,
                    &current_asset,
                    checked_at_ms,
                    data.preflight.wallet.get().trim(),
                    data.clock.get(),
                )
            })
        })
    };
    view! {<div class="stock-funding-needs" aria-label="股票补库路径">
        {row.needs.into_iter().map(|n| {
            let metadata=n.token.clone();
            let chain_conversion_target=n.usdc_conversion_target("Solana");
            let account_conversion_target=n.usdc_conversion_target("Backpack");
            let account_conversion_label=if n.target=="Backpack"{"用账户 USDT 补入"}else{"先补足 Backpack USDC"};
            let target=match n.target.as_str(){"Backpack"=>Some(StockFundingTarget::Backpack),"Solana"=>Some(StockFundingTarget::Solana),_=>None};
            let request=target.map(|target|StockFundingPlanRequest {source_plan:source_plan.clone(),request_id:String::new(),security_asset:asset.clone(),funding_asset:n.asset.clone(),direction:row.direction,target,wallet_address:String::new(),preflight_at_ms:checked_at_ms});
            let action=request.map(|request| {
                let enabled_request=request.clone();
                let enabled_need=n.clone();
                view!{<button type="button" class="row-action stock-funding-save" title="查询账户可提上限或官方充值地址，保存本地计划；不发起转账"
                    disabled=move ||data.preflight.pending.get() ||data.pending.get() ||data.market.with(|m|m.value().is_none_or(|s|!can_save(s,&enabled_request,&enabled_need,data.preflight.wallet.get().trim(),data.clock.get())))
                    on:click=move |_|{let mut r=request.clone();r.wallet_address=data.preflight.wallet.get();data.preflight.funding_build.run(r);}>
                    "保存补库计划"
                </button>}
            });
            let route=metadata.as_ref().map(|t|format!("{}{}",if n.target=="Backpack"{"充值"}else{"提现"},flag(if n.target=="Backpack"{t.deposit_enabled}else{t.withdraw_enabled}))).unwrap_or_else(||"通道待核实".into());
            view! {<div class="stock-funding-need">
                <strong>{format!("{} · {} 缺 {}",n.target,n.asset,n.shortfall.as_deref().unwrap_or("待核实"))}</strong>
                <p class="stock-rfq-note">{format!("{} → {} · {route} · {}",n.source,n.target,match n.source_sufficient {
                    Some(true)=>"来源可调数量足够 · 尚未转账",Some(false)=>"来源不足 · 需外部补入",None=>"来源或所需数量待核实",
                })}</p>
                <dl class="stock-direction-values">
                    <div><dt>"来源可用"</dt><dd>{n.source_available.unwrap_or_else(||"未知".into())}</dd></div>
                    <div><dt>"来源套利备款"</dt><dd>{n.source_trade_reserve.unwrap_or_else(||"待核实".into())}</dd></div>
                    <div><dt>"扣除本次备款后可调"</dt><dd>{n.source_spare.unwrap_or_else(||"未知".into())}</dd></div>
                    <div><dt>"补库保守备款 / 原币"</dt><dd>{n.conservative_source_budget.unwrap_or_else(||"待核实".into())}</dd></div>
                </dl>
                <details><summary>"充提限制与待办"</summary>
                    {metadata.map(|t|view!{<dl class="stock-plan-evidence">
                        <div><dt>"Solana 充值 / 提现"</dt><dd>{format!("{} / {}",flag(t.deposit_enabled),flag(t.withdraw_enabled))}</dd></div>
                        <div><dt>"最低充值 / 提现"</dt><dd>{format!("{} / {}",t.minimum_deposit.unwrap_or_else(||"未知".into()),t.minimum_withdrawal.unwrap_or_else(||"未知".into()))}</dd></div>
                        <div><dt>"提现费 / 原币"</dt><dd>{t.withdrawal_fee.unwrap_or_else(||"未知".into())}</dd></div>
                        <div><dt>"单笔提现上限"</dt><dd>{t.maximum_withdrawal.unwrap_or_else(||"官方未列出上限".into())}</dd></div>
                    </dl>})}
                    <ul>{n.blockers.into_iter().map(|b|view!{<li>{b}</li>}).collect_view()}</ul>
                </details>
                <div class="stock-stablecoin-actions">
                {action}
                {chain_conversion_target.map(|shortfall|view!{
                    <button type="button" class="row-action" title="将本次缺口填入 USDT 兑换试算；不会自动增加投入或兑换"
                        disabled={let current=current_inventory.clone();move ||data.preflight.pending.get() ||data.pending.get() ||!current()}
                        on:click={let current=current_inventory.clone();move |_|{if current(){data.preflight.stablecoin.target.set(shortfall.clone());}}}>"用此缺口试算 USDT 补入"</button>
                })}
                {account_conversion_target.map(|shortfall|view!{
                    <button type="button" class="row-action" title="按此缺口核算账户 USDT 投入；只读，不预留或转账"
                        disabled={let current=current_inventory.clone();move ||data.preflight.pending.get() ||data.pending.get() ||!current()}
                        on:click={let current=current_inventory.clone();move |_|{
                            if !current(){return;}
                            data.preflight.conversion.minimum.set(shortfall.clone());
                            data.preflight.conversion.input.set(String::new());
                            data.preflight.conversion.size.run(());
                            #[cfg(target_arch="wasm32")]
                            if let Some(el)=web_sys::window().and_then(|w|w.document()).and_then(|d|d.get_element_by_id("stock-account-conversion")) {el.scroll_into_view_with_bool(true);}
                        }}>{account_conversion_label}</button>
                })}
                </div>
            </div>}
        }).collect_view()}
    </div>}
}

fn inventory_current(
    s: &StockMarketSnapshot,
    asset: &str,
    checked_at_ms: i64,
    wallet: &str,
    now: i64,
) -> bool {
    !wallet.is_empty()
        && checked_at_ms > 0
        && now >= checked_at_ms
        && now.saturating_sub(checked_at_ms) <= 30_000
        && s.security.as_ref().is_some_and(|a| a.asset == asset)
        && s.preflight.as_ref().is_some_and(|p| {
            p.asset == asset
                && p.checked_at_ms == checked_at_ms
                && p.wallet_address.as_deref() == Some(wallet)
                && (p.source_plan.is_none() || now < p.valid_until_ms)
        })
}

fn can_save(
    s: &StockMarketSnapshot,
    r: &StockFundingPlanRequest,
    n: &StockFundingNeed,
    wallet: &str,
    now: i64,
) -> bool {
    let fresh = |at: i64| at > 0 && now >= at && now.saturating_sub(at) <= 30_000;
    inventory_current(s, &r.security_asset, r.preflight_at_ms, wallet, now)
        && s.preflight.as_ref().is_some_and(|p|p.source_plan == r.source_plan
            && (p.source_plan.is_none() || now < p.valid_until_ms))
        && n.source_sufficient == Some(true)
        && n.shortfall
            .as_deref()
            .and_then(shared_types::stocks::comparison::positive)
            .is_some()
        && n.metadata_at_ms.is_some_and(fresh)
        && n.token.as_ref().is_some_and(|t| {
            (match r.target {
                StockFundingTarget::Backpack => t.deposit_enabled,
                StockFundingTarget::Solana => t.withdraw_enabled,
            }) == Some(true)
        })
        && s.funding_problem.is_none()
        && s.plan_problem.is_none()
        && s.stablecoin_problem.is_none()
        && s.exchange_conversion_problem.is_none()
        && !s.exchange_conversions.iter().any(|p| p.holds_funds(now))
        && !s
            .stablecoin_plans
            .iter()
            .any(|p| p.request.conversion.wallet_address == wallet && p.holds_funds(now))
        && !s.plans.iter().any(|p| p.holds_funds(now))
        && !s
            .funding_plans
            .iter()
            .any(|p| p.phase_at(now).holds_funds())
}

#[cfg(test)]
pub(super) mod tests;

pub(super) fn address(asset: String, data: StockData) -> impl IntoView {
    let request_asset = asset.clone();
    view! {<div class="stock-funding-address">
        <button type="button" class="row-action" disabled=move ||data.preflight.pending.get() ||data.pending.get()
            on:click=move |_|data.preflight.deposit_address.run(request_asset.clone())>"读取 Backpack 充值地址"</button>
        {move ||data.market.with(|m|m.value().and_then(|s|s.deposit_address.clone())).filter(|a|a.asset==asset).map(|a| {
            let fresh=data.clock.get()>=a.checked_at_ms && data.clock.get()-a.checked_at_ms<=30_000;
            view!{<dl class="stock-plan-evidence"><div><dt>{if fresh {"官方账户地址 · Solana"} else {"历史地址 · 使用前重新读取"}}</dt><dd>{a.address}</dd></div></dl>}
        })}
        <p class="stock-rfq-note">"仅查询当前 Backpack 账户地址，未发起充值或提现。到账后需重新构建套利计划。"</p>
    </div>}
}

fn flag(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "开放",
        Some(false) => "关闭",
        None => "未知",
    }
}
