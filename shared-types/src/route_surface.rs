//! API gateway route surface 分类与 fail-closed 暴露策略（PR-AH）。
//!
//! 主服务的 route 被划分成 `MainTrading/Diagnostics/Legacy/Disabled` 四类。核心
//! fail-closed 契约：默认 `RouteExposurePolicy` 只放行 `MainTrading`——`Diagnostics`
//! 必须显式开启、`Legacy` 必须显式开启、`Disabled` 永远不可服务。复制 `.env`
//! 模板或忘记配置时不会无意把诊断/旧准入 route 公网裸奔；旧准入 route 默认归
//! `Legacy`/`Disabled`，不再被默认 surface 固定暴露。

use serde::{Deserialize, Serialize};

/// route 的暴露面分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteSurface {
    /// 当前产品主交易/行情 route，始终服务。
    MainTrading,
    /// 诊断/health/metrics route，仅在显式开启诊断面时服务。
    Diagnostics,
    /// 旧准入/兼容 route，仅在显式开启 legacy 面时服务。
    Legacy,
    /// 已下线 route，永不服务。
    Disabled,
}

impl RouteSurface {
    /// 是否永远被阻断（与策略无关）。
    pub fn always_blocked(self) -> bool {
        matches!(self, Self::Disabled)
    }

    /// 在给定策略下是否对外服务。
    pub fn is_served(self, policy: &RouteExposurePolicy) -> bool {
        match self {
            Self::MainTrading => true,
            Self::Diagnostics => policy.diagnostics_enabled,
            Self::Legacy => policy.legacy_enabled,
            Self::Disabled => false,
        }
    }
}

/// route 暴露策略；`Default` 为 fail-closed（仅 `MainTrading`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteExposurePolicy {
    #[serde(default)]
    pub diagnostics_enabled: bool,
    #[serde(default)]
    pub legacy_enabled: bool,
}

impl RouteExposurePolicy {
    /// 显式 fail-closed 策略：除主交易面外全部关闭。
    pub fn fail_closed() -> Self {
        Self::default()
    }

    /// 一条 route 在本策略下是否服务。
    pub fn serves(&self, surface: RouteSurface) -> bool {
        surface.is_served(self)
    }
}

/// 单条 route 的描述（路径 + 暴露面）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteDescriptor {
    pub path: String,
    pub surface: RouteSurface,
}

impl RouteDescriptor {
    /// 在给定策略下是否挂载该 route。
    pub fn is_mounted(&self, policy: &RouteExposurePolicy) -> bool {
        self.surface.is_served(policy)
    }
}

/// 对一组 route 应用 fail-closed 暴露策略，返回应挂载的 route 路径。
pub fn mounted_paths<'a>(
    routes: &'a [RouteDescriptor],
    policy: &RouteExposurePolicy,
) -> Vec<&'a str> {
    routes
        .iter()
        .filter(|r| r.is_mounted(policy))
        .map(|r| r.path.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(path: &str, surface: RouteSurface) -> RouteDescriptor {
        RouteDescriptor {
            path: path.to_owned(),
            surface,
        }
    }

    #[test]
    fn default_policy_is_fail_closed() {
        let policy = RouteExposurePolicy::default();
        assert!(!policy.diagnostics_enabled);
        assert!(!policy.legacy_enabled);
        assert_eq!(policy, RouteExposurePolicy::fail_closed());
    }

    #[test]
    fn main_trading_always_served() {
        let policy = RouteExposurePolicy::fail_closed();
        assert!(RouteSurface::MainTrading.is_served(&policy));
        assert!(!RouteSurface::MainTrading.always_blocked());
    }

    #[test]
    fn disabled_never_served_regardless_of_policy() {
        let open = RouteExposurePolicy {
            diagnostics_enabled: true,
            legacy_enabled: true,
        };
        assert!(RouteSurface::Disabled.always_blocked());
        assert!(!RouteSurface::Disabled.is_served(&open));
        assert!(!RouteSurface::Disabled.is_served(&RouteExposurePolicy::fail_closed()));
    }

    #[test]
    fn diagnostics_requires_explicit_enable() {
        assert!(!RouteSurface::Diagnostics.is_served(&RouteExposurePolicy::fail_closed()));
        let policy = RouteExposurePolicy {
            diagnostics_enabled: true,
            legacy_enabled: false,
        };
        assert!(RouteSurface::Diagnostics.is_served(&policy));
    }

    #[test]
    fn legacy_requires_explicit_enable() {
        assert!(!RouteSurface::Legacy.is_served(&RouteExposurePolicy::fail_closed()));
        let policy = RouteExposurePolicy {
            diagnostics_enabled: false,
            legacy_enabled: true,
        };
        assert!(RouteSurface::Legacy.is_served(&policy));
    }

    #[test]
    fn mounted_paths_filters_by_fail_closed_default() {
        let routes = vec![
            route("/api/opportunities", RouteSurface::MainTrading),
            route("/api/health", RouteSurface::Diagnostics),
            route("/api/legacy-diagnostic", RouteSurface::Legacy),
            route("/api/old-admin", RouteSurface::Disabled),
        ];
        let mounted = mounted_paths(&routes, &RouteExposurePolicy::fail_closed());
        assert_eq!(mounted, vec!["/api/opportunities"]);
    }

    #[test]
    fn mounted_paths_honours_enabled_surfaces() {
        let routes = vec![
            route("/api/opportunities", RouteSurface::MainTrading),
            route("/api/health", RouteSurface::Diagnostics),
            route("/api/legacy-diagnostic", RouteSurface::Legacy),
            route("/api/old-admin", RouteSurface::Disabled),
        ];
        let policy = RouteExposurePolicy {
            diagnostics_enabled: true,
            legacy_enabled: true,
        };
        let mounted = mounted_paths(&routes, &policy);
        assert_eq!(
            mounted,
            vec![
                "/api/opportunities",
                "/api/health",
                "/api/legacy-diagnostic"
            ]
        );
    }

    #[test]
    fn policy_serves_delegates_to_surface() {
        let policy = RouteExposurePolicy {
            diagnostics_enabled: true,
            legacy_enabled: false,
        };
        assert!(policy.serves(RouteSurface::Diagnostics));
        assert!(!policy.serves(RouteSurface::Legacy));
        assert!(!policy.serves(RouteSurface::Disabled));
    }
}
