//! 应用配置加载。
//!
//! 优先级：环境变量（`APP_*`）> `config.toml` > 默认值。

use crate::error::{AppError, AppResult};
use figment::providers::{Env, Format, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::PathBuf;

pub mod env_file;

pub const DEFAULT_RUNTIME_DIR_NAME: &str = "crossline-omni";
pub const DEFAULT_PORTFOLIO_NAV_FILE: &str = "portfolio_nav.sqlite";
pub const DEFAULT_WATCHLIST_ALERTS_FILE: &str = "watchlist_alerts.sqlite";
pub const DEFAULT_EXECUTION_RUN_LEDGER_FILE: &str = "execution_runs.jsonl";
pub const DEFAULT_ONCHAIN_EXECUTION_RUN_LEDGER_FILE: &str = "onchain_execution_runs.jsonl";
pub const DEFAULT_ONCHAIN_REPLENISHMENT_LEDGER_FILE: &str = "onchain_replenishment_plans.jsonl";
pub const DEFAULT_ONCHAIN_CROSS_CHAIN_LEDGER_FILE: &str = "onchain_cross_chain_runs.jsonl";
pub const DEFAULT_EXECUTION_LEDGER_FILE: &str = "execution_ledger_events.jsonl";
pub const DEFAULT_ORDER_SNAPSHOT_FILE: &str = "order_snapshots.jsonl";
pub const DEFAULT_CLOSE_RUN_LEDGER_FILE: &str = "close_runs.jsonl";
pub const DEFAULT_AUTOMATION_CONFIG_FILE: &str = "automation_config.json";
pub const DEFAULT_WEBHOOK_OUTBOX_FILE: &str = "webhook_outbox.sqlite";
pub const DEFAULT_MARKET_SUBSCRIPTIONS_FILE: &str = "market_subscriptions.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub log_level: String,
    pub log_format: String,
    pub redis_url: Option<String>,
    pub storage: StorageConfig,
    pub history: HistoryConfig,
    pub alerts: AlertsConfig,
    #[serde(default)]
    pub arbitrage: ArbitrageRuntimeConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub api_surface: ApiSurfaceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub postgres_url: Option<String>,
    #[serde(default = "default_storage_data_dir")]
    pub data_dir: String,
    pub portfolio_nav_path: Option<String>,
    #[serde(default)]
    pub watchlist_alerts_path: Option<String>,
    #[serde(default)]
    pub execution_run_ledger_path: Option<String>,
    #[serde(default = "default_onchain_execution_run_ledger_path")]
    pub onchain_execution_run_ledger_path: Option<String>,
    #[serde(default = "default_onchain_replenishment_ledger_path")]
    pub onchain_replenishment_ledger_path: Option<String>,
    #[serde(default = "default_onchain_cross_chain_ledger_path")]
    pub onchain_cross_chain_ledger_path: Option<String>,
    #[serde(default)]
    pub execution_ledger_path: Option<String>,
    #[serde(default)]
    pub order_snapshot_path: Option<String>,
    #[serde(default)]
    pub close_run_ledger_path: Option<String>,
    #[serde(default)]
    pub automation_config_path: Option<String>,
    #[serde(default)]
    pub webhook_outbox_path: Option<String>,
    #[serde(default)]
    pub market_subscriptions_path: Option<String>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        #[cfg(test)]
        let watchlist_alerts_path = None;
        #[cfg(not(test))]
        let watchlist_alerts_path = Some(DEFAULT_WATCHLIST_ALERTS_FILE.to_owned());
        #[cfg(test)]
        let close_run_ledger_path = None;
        #[cfg(not(test))]
        let close_run_ledger_path = Some(DEFAULT_CLOSE_RUN_LEDGER_FILE.to_owned());
        #[cfg(test)]
        let execution_run_ledger_path = None;
        #[cfg(not(test))]
        let execution_run_ledger_path = Some(DEFAULT_EXECUTION_RUN_LEDGER_FILE.to_owned());
        #[cfg(test)]
        let onchain_execution_run_ledger_path = None;
        #[cfg(not(test))]
        let onchain_execution_run_ledger_path =
            Some(DEFAULT_ONCHAIN_EXECUTION_RUN_LEDGER_FILE.to_owned());
        #[cfg(test)]
        let onchain_replenishment_ledger_path = None;
        #[cfg(not(test))]
        let onchain_replenishment_ledger_path =
            Some(DEFAULT_ONCHAIN_REPLENISHMENT_LEDGER_FILE.to_owned());
        #[cfg(test)]
        let onchain_cross_chain_ledger_path = None;
        #[cfg(not(test))]
        let onchain_cross_chain_ledger_path =
            Some(DEFAULT_ONCHAIN_CROSS_CHAIN_LEDGER_FILE.to_owned());
        #[cfg(test)]
        let execution_ledger_path = None;
        #[cfg(not(test))]
        let execution_ledger_path = Some(DEFAULT_EXECUTION_LEDGER_FILE.to_owned());
        #[cfg(test)]
        let order_snapshot_path = None;
        #[cfg(not(test))]
        let order_snapshot_path = Some(DEFAULT_ORDER_SNAPSHOT_FILE.to_owned());
        #[cfg(test)]
        let automation_config_path = None;
        #[cfg(not(test))]
        let automation_config_path = Some(DEFAULT_AUTOMATION_CONFIG_FILE.to_owned());
        #[cfg(test)]
        let webhook_outbox_path = None;
        #[cfg(not(test))]
        let webhook_outbox_path = Some(DEFAULT_WEBHOOK_OUTBOX_FILE.to_owned());
        #[cfg(test)]
        let market_subscriptions_path = None;
        #[cfg(not(test))]
        let market_subscriptions_path = Some(DEFAULT_MARKET_SUBSCRIPTIONS_FILE.to_owned());
        Self {
            postgres_url: None,
            data_dir: default_storage_data_dir(),
            portfolio_nav_path: Some(DEFAULT_PORTFOLIO_NAV_FILE.to_owned()),
            watchlist_alerts_path,
            execution_run_ledger_path,
            onchain_execution_run_ledger_path,
            onchain_replenishment_ledger_path,
            onchain_cross_chain_ledger_path,
            execution_ledger_path,
            order_snapshot_path,
            close_run_ledger_path,
            automation_config_path,
            webhook_outbox_path,
            market_subscriptions_path,
        }
    }
}

impl StorageConfig {
    pub fn data_dir_path(&self) -> PathBuf {
        let value = self.data_dir.trim();
        if value.is_empty() {
            default_runtime_data_dir()
        } else {
            PathBuf::from(value)
        }
    }

    pub fn resolve_runtime_path(&self, value: &str) -> PathBuf {
        let path = PathBuf::from(value.trim());
        if path.is_absolute() {
            path
        } else {
            self.data_dir_path().join(path)
        }
    }
}

fn default_storage_data_dir() -> String {
    default_runtime_data_dir().display().to_string()
}

#[allow(clippy::unnecessary_wraps)] // Serde default must match the optional config field type.
fn default_onchain_execution_run_ledger_path() -> Option<String> {
    Some(DEFAULT_ONCHAIN_EXECUTION_RUN_LEDGER_FILE.to_owned())
}

#[allow(clippy::unnecessary_wraps)]
fn default_onchain_replenishment_ledger_path() -> Option<String> {
    Some(DEFAULT_ONCHAIN_REPLENISHMENT_LEDGER_FILE.to_owned())
}

#[allow(clippy::unnecessary_wraps)]
fn default_onchain_cross_chain_ledger_path() -> Option<String> {
    Some(DEFAULT_ONCHAIN_CROSS_CHAIN_LEDGER_FILE.to_owned())
}

fn default_runtime_data_dir() -> PathBuf {
    if let Some(path) = env_path("XDG_DATA_HOME") {
        return path.join(DEFAULT_RUNTIME_DIR_NAME);
    }
    if cfg!(target_os = "macos") {
        if let Some(home) = env_path("HOME") {
            return home
                .join("Library")
                .join("Application Support")
                .join(DEFAULT_RUNTIME_DIR_NAME);
        }
    }
    if let Some(home) = env_path("HOME") {
        return home
            .join(".local")
            .join("share")
            .join(DEFAULT_RUNTIME_DIR_NAME);
    }
    std::env::temp_dir().join(DEFAULT_RUNTIME_DIR_NAME)
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryConfig {
    pub enabled: bool,
    pub opportunity_ttl_days: u32,
    #[serde(default)]
    pub postgres_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlertsConfig {
    pub telegram: TelegramAlertsConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelegramAlertsConfig {
    pub default_chat_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrageRuntimeConfig {
    pub total_capital_usd: f64,
    pub risk_tolerance: f64,
}

/// 安全配置：bearer token 认证、可信 actor 标签、CORS 白名单、审计日志路径。
///
/// 默认值（未配置任何字段）等同 dev 模式：
/// - `auth_token = None` → 不要求任何认证（仅 dev / 单元测试）
/// - `allowed_origins = []` → 允许所有源（仅 dev）
/// - `audit_log_path = None` → 凭证更新不写审计日志
///
/// 生产部署 **必须** 设置 `APP_SECURITY__AUTH_TOKEN`、
/// `APP_SECURITY__ALLOWED_ORIGINS` 与 `APP_SECURITY__AUDIT_LOG_PATH`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// HTTP bearer token。未设置时不强制认证（仅 dev / 测试）。
    #[serde(default)]
    pub auth_token: Option<String>,

    /// 通过 bearer 校验后写入 audit 的静态 operator 标签；不接受来自客户端的 actor header。
    #[serde(default)]
    pub auth_actor_label: Option<String>,

    /// CORS 允许的 Origin 列表（如 `https://app.example.com`）。
    /// 空列表时允许所有源（仅 dev）。
    #[serde(default)]
    pub allowed_origins: Vec<String>,

    /// 审计日志 JSONL 文件路径。未设置时不写审计日志。
    #[serde(default)]
    pub audit_log_path: Option<String>,

    /// 低风险路径豁免列表（不要求 bearer token）。
    /// `/health` 仅精确豁免 liveness；`/health/ready` 属于 readiness，必须按 route
    /// inventory 的 bearer policy 保护。
    #[serde(default = "default_auth_exempt_paths")]
    pub auth_exempt_paths: Vec<String>,
}

fn default_auth_exempt_paths() -> Vec<String> {
    vec!["/health".to_owned()]
}

const HEALTH_AUTH_EXEMPT_PREFIX: &str = "/health";
const AUTH_EXEMPT_LOW_RISK_PREFIXES: [&str; 3] = ["/health", "/metrics", "/api/metrics"];

impl SecurityConfig {
    /// 当前是否要求 bearer token。
    pub fn auth_required(&self) -> bool {
        self.auth_token
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
    }

    /// 返回经配置校验的静态 operator 标签。token 指纹仍会附加在标签后，避免标签本身
    /// 被误当作 credential 或可伪造的客户端身份。
    pub fn verified_actor_label(&self) -> Option<&str> {
        self.auth_actor_label
            .as_deref()
            .map(str::trim)
            .filter(|label| !label.is_empty())
    }

    /// 当前是否配置了真实审计日志 sink。
    fn audit_log_configured(&self) -> bool {
        self.audit_log_path
            .as_deref()
            .is_some_and(|path| !path.trim().is_empty())
    }

    /// 路径是否豁免认证。
    pub fn is_path_exempt(&self, path: &str) -> bool {
        path_matches_liveness(path)
            || self
                .auth_exempt_paths
                .iter()
                .any(|prefix| path_matches_exempt_scope(path, prefix.as_str()))
    }

    pub fn ensure_auth_exempt_paths_safe(&self) -> AppResult<()> {
        for raw in &self.auth_exempt_paths {
            let prefix = raw.trim();
            if prefix.is_empty() {
                continue;
            }
            if !prefix.starts_with('/') {
                return Err(AppError::Config(format!(
                    "invalid auth exempt path `{prefix}`: path prefixes must start with /"
                )));
            }
            if !AUTH_EXEMPT_LOW_RISK_PREFIXES.contains(&prefix) {
                return Err(AppError::Config(format!(
                    "invalid auth exempt path `{prefix}`: only /health, /metrics, and /api/metrics may bypass bearer auth"
                )));
            }
        }
        Ok(())
    }

    fn ensure_actor_label_safe(&self) -> AppResult<()> {
        let Some(label) = self.verified_actor_label() else {
            return Ok(());
        };
        let valid = label.len() <= 64
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
        if valid {
            return Ok(());
        }
        Err(AppError::Config(
            "invalid APP_SECURITY__AUTH_ACTOR_LABEL: use 1-64 ASCII letters, digits, '.', '_' or '-'"
                .to_owned(),
        ))
    }
}

fn path_matches_liveness(path: &str) -> bool {
    path == HEALTH_AUTH_EXEMPT_PREFIX
}

fn path_matches_exempt_scope(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    if prefix == HEALTH_AUTH_EXEMPT_PREFIX {
        return path_matches_liveness(path);
    }
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

/// 非主线 / legacy / diagnostic API 暴露开关（runtime gate）。
///
/// 主线 P0 router（arbitrage / exchanges / portfolio / history / venues / system /
/// trading / health / metrics / websocket / review）始终暴露；
/// 下列辅助 router 可按部署需要关闭以缩小攻击面与 DTO 漂移面。
/// 默认姿态：当前主工作台未消费的 legacy/diagnostic/helper 默认关闭。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiSurfaceConfig {
    /// `/api/chat`、`/api/chat/providers`：LLM chat 辅助端点（当前主工作台未消费）。默认关。
    #[serde(default)]
    pub chat: bool,
    /// `/api/llm/*` LLM 诊断端点（explain / diagnose / daily-brief，无前端引用）。默认关。
    #[serde(default)]
    pub llm_diagnostics: bool,
    /// `/api/options/*`：期权分析辅助端点（当前主工作台未消费）。默认关。
    #[serde(default)]
    pub options: bool,
    /// `/api/watchlist*` 与 `/api/alerts/rules*`：观察/告警支撑面（当前主交易闭环未消费）。默认关。
    #[serde(default)]
    pub watchlist_alerts: bool,
    /// `/api/v1/spot/*`：只读现货诊断端点（Settings 手动调试专用）。默认关。
    #[serde(default)]
    pub spot_v1: bool,
    /// `/api/v1/strategy/*`：旧版策略诊断端点。主工作台使用 `/api/strategy/main-kinds`；默认关。
    #[serde(default)]
    pub strategy_v1: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            // host 默认 127.0.0.1：仅本机访问。
            // 生产部署需显式 `APP_HOST=0.0.0.0` + 配套反代/认证（见 SecurityConfig）。
            // 历史默认 0.0.0.0 配合 CORS Any + 无认证 = 公网攻击面（AUDIT_02_API.md P0 #1）。
            host: "127.0.0.1".to_owned(),
            port: 8000,
            log_level: "info".to_owned(),
            log_format: "pretty".to_owned(),
            redis_url: None,
            storage: StorageConfig::default(),
            history: HistoryConfig::default(),
            alerts: AlertsConfig::default(),
            arbitrage: ArbitrageRuntimeConfig::default(),
            security: SecurityConfig::default(),
            api_surface: ApiSurfaceConfig::default(),
        }
    }
}

impl Default for ArbitrageRuntimeConfig {
    fn default() -> Self {
        Self {
            total_capital_usd: 100_000.0,
            risk_tolerance: 0.5,
        }
    }
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            opportunity_ttl_days: 30,
            postgres_url: None,
        }
    }
}

impl HistoryConfig {
    pub fn database_url<'a>(&'a self, storage: &'a StorageConfig) -> Option<&'a str> {
        self.postgres_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .or_else(|| {
                storage
                    .postgres_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
            })
    }
}

impl AppConfig {
    /// 从环境变量与可选 `config.toml` 文件加载配置。
    pub fn load() -> AppResult<Self> {
        env_file::load()?;

        let figment = Figment::from(figment::providers::Serialized::defaults(
            AppConfig::default(),
        ))
        .merge(Toml::file("config.toml"))
        .merge(Env::prefixed("APP_").split("__"));

        let mut config = figment
            .extract::<AppConfig>()
            .map_err(|e| AppError::Config(e.to_string()))?;
        config.normalize_runtime_paths();
        Ok(config)
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// `host` 是否仅绑定本机 loopback（`127.0.0.1` / `::1` / `localhost`）。
    /// 无法解析为 IP 的主机名保守按非 loopback 处理（要求认证）。
    pub fn is_loopback_bind(&self) -> bool {
        let host = self.host.trim();
        if host.eq_ignore_ascii_case("localhost") {
            return true;
        }
        host.parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
    }

    /// 公网绑定安全门槛：绑定到非 loopback 接口时必须配置 `auth_token`、
    /// 显式 CORS origin 与真实 audit log path，否则拒绝启动。
    /// loopback（仅本机可达）允许无认证（dev / 测试）。这是防止复制 `.env.example`
    /// 把服务以 `0.0.0.0` + 无鉴权暴露到不可信网络的硬失败闸门。
    pub fn ensure_bind_security(&self) -> AppResult<()> {
        self.security.ensure_auth_exempt_paths_safe()?;
        self.security.ensure_actor_label_safe()?;
        if self.is_loopback_bind() {
            return Ok(());
        }
        if !self.security.auth_required() {
            return Err(AppError::Config(format!(
                "refusing to start: APP_HOST={} exposes a non-loopback interface without authentication; \
                 set APP_SECURITY__AUTH_TOKEN (recommended) or bind APP_HOST=127.0.0.1 for local-only access",
                self.host
            )));
        }
        if self.security.allowed_origins.is_empty()
            || self
                .security
                .allowed_origins
                .iter()
                .any(|origin| origin.trim() == "*")
        {
            return Err(AppError::Config(format!(
                "refusing to start: APP_HOST={} exposes a non-loopback interface with open CORS; \
                 configure APP_SECURITY__ALLOWED_ORIGINS with explicit origins (no `*`) or bind APP_HOST=127.0.0.1 for local-only access",
                self.host
            )));
        }
        if !self.security.audit_log_configured() {
            return Err(AppError::Config(format!(
                "refusing to start: APP_HOST={} exposes a non-loopback interface without audit logging; \
                 set APP_SECURITY__AUDIT_LOG_PATH or bind APP_HOST=127.0.0.1 for local-only access",
                self.host
            )));
        }
        Ok(())
    }

    fn normalize_runtime_paths(&mut self) {
        self.storage.portfolio_nav_path =
            normalize_runtime_path(self.storage.portfolio_nav_path.as_deref(), &self.storage);
        self.storage.watchlist_alerts_path =
            normalize_runtime_path(self.storage.watchlist_alerts_path.as_deref(), &self.storage);
        self.storage.execution_run_ledger_path = normalize_runtime_path(
            self.storage.execution_run_ledger_path.as_deref(),
            &self.storage,
        );
        self.storage.execution_ledger_path =
            normalize_runtime_path(self.storage.execution_ledger_path.as_deref(), &self.storage);
        self.storage.order_snapshot_path =
            normalize_runtime_path(self.storage.order_snapshot_path.as_deref(), &self.storage);
        self.storage.close_run_ledger_path =
            normalize_runtime_path(self.storage.close_run_ledger_path.as_deref(), &self.storage);
        self.storage.automation_config_path = normalize_runtime_path(
            self.storage.automation_config_path.as_deref(),
            &self.storage,
        );
        self.storage.webhook_outbox_path =
            normalize_runtime_path(self.storage.webhook_outbox_path.as_deref(), &self.storage);
        self.security.audit_log_path =
            normalize_runtime_path(self.security.audit_log_path.as_deref(), &self.storage);
    }
}

fn normalize_runtime_path(value: Option<&str>, storage: &StorageConfig) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| storage.resolve_runtime_path(value).display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    fn config_with(host: &str, auth_token: Option<&str>) -> AppConfig {
        AppConfig {
            host: host.to_owned(),
            security: SecurityConfig {
                auth_token: auth_token.map(str::to_owned),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn loopback_without_auth_is_allowed() {
        for host in ["127.0.0.1", "::1", "localhost", "LOCALHOST"] {
            let config = config_with(host, None);
            assert!(config.is_loopback_bind(), "{host} should be loopback");
            assert!(
                config.ensure_bind_security().is_ok(),
                "{host} no-auth must pass"
            );
        }
    }

    #[test]
    fn non_loopback_without_auth_is_rejected() {
        for host in ["0.0.0.0", "192.168.1.10", "::", "my-host"] {
            let config = config_with(host, None);
            assert!(!config.is_loopback_bind(), "{host} should not be loopback");
            assert!(
                config.ensure_bind_security().is_err(),
                "{host} no-auth must hard fail"
            );
        }
    }

    #[test]
    fn non_loopback_with_auth_is_allowed() {
        let mut config = config_with("0.0.0.0", Some("secret-token"));
        config.security.allowed_origins = vec!["http://127.0.0.1:8080".to_owned()];
        config.security.audit_log_path = Some("security_audit.jsonl".to_owned());
        assert!(config.ensure_bind_security().is_ok());
    }

    #[test]
    fn cors_origin_env_array_loads_and_satisfies_public_bind_contract() {
        let temp = std::env::temp_dir().join(format!(
            "crossline-cors-env-contract-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp).expect("create isolated config directory");
        let mut command = Command::new(std::env::current_exe().expect("current test binary"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("APP_") {
                command.env_remove(key);
            }
        }
        let output = command
            .current_dir(&temp)
            .arg("--exact")
            .arg("config::tests::cors_origin_env_child")
            .arg("--nocapture")
            .env("CROSSLINE_CORS_ENV_CHILD", "1")
            .env("APP_HOST", "0.0.0.0")
            .env("APP_SECURITY__AUTH_TOKEN", "operator-token")
            .env(
                "APP_SECURITY__ALLOWED_ORIGINS",
                r#"["https://operator.example","http://127.0.0.1:8080"]"#,
            )
            .env(
                "APP_SECURITY__AUDIT_LOG_PATH",
                "/tmp/crossline-security-audit.jsonl",
            )
            .output()
            .expect("run isolated environment config test");
        let _ = std::fs::remove_dir_all(temp);

        assert!(
            output.status.success(),
            "child config test failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn cors_origin_env_child() {
        if std::env::var("CROSSLINE_CORS_ENV_CHILD").as_deref() != Ok("1") {
            return;
        }
        let config = AppConfig::load().expect("environment security contract must load");

        assert_eq!(
            config.security.allowed_origins,
            vec![
                "https://operator.example".to_owned(),
                "http://127.0.0.1:8080".to_owned(),
            ]
        );
        assert!(config.ensure_bind_security().is_ok());
    }

    #[test]
    fn non_loopback_with_auth_requires_cors_origins() {
        let config = config_with("0.0.0.0", Some("secret-token"));
        assert!(config.ensure_bind_security().is_err());
    }

    #[test]
    fn non_loopback_with_auth_and_cors_requires_audit_log() {
        let mut config = config_with("0.0.0.0", Some("secret-token"));
        config.security.allowed_origins = vec!["http://127.0.0.1:8080".to_owned()];

        let error = config
            .ensure_bind_security()
            .expect_err("public bind with auth and CORS still needs audit log path");

        assert!(
            error.to_string().contains("APP_SECURITY__AUDIT_LOG_PATH"),
            "error should name the required audit config key: {error}"
        );
    }

    #[test]
    fn non_loopback_with_wildcard_cors_origin_is_rejected() {
        for wildcard in ["*", " * "] {
            let mut config = config_with("0.0.0.0", Some("secret-token"));
            config.security.allowed_origins = vec![wildcard.to_owned()];
            assert!(
                config.ensure_bind_security().is_err(),
                "wildcard origin {wildcard:?} must hard fail on a public bind"
            );
        }
    }

    #[test]
    fn blank_auth_token_does_not_satisfy_public_bind() {
        let config = config_with("0.0.0.0", Some("   "));
        assert!(!config.security.auth_required());
        assert!(config.ensure_bind_security().is_err());
    }

    #[test]
    fn health_auth_exemption_survives_partial_security_config() {
        let security = SecurityConfig {
            auth_token: Some("token".to_owned()),
            auth_actor_label: None,
            allowed_origins: vec!["http://127.0.0.1:8080".to_owned()],
            audit_log_path: None,
            auth_exempt_paths: Vec::new(),
        };

        assert!(security.auth_required());
        assert!(security.is_path_exempt("/health"));
        assert!(!security.is_path_exempt("/health/ready"));
        assert!(!security.is_path_exempt("/api/system/health"));
    }

    #[test]
    fn auth_exemption_matches_only_exact_or_child_paths() {
        let security = SecurityConfig {
            auth_exempt_paths: vec!["/metrics".to_owned(), "/api/metrics".to_owned()],
            ..Default::default()
        };

        assert!(security.is_path_exempt("/health"));
        assert!(!security.is_path_exempt("/health/ready"));
        assert!(!security.is_path_exempt("/healthz"));
        assert!(security.is_path_exempt("/metrics"));
        assert!(security.is_path_exempt("/metrics/prometheus"));
        assert!(!security.is_path_exempt("/metrics-anything"));
        assert!(security.is_path_exempt("/api/metrics"));
        assert!(security.is_path_exempt("/api/metrics/runtime"));
        assert!(!security.is_path_exempt("/api/metrics-extra"));
    }

    #[test]
    fn high_risk_api_prefix_cannot_bypass_auth() {
        for prefix in ["/api", "/api/", "/api/trading", "/api/exchanges"] {
            let security = SecurityConfig {
                auth_exempt_paths: vec![prefix.to_owned()],
                ..Default::default()
            };
            assert!(
                security.ensure_auth_exempt_paths_safe().is_err(),
                "{prefix} must not bypass auth"
            );
        }
    }

    #[test]
    fn low_risk_exempt_paths_are_allowed() {
        let security = SecurityConfig {
            auth_exempt_paths: vec![
                "/health".to_owned(),
                "/metrics".to_owned(),
                "/api/metrics".to_owned(),
            ],
            ..Default::default()
        };

        assert!(security.ensure_auth_exempt_paths_safe().is_ok());
    }

    #[test]
    fn configured_actor_label_must_be_safe_and_static() {
        let mut config = config_with("127.0.0.1", Some("token"));
        config.security.auth_actor_label = Some("operator.primary-1".to_owned());
        assert_eq!(
            config.security.verified_actor_label(),
            Some("operator.primary-1")
        );
        assert!(config.ensure_bind_security().is_ok());

        config.security.auth_actor_label = Some("operator name".to_owned());
        let error = config
            .ensure_bind_security()
            .expect_err("whitespace in actor label must be rejected");
        assert!(error.to_string().contains("AUTH_ACTOR_LABEL"));
    }

    #[test]
    fn api_surface_defaults_disable_non_p0_helpers() {
        let surface = ApiSurfaceConfig::default();

        assert!(!surface.strategy_v1);
        assert!(!surface.chat);
        assert!(!surface.llm_diagnostics);
        assert!(!surface.options);
        assert!(!surface.watchlist_alerts);
        assert!(!surface.spot_v1);
    }

    #[test]
    fn history_database_url_prefers_isolated_override_and_keeps_legacy_fallback() {
        let storage = StorageConfig {
            postgres_url: Some("postgres://storage".to_owned()),
            ..StorageConfig::default()
        };
        let history = HistoryConfig::default();

        assert_eq!(history.database_url(&storage), Some("postgres://storage"));

        let history = HistoryConfig {
            postgres_url: Some(" postgres://history ".to_owned()),
            ..HistoryConfig::default()
        };
        assert_eq!(history.database_url(&storage), Some("postgres://history"));
    }

    #[test]
    fn default_nav_storage_uses_ignored_runtime_dir() {
        let storage = StorageConfig::default();
        let data_dir = storage.data_dir_path();
        let nav_path = storage
            .portfolio_nav_path
            .as_deref()
            .map(|value| storage.resolve_runtime_path(value));

        assert_eq!(
            storage.portfolio_nav_path.as_deref(),
            Some(DEFAULT_PORTFOLIO_NAV_FILE)
        );
        assert!(data_dir.is_absolute(), "runtime data dir must be fixed");
        assert!(
            data_dir.ends_with(Path::new(DEFAULT_RUNTIME_DIR_NAME)),
            "default data dir must be namespaced"
        );
        let expected_nav_path = data_dir.join(DEFAULT_PORTFOLIO_NAV_FILE);
        assert_eq!(nav_path.as_deref(), Some(expected_nav_path.as_path()));
        assert_eq!(storage.execution_run_ledger_path, None);
        assert_eq!(storage.onchain_execution_run_ledger_path, None);
        assert_eq!(storage.onchain_replenishment_ledger_path, None);
        assert_eq!(storage.onchain_cross_chain_ledger_path, None);
        assert_eq!(storage.watchlist_alerts_path, None);
        assert_eq!(storage.execution_ledger_path, None);
        assert_eq!(storage.order_snapshot_path, None);
        assert_eq!(storage.close_run_ledger_path, None);
        assert_eq!(storage.automation_config_path, None);
        assert_eq!(storage.webhook_outbox_path, None);
    }

    #[test]
    fn legacy_storage_config_enables_onchain_execution_recovery_log() {
        let mut value = serde_json::to_value(StorageConfig::default()).expect("storage encodes");
        value
            .as_object_mut()
            .expect("storage is an object")
            .remove("onchain_execution_run_ledger_path");

        let restored: StorageConfig = serde_json::from_value(value).expect("storage decodes");

        assert_eq!(
            restored.onchain_execution_run_ledger_path.as_deref(),
            Some(DEFAULT_ONCHAIN_EXECUTION_RUN_LEDGER_FILE)
        );
    }

    #[test]
    fn legacy_storage_config_enables_onchain_replenishment_recovery_log() {
        let mut value = serde_json::to_value(StorageConfig::default()).expect("storage encodes");
        value
            .as_object_mut()
            .expect("storage is an object")
            .remove("onchain_replenishment_ledger_path");

        let restored: StorageConfig = serde_json::from_value(value).expect("storage decodes");

        assert_eq!(
            restored.onchain_replenishment_ledger_path.as_deref(),
            Some(DEFAULT_ONCHAIN_REPLENISHMENT_LEDGER_FILE)
        );
    }

    #[test]
    fn legacy_storage_config_enables_onchain_cross_chain_recovery_log() {
        let mut value = serde_json::to_value(StorageConfig::default()).expect("storage encodes");
        value
            .as_object_mut()
            .expect("storage is an object")
            .remove("onchain_cross_chain_ledger_path");

        let restored: StorageConfig = serde_json::from_value(value).expect("storage decodes");

        assert_eq!(
            restored.onchain_cross_chain_ledger_path.as_deref(),
            Some(DEFAULT_ONCHAIN_CROSS_CHAIN_LEDGER_FILE)
        );
    }

    #[test]
    fn relative_runtime_paths_are_resolved_under_data_dir() {
        let storage = StorageConfig {
            data_dir: "/tmp/crossline-test".to_owned(),
            portfolio_nav_path: Some("portfolio/nav.sqlite".to_owned()),
            watchlist_alerts_path: Some("alerts/watchlist_alerts.sqlite".to_owned()),
            execution_run_ledger_path: Some("execution/execution_runs.jsonl".to_owned()),
            execution_ledger_path: Some("execution/execution_ledger_events.jsonl".to_owned()),
            order_snapshot_path: Some("execution/order_snapshots.jsonl".to_owned()),
            close_run_ledger_path: Some("close/close_runs.jsonl".to_owned()),
            automation_config_path: Some("automation/config.json".to_owned()),
            webhook_outbox_path: Some("webhook/outbox.sqlite".to_owned()),
            ..Default::default()
        };

        assert_eq!(
            storage.resolve_runtime_path("portfolio/nav.sqlite"),
            Path::new("/tmp/crossline-test").join("portfolio/nav.sqlite")
        );
        assert_eq!(
            storage.resolve_runtime_path("close/close_runs.jsonl"),
            Path::new("/tmp/crossline-test").join("close/close_runs.jsonl")
        );
        assert_eq!(
            storage.resolve_runtime_path("alerts/watchlist_alerts.sqlite"),
            Path::new("/tmp/crossline-test").join("alerts/watchlist_alerts.sqlite")
        );
        assert_eq!(
            storage.resolve_runtime_path("execution/execution_runs.jsonl"),
            Path::new("/tmp/crossline-test").join("execution/execution_runs.jsonl")
        );
        assert_eq!(
            storage.resolve_runtime_path("execution/execution_ledger_events.jsonl"),
            Path::new("/tmp/crossline-test").join("execution/execution_ledger_events.jsonl")
        );
        assert_eq!(
            storage.resolve_runtime_path("execution/order_snapshots.jsonl"),
            Path::new("/tmp/crossline-test").join("execution/order_snapshots.jsonl")
        );
        assert_eq!(
            storage.resolve_runtime_path("automation/config.json"),
            Path::new("/tmp/crossline-test").join("automation/config.json")
        );
        assert_eq!(
            storage.resolve_runtime_path("webhook/outbox.sqlite"),
            Path::new("/tmp/crossline-test").join("webhook/outbox.sqlite")
        );
    }

    #[test]
    fn absolute_runtime_paths_are_preserved() {
        let storage = StorageConfig {
            data_dir: "/tmp/crossline-test".to_owned(),
            portfolio_nav_path: Some("/var/tmp/nav.sqlite".to_owned()),
            ..Default::default()
        };

        assert_eq!(
            storage.resolve_runtime_path("/var/tmp/nav.sqlite"),
            Path::new("/var/tmp/nav.sqlite")
        );
    }

    #[test]
    fn security_audit_path_uses_same_runtime_data_dir() {
        let storage = StorageConfig {
            data_dir: "/tmp/crossline-test".to_owned(),
            ..Default::default()
        };

        assert_eq!(
            normalize_runtime_path(Some("security_audit.jsonl"), &storage).as_deref(),
            Some("/tmp/crossline-test/security_audit.jsonl")
        );
    }
}
