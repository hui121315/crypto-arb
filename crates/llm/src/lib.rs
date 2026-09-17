#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]
//! LLM 多厂商路由：OpenAI / Claude / Gemini / `DeepSeek` 统一接口。

pub mod error;
pub mod evidence;
pub mod provider;
pub mod providers;
pub mod router;
pub mod types;

pub use error::{LlmError, LlmResult};
pub use evidence::{
    provider_evidence, ProviderApiEvidence, CLAUDE_EVIDENCE, DEEPSEEK_EVIDENCE, GEMINI_EVIDENCE,
    OPENAI_EVIDENCE, PROVIDER_API_EVIDENCE,
};
pub use provider::LlmProvider;
pub use providers::{ClaudeProvider, DeepSeekProvider, GeminiProvider, OpenAiProvider};
pub use router::{LlmRouter, ProviderSelection};
pub use types::{ChatRequest, ChatResponse, Message, Role, Usage};
