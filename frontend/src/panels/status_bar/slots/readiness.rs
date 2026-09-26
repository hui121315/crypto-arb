use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::panels::status_bar) enum Readiness {
    Ready,
    NotRequired,
    Disabled,
    Setup,
    Unknown,
    Stale,
    Unsupported,
    Warning,
    Blocked,
    Error,
}

impl Readiness {
    pub(in crate::panels::status_bar) fn needs_attention(self) -> bool {
        !matches!(self, Self::Ready | Self::NotRequired)
    }

    pub(in crate::panels::status_bar) fn state(self) -> &'static str {
        match self {
            Self::Ready | Self::NotRequired => "healthy",
            Self::Disabled | Self::Setup | Self::Unknown | Self::Stale | Self::Unsupported => {
                "unknown"
            }
            Self::Warning => "warning",
            Self::Blocked | Self::Error => "degraded",
        }
    }

    pub(super) fn slot_class(self) -> &'static str {
        match self.state() {
            "healthy" => "slot",
            "unknown" => "slot unknown",
            "warning" => "slot warning",
            _ => "slot degraded",
        }
    }

    pub(super) fn dot_class(self) -> &'static str {
        match self {
            Self::Ready => "slot-dot ok",
            Self::Warning => "slot-dot warning",
            Self::Blocked | Self::Error => "slot-dot red",
            _ => "slot-dot neutral",
        }
    }

    pub(in crate::panels::status_bar) fn label(self) -> &'static str {
        match self {
            Self::Ready => "正常",
            Self::NotRequired => "模拟无需",
            Self::Disabled => "未启用",
            Self::Setup => "需配置凭证",
            Self::Unknown => "等待确认",
            Self::Stale => "已过期",
            Self::Unsupported => "不支持",
            Self::Warning => "降级",
            Self::Blocked => "受限",
            Self::Error => "读取失败",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::panels::status_bar) enum RuntimeCategory {
    Market,
    Api,
    PrivateWs,
    Background,
    Snapshot,
    AppWs,
}

impl RuntimeCategory {
    pub(in crate::panels::status_bar) fn label(self) -> &'static str {
        match self {
            Self::Market => "行情",
            Self::Api => "交易接口",
            Self::PrivateWs => "私有账户流",
            Self::Background => "后台任务",
            Self::Snapshot => "运行状态",
            Self::AppWs => "应用连接",
        }
    }

    fn rows(self, snapshot: &VenueOperationHealthSnapshot) -> Vec<&VenueOperationHealth> {
        match self {
            Self::Market => market_data_rows(snapshot).collect(),
            Self::Api => snapshot
                .rows
                .iter()
                .filter(|row| is_api_operation_row(row))
                .collect(),
            Self::PrivateWs => snapshot
                .rows
                .iter()
                .filter(|row| is_private_ws_row(row))
                .collect(),
            Self::Background => snapshot
                .rows
                .iter()
                .filter(|row| {
                    row.source == "task_registry"
                        && row.operation.starts_with("background_task:")
                        && row.configured == Some(true)
                })
                .collect(),
            Self::Snapshot | Self::AppWs => Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::panels::status_bar) struct CategoryReadiness {
    pub category: RuntimeCategory,
    pub readiness: Readiness,
    pub detail: String,
}

pub(in crate::panels::status_bar) fn category_readiness(
    category: RuntimeCategory,
    snapshot: Option<&VenueOperationHealthSnapshot>,
    problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> CategoryReadiness {
    let result = |readiness, detail: String| CategoryReadiness {
        category,
        readiness,
        detail,
    };
    if let Some(problem) = problem {
        return result(
            problem_readiness(problem),
            format!("{} · {}", problem.code, problem.message),
        );
    }
    let Some(snapshot) = snapshot else {
        return result(Readiness::Unknown, "等待当前连接的后台健康快照".into());
    };
    let rows = category.rows(snapshot);
    if rows.is_empty() {
        return if category == RuntimeCategory::Background {
            result(Readiness::Ready, String::new())
        } else {
            result(
                Readiness::Unknown,
                "后台快照未提供这类接口的运行样本".into(),
            )
        };
    }
    let mut active = rows
        .iter()
        .copied()
        .filter(|row| row.configured != Some(false))
        .collect::<Vec<_>>();
    if active.is_empty() {
        return if category == RuntimeCategory::Market {
            result(
                Readiness::Disabled,
                "行情订阅未启用，不代表交易所连接失败".into(),
            )
        } else if environment == Some(ExecutionEnvironment::Paper) {
            result(
                Readiness::NotRequired,
                "模拟模式未接入私有账户；不代表实盘接口可用".into(),
            )
        } else if environment.is_none() {
            result(
                Readiness::Unknown,
                "执行环境尚未确认；私有凭证未配置".into(),
            )
        } else {
            result(
                Readiness::Setup,
                "请在设置的 API 凭证中配置需要使用的交易所".into(),
            )
        };
    }
    if category == RuntimeCategory::Api {
        active.extend(snapshot.rows.iter().filter(|row| {
            is_authenticated_api_transport_row(row) && row.configured != Some(false)
        }));
    }
    let classify = |row: &VenueOperationHealth| match row.status {
        VenueOperationStatus::Blocked => Readiness::Blocked,
        VenueOperationStatus::Warn => Readiness::Warning,
        VenueOperationStatus::Unknown => Readiness::Unknown,
        VenueOperationStatus::Unsupported => Readiness::Unsupported,
        VenueOperationStatus::Ok
            if row.configured.is_none()
                && matches!(category, RuntimeCategory::Api | RuntimeCategory::PrivateWs) =>
        {
            Readiness::Unknown
        }
        VenueOperationStatus::Ok => Readiness::Ready,
    };
    let row = active.into_iter().max_by_key(|row| classify(row));
    match row {
        Some(row) => {
            let readiness = classify(row);
            let reason =
                if readiness == Readiness::Unknown && row.status == VenueOperationStatus::Ok {
                    "配置状态尚未确认"
                } else {
                    row.message.as_str()
                };
            result(
                readiness,
                format!("{} · {} · {reason}", row.venue, row.operation),
            )
        }
        None => result(Readiness::Unknown, "等待运行样本".into()),
    }
}

pub(in crate::panels::status_bar) fn operation_readiness(
    state: &LoadState<VenueOperationHealthSnapshot>,
    environment: Option<ExecutionEnvironment>,
) -> Vec<CategoryReadiness> {
    if !matches!(state, LoadState::Ready(_)) {
        return vec![CategoryReadiness {
            category: RuntimeCategory::Snapshot,
            readiness: state
                .problem()
                .map_or(Readiness::Unknown, problem_readiness),
            detail: state.problem().map_or_else(
                || "等待当前连接的后台健康快照".into(),
                |problem| format!("{} · {}", problem.code, problem.message),
            ),
        }];
    }
    [
        RuntimeCategory::Market,
        RuntimeCategory::Api,
        RuntimeCategory::PrivateWs,
        RuntimeCategory::Background,
    ]
    .into_iter()
    .map(|category| category_readiness(category, state.value(), state.problem(), environment))
    .collect()
}

pub(super) fn problem_readiness(problem: &ApiProblem) -> Readiness {
    match problem.code.as_str() {
        "OPERATION_HEALTH_STALE" | "SYSTEM_HEALTH_STALE" | "TRADING_STATUS_STALE" => {
            Readiness::Stale
        }
        "OPERATION_HEALTH_UNCONFIRMED" | "SYSTEM_HEALTH_UNCONFIRMED" => Readiness::Unknown,
        _ => Readiness::Error,
    }
}

pub(super) fn operation_problem_label(problem: &ApiProblem) -> &'static str {
    if problem_readiness(problem).state() == "unknown" {
        "待确认"
    } else {
        "异常"
    }
}

pub(in crate::panels::status_bar) fn app_connection_readiness(
    channel: &crate::api::ws::WsChannelState,
    snapshot: Option<&VenueOperationHealthSnapshot>,
) -> CategoryReadiness {
    use crate::api::ws::WsStatus;
    let readiness = if channel.last_error.is_some() {
        Readiness::Error
    } else if app_ws_lag_summary(snapshot).recent_channels > 0 {
        Readiness::Warning
    } else if channel.status == WsStatus::Disconnected {
        Readiness::Warning
    } else if channel.status != WsStatus::Connected || !channel.subscribed {
        Readiness::Unknown
    } else {
        Readiness::Ready
    };
    CategoryReadiness {
        category: RuntimeCategory::AppWs,
        readiness,
        detail: channel.last_error.as_ref().map_or_else(
            || app_ws_label(channel, snapshot),
            |problem| format!("{} · {}", problem.code, problem.message),
        ),
    }
}
