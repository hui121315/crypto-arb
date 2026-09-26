use shared_types::{ApiProblem, VenueOperationStatus};
use std::collections::HashSet;

use super::NavHistoryResponse;

#[derive(Clone)]
pub(crate) struct HistoryNotice {
    pub(crate) title: String,
    pub(crate) message: String,
    pub(crate) tone: &'static str,
    pub(crate) facts: Vec<String>,
}

#[derive(Clone, PartialEq)]
pub(crate) struct HistoryDiagnostic {
    pub(crate) label: String,
    pub(crate) value: String,
}

pub(crate) fn history_notice(
    response: &NavHistoryResponse,
    stale_problem: Option<&ApiProblem>,
) -> Option<HistoryNotice> {
    let problem = primary_problem(response, stale_problem);
    let account_equity_missing = problem.is_some_and(is_account_equity_missing);
    let (title, message, tone) = if stale_problem.is_some() {
        (
            "净值历史暂时不可刷新",
            "保留上次读取结果，可刷新重试。",
            "warn",
        )
    } else if response
        .storage_health
        .as_ref()
        .is_some_and(|health| health.status == VenueOperationStatus::Blocked)
    {
        (
            "净值历史存储不可用",
            "历史样本当前无法可靠读写，请在技术诊断中查看存储状态。",
            "blocked",
        )
    } else if account_equity_missing {
        (
            "账户净值 暂停采样",
            "账户权益覆盖尚未完整。当前余额与持仓仍可查看，补齐账户读取后会自动恢复 账户净值 采样。",
            "warn",
        )
    } else if problem.is_some() {
        (
            "净值历史数据依据不完整",
            "部分历史数据依据尚未就绪，已保留当前可验证数据。",
            "warn",
        )
    } else {
        return None;
    };
    Some(HistoryNotice {
        title: title.to_owned(),
        message: message.to_owned(),
        tone,
        facts: history_facts(response, problem),
    })
}

pub(crate) fn history_diagnostics(
    response: &NavHistoryResponse,
    stale_problem: Option<&ApiProblem>,
) -> Vec<HistoryDiagnostic> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for problem in history_problems(response, stale_problem) {
        let key = format!(
            "{}:{}:{}",
            problem.code,
            problem.source.as_deref().unwrap_or_default(),
            problem.message
        );
        if !seen.insert(key) {
            continue;
        }
        push_diagnostic(&mut rows, "问题代码", &problem.code);
        let mut message = problem.message.clone();
        if let Some(path) = problem_detail_text(problem, "path") {
            message = message.replace(path, &compact_home_path(path));
        }
        push_diagnostic(&mut rows, "详情", &message);
        if let Some(source) = problem.source.as_deref() {
            push_diagnostic(&mut rows, "数据源", source);
        }
        if let Some(reason) = problem_detail_text(problem, "latestSampleProblem") {
            push_diagnostic(&mut rows, "采样原因", &localize_sample_problem(reason));
        }
        if let Some(path) = problem_detail_text(problem, "path") {
            push_diagnostic(&mut rows, "存储文件", &compact_home_path(path));
        }
        if let Some(request_id) = problem.request_id.as_deref() {
            push_diagnostic(&mut rows, "请求", request_id);
        }
    }
    rows
}

fn primary_problem<'a>(
    response: &'a NavHistoryResponse,
    stale_problem: Option<&'a ApiProblem>,
) -> Option<&'a ApiProblem> {
    stale_problem
        .or(response.problem.as_ref())
        .or_else(|| response.problems.first())
        .or_else(|| {
            response
                .storage_health
                .as_ref()
                .and_then(|health| health.problem.as_ref())
        })
}

fn history_problems<'a>(
    response: &'a NavHistoryResponse,
    stale_problem: Option<&'a ApiProblem>,
) -> Vec<&'a ApiProblem> {
    stale_problem
        .into_iter()
        .chain(response.problem.iter())
        .chain(response.problems.iter())
        .chain(
            response
                .storage_health
                .iter()
                .filter_map(|health| health.problem.as_ref()),
        )
        .collect()
}

fn is_account_equity_missing(problem: &ApiProblem) -> bool {
    problem_detail_text(problem, "latestSampleSource") == Some("account_equity_missing")
        || problem
            .message
            .contains("account-level equity coverage incomplete")
}

fn history_facts(response: &NavHistoryResponse, problem: Option<&ApiProblem>) -> Vec<String> {
    let mut facts = vec![format!("历史样本 {}", response.count)];
    let load_success = problem.and_then(|row| problem_detail_u64(row, "loadSuccessTotal"));
    let load_error = problem.and_then(|row| problem_detail_u64(row, "loadErrorTotal"));
    if let Some(success) = load_success {
        facts.push(format!("载入成功 {success}"));
    }
    if let Some(error) = load_error.filter(|value| *value > 0) {
        facts.push(format!("载入错误 {error}"));
    }
    facts.push(format!(
        "写入成功 {}",
        response.backend_status.append_success_total
    ));
    if response.backend_status.append_error_total > 0 {
        facts.push(format!(
            "写入错误 {}",
            response.backend_status.append_error_total
        ));
    }
    facts
}

fn problem_detail_text<'a>(problem: &'a ApiProblem, key: &str) -> Option<&'a str> {
    problem
        .details
        .as_ref()?
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn problem_detail_u64(problem: &ApiProblem, key: &str) -> Option<u64> {
    problem.details.as_ref()?.get(key)?.as_u64()
}

fn localize_sample_problem(problem: &str) -> String {
    if problem.contains("account-level equity coverage incomplete") {
        "账户级权益覆盖未完整，已跳过本轮 净值记录".to_owned()
    } else {
        problem.to_owned()
    }
}

fn compact_home_path(path: &str) -> String {
    ["/Users/", "/home/"]
        .into_iter()
        .find_map(|prefix| {
            let relative = path.strip_prefix(prefix)?;
            let home_suffix = relative.find('/').map(|index| &relative[index..])?;
            Some(format!("~{home_suffix}"))
        })
        .unwrap_or_else(|| path.to_owned())
}

fn push_diagnostic(rows: &mut Vec<HistoryDiagnostic>, label: &str, value: &str) {
    if rows
        .iter()
        .any(|row| row.label == label && row.value == value)
    {
        return;
    }
    rows.push(HistoryDiagnostic {
        label: label.to_owned(),
        value: value.to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_equity_gap_becomes_concise_notice_and_compact_diagnostics() -> Result<(), String> {
        let problem = ApiProblem {
            code: "NAV_STORAGE_UNAVAILABLE".to_owned(),
            message: "NAV sample skipped: account-level equity coverage incomplete".to_owned(),
            status: None,
            request_id: None,
            retry_after_ms: None,
            source: Some("portfolio_nav_store".to_owned()),
            recovery_action: None,
            details: Some(serde_json::json!({
                "latestSampleSource": "account_equity_missing",
                "latestSampleProblem": "account-level equity coverage incomplete; NAV sample skipped",
                "loadSuccessTotal": 1,
                "loadErrorTotal": 0,
                "path": "/Users/test/Library/Application Support/crossline-omni/portfolio_nav.sqlite"
            })),
        };
        let response = NavHistoryResponse {
            count: 0,
            rows: Vec::new(),
            page: None,
            row_cap: None,
            backend_status: shared_types::history::HistoryBackendStatus::default(),
            storage_health: None,
            source: "portfolio_nav_history".to_owned(),
            observed_at_ms: 1,
            latest_at_ms: None,
            freshness_ms: None,
            problem: Some(problem),
            retry_after_ms: None,
            problems: Vec::new(),
        };

        let notice = history_notice(&response, None)
            .ok_or_else(|| "account-equity gap should produce a NAV notice".to_owned())?;
        let diagnostics = history_diagnostics(&response, None);

        assert_eq!(notice.title, "账户净值 暂停采样");
        assert!(notice.message.contains("账户权益覆盖尚未完整"));
        assert!(notice.facts.iter().any(|fact| fact == "载入成功 1"));
        assert!(diagnostics.iter().any(|row| {
            row.label == "存储文件"
                && row.value == "~/Library/Application Support/crossline-omni/portfolio_nav.sqlite"
        }));
        assert!(!diagnostics
            .iter()
            .any(|row| row.value.starts_with("/Users/")));
        assert_eq!(
            compact_home_path(
                "/Users/test/CascadeProjects/crypto-arb/.crossline-runtime/dev/data/portfolio_nav.sqlite"
            ),
            "~/CascadeProjects/crypto-arb/.crossline-runtime/dev/data/portfolio_nav.sqlite"
        );
        assert_eq!(
            compact_home_path("/home/operator/crossline/data/portfolio_nav.sqlite"),
            "~/crossline/data/portfolio_nav.sqlite"
        );
        Ok(())
    }
}
