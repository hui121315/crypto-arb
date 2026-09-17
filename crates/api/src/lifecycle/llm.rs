use super::env;
use crate::state::AppState;
use llm::{ClaudeProvider, DeepSeekProvider, GeminiProvider, OpenAiProvider};
use std::sync::Arc;
use tracing::{info, warn};

/// LLM provider 注册（按 env API key 条件启用）。
pub(super) fn register(state: &AppState) {
    let router = state.llm_router();
    let mut count = 0_usize;

    count += register_provider(
        "openai",
        env::var("OPENAI_API_KEY"),
        |key| OpenAiProvider::new(key).map(|p| Arc::new(p) as _),
        router,
    );
    count += register_provider(
        "claude",
        env::var("ANTHROPIC_API_KEY"),
        |key| ClaudeProvider::new(key).map(|p| Arc::new(p) as _),
        router,
    );
    count += register_provider(
        "gemini",
        env::var("GOOGLE_API_KEY"),
        |key| GeminiProvider::new(key).map(|p| Arc::new(p) as _),
        router,
    );
    count += register_provider(
        "deepseek",
        env::var("DEEPSEEK_API_KEY"),
        |key| DeepSeekProvider::new(key).map(|p| Arc::new(p) as _),
        router,
    );

    info!(count, "LLM provider registration complete");
}

fn register_provider<F>(
    name: &'static str,
    key: Option<String>,
    build: F,
    router: &llm::LlmRouter,
) -> usize
where
    F: FnOnce(String) -> llm::LlmResult<Arc<dyn llm::LlmProvider>>,
{
    let Some(provider) = provider_from_env(name, key, build) else {
        return 0;
    };
    router.register(provider);
    info!("LLM registered: {name}");
    1
}

fn provider_from_env<F>(
    name: &'static str,
    key: Option<String>,
    build: F,
) -> Option<Arc<dyn llm::LlmProvider>>
where
    F: FnOnce(String) -> llm::LlmResult<Arc<dyn llm::LlmProvider>>,
{
    let key = key?;
    build_provider(name, key, build)
}

fn build_provider<F>(name: &'static str, key: String, build: F) -> Option<Arc<dyn llm::LlmProvider>>
where
    F: FnOnce(String) -> llm::LlmResult<Arc<dyn llm::LlmProvider>>,
{
    match build(key) {
        Ok(provider) => Some(provider),
        Err(e) => {
            warn!(error = %e, "{name} provider build failed");
            None
        }
    }
}
