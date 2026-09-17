use crate::api::rest::ApiClient;
use crate::api::ws_runtime::provide_ws_runtime;
use crate::state::arbitrage_stream::{provide_arbitrage_stream, ArbitrageStream};
use crate::state::strategy_kinds::provide_strategy_kinds;
use crate::state::AppContext;
use leptos::prelude::*;

#[derive(Clone)]
pub struct GlobalContext {
    pub client: ApiClient,
    pub arbitrage_stream: ArbitrageStream,
}

pub fn provide_global() {
    let app_context = expect_context::<AppContext>();
    provide_ws_runtime(app_context.api_base, app_context.api_auth_token);
    let client =
        ApiClient::with_base_signal_and_auth(app_context.api_base, app_context.api_auth_token);
    let arbitrage_stream = provide_arbitrage_stream();
    provide_strategy_kinds(&client);
    provide_context(GlobalContext {
        client,
        arbitrage_stream,
    });
}

pub fn use_global() -> GlobalContext {
    expect_context::<GlobalContext>()
}
