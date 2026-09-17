//! `LlmProvider` trait：所有具体厂商适配器必须实现。

use crate::error::LlmResult;
use crate::evidence::ProviderApiEvidence;
use crate::types::{ChatRequest, ChatResponse};
use async_trait::async_trait;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &'static str;
    /// Official endpoint evidence for this provider implementation.
    fn evidence(&self) -> &'static ProviderApiEvidence;
    /// 该 provider 默认使用的模型 ID。
    fn default_model(&self) -> &str;
    /// 非流式 chat 调用。
    async fn chat(&self, req: ChatRequest) -> LlmResult<ChatResponse>;
}
