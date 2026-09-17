//! LLM 通用类型：消息、请求、响应、用量。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// 显式指定模型，否则用 provider 默认
    pub model: Option<String>,
    pub messages: Vec<Message>,
    pub temperature: f64,
    pub max_tokens: u32,
}

impl ChatRequest {
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            model: None,
            messages,
            temperature: 0.7,
            max_tokens: 1024,
        }
    }

    pub fn model(mut self, m: impl Into<String>) -> Self {
        self.model = Some(m.into());
        self
    }

    pub fn temperature(mut self, t: f64) -> Self {
        self.temperature = t.clamp(0.0, 2.0);
        self
    }

    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }

    /// 抽取系统消息（如有）；其余消息原样保留。
    pub fn split_system(&self) -> (Option<String>, Vec<Message>) {
        let mut system = None;
        let mut rest = Vec::with_capacity(self.messages.len());
        for m in &self.messages {
            if m.role == Role::System && system.is_none() {
                system = Some(m.content.clone());
            } else {
                rest.push(m.clone());
            }
        }
        (system, rest)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub provider: String,
    #[serde(default)]
    pub usage: Usage,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn role_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Role::System).unwrap(), "\"system\"");
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
    }

    #[test]
    fn chat_request_split_system() {
        let req = ChatRequest::new(vec![
            Message::system("You are helpful."),
            Message::user("Hello"),
            Message::assistant("Hi"),
        ]);
        let (sys, rest) = req.split_system();
        assert_eq!(sys, Some("You are helpful.".into()));
        assert_eq!(rest.len(), 2);
        assert_eq!(rest[0].role, Role::User);
        assert_eq!(rest[1].role, Role::Assistant);
    }

    #[test]
    fn chat_request_no_system_message() {
        let req = ChatRequest::new(vec![Message::user("Hi")]);
        let (sys, rest) = req.split_system();
        assert!(sys.is_none());
        assert_eq!(rest.len(), 1);
    }

    #[test]
    fn temperature_clamped() {
        let r = ChatRequest::new(vec![]).temperature(5.0);
        assert!((r.temperature - 2.0).abs() < 1e-12);
        let r2 = ChatRequest::new(vec![]).temperature(-1.0);
        assert!((r2.temperature - 0.0).abs() < 1e-12);
    }
}
