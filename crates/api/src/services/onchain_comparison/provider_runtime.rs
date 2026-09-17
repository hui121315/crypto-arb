use super::provider_types::ProviderRuntime;
use super::{cow, okx};

const JUPITER_KEYED_INTERVAL_MS: i64 = 2_250;
const JUPITER_KEYLESS_INTERVAL_MS: i64 = 4_500;
const JUPITER_KEYED_REQUEST_GAP_MS: u64 = 1_050;
const JUPITER_KEYLESS_REQUEST_GAP_MS: u64 = 2_100;
const ZEROEX_INTERVAL_MS: i64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct QuoteRetryBackoff {
    pub(super) base_delay_ms: i64,
    pub(super) max_delay_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct JupiterRateProfile {
    pub(super) quote_interval_ms: i64,
    pub(super) request_gap_ms: u64,
}

pub(super) const fn jupiter_rate_profile(keyed: bool) -> JupiterRateProfile {
    if keyed {
        JupiterRateProfile {
            quote_interval_ms: JUPITER_KEYED_INTERVAL_MS,
            request_gap_ms: JUPITER_KEYED_REQUEST_GAP_MS,
        }
    } else {
        JupiterRateProfile {
            quote_interval_ms: JUPITER_KEYLESS_INTERVAL_MS,
            request_gap_ms: JUPITER_KEYLESS_REQUEST_GAP_MS,
        }
    }
}

pub(super) fn provider_runtime(provider: &str) -> ProviderRuntime {
    match provider {
        "jupiter_swap_v2" => jupiter_runtime(false),
        "jupiter_swap_v2_keyed" => jupiter_runtime(true),
        "zeroex_swap_v2" => zeroex_runtime(),
        "okx_dex_v6" => okx_runtime(),
        "cow_protocol" => ProviderRuntime {
            configured: true,
            problem: None,
            quote_interval_ms: cow::COW_INTERVAL_MS,
        },
        provider => ProviderRuntime {
            configured: false,
            problem: Some(format!("unsupported quote provider {provider}")),
            quote_interval_ms: JUPITER_KEYLESS_INTERVAL_MS,
        },
    }
}

pub(super) fn quote_retry_backoff(
    reason: &str,
    quote_interval_ms: i64,
) -> Option<QuoteRetryBackoff> {
    let reason = reason.to_ascii_lowercase();
    if reason.contains("http 429")
        || reason.contains("rate limit")
        || reason.contains("rate_limit_wait")
    {
        return Some(QuoteRetryBackoff {
            base_delay_ms: 1_000,
            max_delay_ms: 10_000,
        });
    }
    if reason.contains("transport=timeout")
        || reason.contains("transport=request failed")
        || reason.contains("timed out")
        || reason.contains("timeout")
        || reason.contains("request failed")
        || reason.contains("无法连接")
        || reason.contains("网络传输失败")
        || reason.contains("response read failed")
    {
        return Some(QuoteRetryBackoff {
            base_delay_ms: quote_interval_ms.max(2_500),
            max_delay_ms: 10_000,
        });
    }
    if reason.contains("http 5") {
        return Some(QuoteRetryBackoff {
            base_delay_ms: quote_interval_ms.saturating_mul(2).max(5_000),
            max_delay_ms: 30_000,
        });
    }
    None
}

pub(super) fn env_key(name: &str) -> Option<String> {
    crate::services::venue_credentials::secret(name)
}

fn jupiter_runtime(keyed: bool) -> ProviderRuntime {
    let ready = !keyed || env_key("JUPITER_API_KEY").is_some();
    let rate = jupiter_rate_profile(keyed);
    ProviderRuntime {
        configured: ready,
        problem: (!ready).then(|| {
            "当前选择 Jupiter API Key 加速路由；请先在 Provider 凭证中填写 JUPITER_API_KEY"
                .to_owned()
        }),
        quote_interval_ms: rate.quote_interval_ms,
    }
}

fn zeroex_runtime() -> ProviderRuntime {
    let ready = env_key("ZEROX_API_KEY").is_some();
    ProviderRuntime {
        configured: ready,
        problem: (!ready).then(|| "需要配置 ZEROX_API_KEY 才能读取 0x EVM 指示价".to_owned()),
        quote_interval_ms: ZEROEX_INTERVAL_MS,
    }
}

fn okx_runtime() -> ProviderRuntime {
    match okx::credentials() {
        Ok(_) => ProviderRuntime {
            configured: true,
            problem: None,
            quote_interval_ms: okx::OKX_INTERVAL_MS,
        },
        Err(problem) => ProviderRuntime {
            configured: false,
            problem: Some(problem),
            quote_interval_ms: okx::OKX_INTERVAL_MS,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jupiter_pair_profiles_respect_two_request_rate_budgets() {
        assert_eq!(
            jupiter_rate_profile(false),
            JupiterRateProfile {
                quote_interval_ms: 4_500,
                request_gap_ms: 2_100,
            }
        );
        assert_eq!(
            jupiter_rate_profile(true),
            JupiterRateProfile {
                quote_interval_ms: 2_250,
                request_gap_ms: 1_050,
            }
        );
        assert!(provider_runtime("jupiter_swap_v2").configured);
        assert_eq!(provider_runtime("jupiter_swap_v2").quote_interval_ms, 4_500);
    }

    #[test]
    fn transient_quote_failures_receive_a_bounded_backoff_base() {
        assert_eq!(
            quote_retry_backoff("Jupiter quote returned HTTP 429", 2_250),
            Some(QuoteRetryBackoff {
                base_delay_ms: 1_000,
                max_delay_ms: 10_000,
            })
        );
        assert_eq!(
            quote_retry_backoff("rate_limit_wait · retry_after_ms=3250", 2_250),
            Some(QuoteRetryBackoff {
                base_delay_ms: 1_000,
                max_delay_ms: 10_000,
            })
        );
        assert_eq!(
            quote_retry_backoff("0x quote request failed: timeout", 1_000),
            Some(QuoteRetryBackoff {
                base_delay_ms: 2_500,
                max_delay_ms: 10_000,
            })
        );
        assert_eq!(
            quote_retry_backoff(
                "Jupiter 报价无法连接（api.jup.ag） · transport=request failed",
                2_250,
            ),
            Some(QuoteRetryBackoff {
                base_delay_ms: 2_500,
                max_delay_ms: 10_000,
            })
        );
        assert_eq!(
            quote_retry_backoff("Jupiter quote returned HTTP 503", 2_250),
            Some(QuoteRetryBackoff {
                base_delay_ms: 5_000,
                max_delay_ms: 30_000,
            })
        );
        assert_eq!(
            quote_retry_backoff("quote identity is invalid", 2_250),
            None
        );
    }
}
