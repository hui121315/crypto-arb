//! Official provider endpoint evidence used by the optional LLM surface.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderApiEvidence {
    pub provider: &'static str,
    pub doc_url: &'static str,
    pub checked_at: &'static str,
    pub base_url: &'static str,
    pub endpoint_path: &'static str,
    pub auth_headers: &'static [&'static str],
    pub default_model: &'static str,
}

impl ProviderApiEvidence {
    pub fn endpoint_url(self, base_url: &str, model: &str) -> String {
        let endpoint = self.endpoint_path.replace("{model}", model);
        format!("{}{}", base_url.trim_end_matches('/'), endpoint)
    }
}

pub const OPENAI_EVIDENCE: ProviderApiEvidence = ProviderApiEvidence {
    provider: "openai",
    doc_url: "https://platform.openai.com/docs/api-reference/chat/create",
    checked_at: "2026-07-12",
    base_url: "https://api.openai.com",
    endpoint_path: "/v1/chat/completions",
    auth_headers: &["Authorization: Bearer"],
    default_model: "gpt-4o-mini",
};

pub const CLAUDE_EVIDENCE: ProviderApiEvidence = ProviderApiEvidence {
    provider: "claude",
    doc_url: "https://docs.anthropic.com/en/api/messages",
    checked_at: "2026-07-12",
    base_url: "https://api.anthropic.com",
    endpoint_path: "/v1/messages",
    auth_headers: &["x-api-key", "anthropic-version"],
    default_model: "claude-3-5-sonnet-latest",
};

pub const GEMINI_EVIDENCE: ProviderApiEvidence = ProviderApiEvidence {
    provider: "gemini",
    doc_url: "https://ai.google.dev/api/generate-content",
    checked_at: "2026-07-12",
    base_url: "https://generativelanguage.googleapis.com",
    endpoint_path: "/v1beta/models/{model}:generateContent",
    auth_headers: &["x-goog-api-key"],
    default_model: "gemini-1.5-flash",
};

pub const DEEPSEEK_EVIDENCE: ProviderApiEvidence = ProviderApiEvidence {
    provider: "deepseek",
    doc_url: "https://api-docs.deepseek.com/api/create-chat-completion",
    checked_at: "2026-07-12",
    base_url: "https://api.deepseek.com",
    endpoint_path: "/chat/completions",
    auth_headers: &["Authorization: Bearer"],
    default_model: "deepseek-v4-flash",
};

pub const PROVIDER_API_EVIDENCE: &[ProviderApiEvidence] = &[
    OPENAI_EVIDENCE,
    CLAUDE_EVIDENCE,
    GEMINI_EVIDENCE,
    DEEPSEEK_EVIDENCE,
];

pub fn provider_evidence(name: &str) -> Option<&'static ProviderApiEvidence> {
    PROVIDER_API_EVIDENCE
        .iter()
        .find(|evidence| evidence.provider == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_registry_is_unique_and_complete() {
        let mut names = PROVIDER_API_EVIDENCE
            .iter()
            .map(|evidence| evidence.provider)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();

        assert_eq!(names.len(), PROVIDER_API_EVIDENCE.len());
        assert!(PROVIDER_API_EVIDENCE.iter().all(|evidence| {
            evidence.doc_url.starts_with("https://")
                && evidence.base_url.starts_with("https://")
                && evidence.endpoint_path.starts_with('/')
                && !evidence.default_model.is_empty()
        }));
    }

    #[test]
    fn deepseek_uses_its_official_non_v1_endpoint() {
        assert_eq!(
            DEEPSEEK_EVIDENCE.endpoint_url(DEEPSEEK_EVIDENCE.base_url, "deepseek-v4-flash"),
            "https://api.deepseek.com/chat/completions"
        );
    }
}
