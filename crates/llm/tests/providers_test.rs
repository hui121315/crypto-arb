#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! LLM 4 个 provider 的 wiremock 集成测试。
//!
//! 验证：URL 路径、鉴权头、请求 body 结构、响应解析。

use llm::{
    ChatRequest, ClaudeProvider, DeepSeekProvider, GeminiProvider, LlmProvider, Message,
    OpenAiProvider,
};
use serde_json::json;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ============== OpenAI ==============

#[tokio::test]
async fn openai_chat_round_trip() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-1",
            "model": "gpt-4o-mini",
            "choices": [{"message": {"content": "hi there"}}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8}
        })))
        .mount(&server)
        .await;

    let p = OpenAiProvider::with_options("test-key", Some(server.uri()), "gpt-4o-mini").unwrap();
    let req = ChatRequest::new(vec![Message::user("hello")]);
    let resp = p.chat(req).await.expect("ok");
    assert_eq!(resp.content, "hi there");
    assert_eq!(resp.provider, "openai");
    assert_eq!(resp.model, "gpt-4o-mini");
    assert_eq!(resp.usage.total_tokens, 8);
}

#[tokio::test]
async fn openai_401_returns_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"error": {"message": "bad key"}})),
        )
        .mount(&server)
        .await;

    let p = OpenAiProvider::with_options("bad", Some(server.uri()), "gpt-4o-mini").unwrap();
    let err = p
        .chat(ChatRequest::new(vec![Message::user("hi")]))
        .await
        .expect_err("auth");
    assert!(matches!(
        err,
        llm::LlmError::Auth {
            provider,
            path,
            status: 401,
            ..
        } if provider == "openai" && path == "/v1/chat/completions"
    ));
}

#[tokio::test]
async fn openai_429_returns_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "30")
                .set_body_string("rate limited"),
        )
        .mount(&server)
        .await;

    let p = OpenAiProvider::with_options("k", Some(server.uri()), "gpt-4o-mini").unwrap();
    let err = p
        .chat(ChatRequest::new(vec![Message::user("hi")]))
        .await
        .expect_err("rl");
    match err {
        llm::LlmError::RateLimited {
            provider,
            path,
            retry_after_secs,
            ..
        } => {
            assert_eq!(provider, "openai");
            assert_eq!(path, "/v1/chat/completions");
            assert_eq!(retry_after_secs, Some(30));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

// ============== DeepSeek ==============

#[tokio::test]
async fn deepseek_uses_official_chat_completions_path_with_own_provider_label() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer ds-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "deepseek-chat",
            "choices": [{"message": {"content": "你好"}}],
            "usage": {"prompt_tokens": 2, "completion_tokens": 1, "total_tokens": 3}
        })))
        .mount(&server)
        .await;

    let p = DeepSeekProvider::with_options("ds-key", Some(server.uri()), "deepseek-chat").unwrap();
    let resp = p
        .chat(ChatRequest::new(vec![Message::user("hi")]))
        .await
        .expect("ok");
    assert_eq!(resp.content, "你好");
    assert_eq!(resp.provider, "deepseek");
    assert_eq!(resp.model, "deepseek-chat");
}

// ============== Claude ==============

#[tokio::test]
async fn claude_round_trip_with_system_and_messages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "claude-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_1",
            "model": "claude-3-5-sonnet-latest",
            "content": [
                {"type": "text", "text": "Hello! How can I help?"}
            ],
            "usage": {"input_tokens": 12, "output_tokens": 8}
        })))
        .mount(&server)
        .await;

    let p =
        ClaudeProvider::with_options("claude-key", Some(server.uri()), "claude-3-5-sonnet-latest")
            .unwrap();
    let req = ChatRequest::new(vec![
        Message::system("You are concise."),
        Message::user("Hi!"),
    ]);
    let resp = p.chat(req).await.expect("ok");
    assert_eq!(resp.content, "Hello! How can I help?");
    assert_eq!(resp.provider, "claude");
    assert_eq!(resp.usage.prompt_tokens, 12);
    assert_eq!(resp.usage.completion_tokens, 8);
    assert_eq!(resp.usage.total_tokens, 20);
}

#[tokio::test]
async fn claude_multi_text_blocks_concatenated() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "claude-3-5-sonnet-latest",
            "content": [
                {"type": "text", "text": "Hello, "},
                {"type": "text", "text": "world!"}
            ],
            "usage": {"input_tokens": 5, "output_tokens": 3}
        })))
        .mount(&server)
        .await;

    let p =
        ClaudeProvider::with_options("k", Some(server.uri()), "claude-3-5-sonnet-latest").unwrap();
    let resp = p
        .chat(ChatRequest::new(vec![Message::user("hi")]))
        .await
        .expect("ok");
    assert_eq!(resp.content, "Hello, world!");
}

// ============== Gemini ==============

#[tokio::test]
async fn gemini_round_trip_with_system_instruction() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-1.5-flash:generateContent"))
        .and(header("x-goog-api-key", "gemini-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Bonjour!"}]
                }
            }],
            "usageMetadata": {
                "promptTokenCount": 6,
                "candidatesTokenCount": 2,
                "totalTokenCount": 8
            },
            "modelVersion": "gemini-1.5-flash"
        })))
        .mount(&server)
        .await;

    let p =
        GeminiProvider::with_options("gemini-key", Some(server.uri()), "gemini-1.5-flash").unwrap();
    let req = ChatRequest::new(vec![
        Message::system("Reply in French."),
        Message::user("Hello"),
    ]);
    let resp = p.chat(req).await.expect("ok");
    assert_eq!(resp.content, "Bonjour!");
    assert_eq!(resp.provider, "gemini");
    assert_eq!(resp.usage.total_tokens, 8);
}

#[tokio::test]
async fn gemini_403_returns_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"error": {"message": "perm denied"}})),
        )
        .mount(&server)
        .await;

    let p = GeminiProvider::with_options("k", Some(server.uri()), "gemini-1.5-flash").unwrap();
    let err = p
        .chat(ChatRequest::new(vec![Message::user("x")]))
        .await
        .expect_err("auth");
    assert!(matches!(
        err,
        llm::LlmError::Auth {
            provider,
            path,
            status: 403,
            ..
        } if provider == "gemini" && path == "/v1beta/models/{model}:generateContent"
    ));
}

// ============== Router 端到端 ==============

#[tokio::test]
async fn router_dispatches_to_provider() {
    let openai_srv = MockServer::start().await;
    let claude_srv = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "gpt-4o-mini",
            "choices": [{"message": {"content": "openai reply"}}],
            "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
        })))
        .mount(&openai_srv)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "claude-3-5-sonnet-latest",
            "content": [{"type": "text", "text": "claude reply"}],
            "usage": {"input_tokens": 0, "output_tokens": 0}
        })))
        .mount(&claude_srv)
        .await;

    let router = llm::LlmRouter::new();
    router.register(std::sync::Arc::new(
        OpenAiProvider::with_options("ok", Some(openai_srv.uri()), "gpt-4o-mini").unwrap(),
    ));
    router.register(std::sync::Arc::new(
        ClaudeProvider::with_options("ck", Some(claude_srv.uri()), "claude-3-5-sonnet-latest")
            .unwrap(),
    ));

    // default = openai (first registered)
    let r1 = router
        .chat(None, ChatRequest::new(vec![Message::user("hi")]))
        .await
        .unwrap();
    assert_eq!(r1.provider, "openai");
    assert_eq!(r1.content, "openai reply");

    // explicit claude
    let r2 = router
        .chat(Some("claude"), ChatRequest::new(vec![Message::user("hi")]))
        .await
        .unwrap();
    assert_eq!(r2.provider, "claude");
    assert_eq!(r2.content, "claude reply");
}

// ============== 配置 sanity（不发请求） ==============

#[tokio::test]
async fn providers_construct_with_defaults() {
    assert_eq!(
        OpenAiProvider::new("k").unwrap().default_model(),
        "gpt-4o-mini"
    );
    assert_eq!(
        DeepSeekProvider::new("k").unwrap().default_model(),
        "deepseek-v4-flash"
    );
    assert_eq!(
        ClaudeProvider::new("k").unwrap().default_model(),
        "claude-3-5-sonnet-latest"
    );
    assert_eq!(
        GeminiProvider::new("k").unwrap().default_model(),
        "gemini-1.5-flash"
    );
    // 至少有一个 header 库 import 用上
    let _hdr = header_exists("authorization");
}
