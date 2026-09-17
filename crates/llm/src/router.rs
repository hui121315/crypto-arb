//! LLM 路由：根据 provider 名称分发请求。

use crate::error::{LlmError, LlmResult};
use crate::evidence::ProviderApiEvidence;
use crate::provider::LlmProvider;
use crate::types::{ChatRequest, ChatResponse};
use dashmap::DashMap;
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSelection {
    pub name: String,
    pub default_model: String,
    pub evidence: ProviderApiEvidence,
}

#[derive(Default)]
pub struct LlmRouter {
    providers: DashMap<String, Arc<dyn LlmProvider>>,
    default_provider: parking_lot::RwLock<Option<String>>,
}

impl fmt::Debug for LlmRouter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LlmRouter")
            .field("providers", &self.names())
            .field("default_provider", &self.default_name())
            .finish()
    }
}

impl LlmRouter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一个 provider。若同名已存在则被替换。
    pub fn register(&self, provider: Arc<dyn LlmProvider>) {
        let name = provider.name().to_owned();
        self.providers.insert(name.clone(), provider);
        // 若尚未设置默认，则把第一个注册的设为默认
        let mut guard = self.default_provider.write();
        if guard.is_none() {
            *guard = Some(name);
        }
    }

    /// 显式设置默认 provider。
    pub fn set_default(&self, name: &str) -> LlmResult<()> {
        if !self.providers.contains_key(name) {
            return Err(LlmError::ProviderNotFound(name.into()));
        }
        *self.default_provider.write() = Some(name.into());
        Ok(())
    }

    pub fn default_name(&self) -> Option<String> {
        self.default_provider.read().clone()
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.providers.iter().map(|e| e.key().clone()).collect();
        v.sort();
        v
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Resolves a provider without sending data so callers can durably audit the
    /// intended external destination before the request leaves the process.
    pub fn selection(&self, provider: Option<&str>) -> LlmResult<ProviderSelection> {
        let (name, provider) = self.resolve(provider)?;
        Ok(ProviderSelection {
            name,
            default_model: provider.default_model().to_owned(),
            evidence: *provider.evidence(),
        })
    }

    pub fn registered_evidence(&self) -> Vec<ProviderApiEvidence> {
        let mut evidence = self
            .providers
            .iter()
            .map(|entry| *entry.value().evidence())
            .collect::<Vec<_>>();
        evidence.sort_by_key(|item| item.provider);
        evidence
    }

    /// 分发 chat 调用。`provider=None` 时使用默认 provider。
    pub async fn chat(&self, provider: Option<&str>, req: ChatRequest) -> LlmResult<ChatResponse> {
        let (_, provider) = self.resolve(provider)?;
        provider.chat(req).await
    }

    fn resolve(&self, provider: Option<&str>) -> LlmResult<(String, Arc<dyn LlmProvider>)> {
        let name = match provider {
            Some(p) => p.to_owned(),
            None => self
                .default_name()
                .ok_or_else(|| LlmError::ProviderNotFound("(no default)".into()))?,
        };
        let p = self
            .providers
            .get(&name)
            .ok_or_else(|| LlmError::ProviderNotFound(name.clone()))?;
        let provider = Arc::clone(p.value());
        drop(p);
        Ok((name, provider))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockProvider {
        name_: &'static str,
        calls: AtomicUsize,
    }

    impl MockProvider {
        fn new(name: &'static str) -> Self {
            Self {
                name_: name,
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &'static str {
            self.name_
        }
        fn evidence(&self) -> &'static ProviderApiEvidence {
            &crate::evidence::OPENAI_EVIDENCE
        }
        fn default_model(&self) -> &str {
            "mock-1"
        }
        async fn chat(&self, _req: ChatRequest) -> LlmResult<ChatResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ChatResponse {
                content: format!("hello from {}", self.name_),
                model: "mock-1".into(),
                provider: self.name_.into(),
                usage: Default::default(),
            })
        }
    }

    #[tokio::test]
    async fn empty_router_chat_fails() {
        let r = LlmRouter::new();
        let resp = r.chat(None, ChatRequest::new(vec![])).await;
        assert!(matches!(resp, Err(LlmError::ProviderNotFound(_))));
    }

    #[tokio::test]
    async fn first_registered_becomes_default() {
        let r = LlmRouter::new();
        r.register(Arc::new(MockProvider::new("a")));
        r.register(Arc::new(MockProvider::new("b")));
        assert_eq!(r.default_name().as_deref(), Some("a"));
        assert_eq!(r.names(), vec!["a", "b"]);
        assert_eq!(r.len(), 2);
    }

    #[tokio::test]
    async fn set_default_changes_route() {
        let r = LlmRouter::new();
        r.register(Arc::new(MockProvider::new("a")));
        r.register(Arc::new(MockProvider::new("b")));
        r.set_default("b").unwrap();
        let resp = r.chat(None, ChatRequest::new(vec![])).await.unwrap();
        assert_eq!(resp.provider, "b");
    }

    #[tokio::test]
    async fn explicit_provider_overrides_default() {
        let r = LlmRouter::new();
        r.register(Arc::new(MockProvider::new("a")));
        r.register(Arc::new(MockProvider::new("b")));
        let resp = r.chat(Some("b"), ChatRequest::new(vec![])).await.unwrap();
        assert_eq!(resp.provider, "b");
        assert_eq!(resp.content, "hello from b");
    }

    #[tokio::test]
    async fn unknown_provider_errors() {
        let r = LlmRouter::new();
        r.register(Arc::new(MockProvider::new("a")));
        let resp = r.chat(Some("nope"), ChatRequest::new(vec![])).await;
        assert!(matches!(resp, Err(LlmError::ProviderNotFound(_))));
    }

    #[tokio::test]
    async fn set_default_with_missing_provider_errors() {
        let r = LlmRouter::new();
        let err = r.set_default("missing").unwrap_err();
        assert!(matches!(err, LlmError::ProviderNotFound(_)));
    }

    #[tokio::test]
    async fn selection_exposes_provider_evidence_without_dispatching() {
        let r = LlmRouter::new();
        r.register(Arc::new(MockProvider::new("a")));

        let selection = r.selection(None).expect("selection");

        assert_eq!(selection.name, "a");
        assert_eq!(selection.default_model, "mock-1");
        assert_eq!(selection.evidence.provider, "openai");
    }
}
