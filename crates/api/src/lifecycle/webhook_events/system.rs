use super::{emit_value, Cursor};
use crate::state::AppState;
use shared_types::{RiskStatusSlot, SystemHealth, WebhookEventKind};

mod message;

use message::system_payload;

const SAME_SEVERITY_ALERT_COOLDOWN_MS: i64 = 15 * 60 * 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SystemAlertFingerprint {
    risk: RiskStatusSlot,
    degraded: bool,
    problems: Vec<(String, String, String, Option<String>)>,
}

pub(super) fn fingerprint(health: &SystemHealth) -> SystemAlertFingerprint {
    let mut problems = health
        .problems
        .iter()
        .map(|problem| {
            (
                problem.scope.clone(),
                problem.operation.clone(),
                problem.code.clone(),
                problem.venue.clone(),
            )
        })
        .collect::<Vec<_>>();
    problems.sort();
    problems.dedup();
    SystemAlertFingerprint {
        risk: health.risk,
        degraded: health.degraded,
        problems,
    }
}

pub(super) const fn initial_notify_after_ms(health: &SystemHealth) -> i64 {
    health
        .updated_at_ms
        .saturating_add(SAME_SEVERITY_ALERT_COOLDOWN_MS)
}

pub(super) async fn emit_system_event(state: &AppState, cursor: &mut Cursor) {
    let Some(health) = state.system_health_snapshot().value_now() else {
        return;
    };
    let next = fingerprint(&health);
    let Some(previous) = cursor.system_fingerprint.as_ref() else {
        cursor.system_fingerprint = Some(next);
        cursor.system_notify_after_ms = initial_notify_after_ms(&health);
        cursor.system_notified_risk = Some(health.risk);
        cursor.system_notified_degraded = health.degraded;
        return;
    };
    if previous == &next {
        return;
    }
    if !is_alerting(&next) {
        cursor.system_fingerprint = Some(next);
        cursor.system_notify_after_ms = 0;
        cursor.system_notified_risk = Some(RiskStatusSlot::Ok);
        cursor.system_notified_degraded = false;
        return;
    }
    let severity_escalated = is_unnotified_escalation(
        cursor.system_notified_risk,
        cursor.system_notified_degraded,
        &next,
    );
    if health.updated_at_ms < cursor.system_notify_after_ms && !severity_escalated {
        cursor.system_fingerprint = Some(next);
        return;
    }
    let kind = alert_kind(severity_escalated);
    let payload = system_payload(&health, kind);
    if emit_value(
        state,
        kind,
        format!("{}-{}", event_key(kind), health.updated_at_ms),
        &payload,
    )
    .await
    {
        record_notified_severity(cursor, &next);
        cursor.system_fingerprint = Some(next);
        cursor.system_notify_after_ms = initial_notify_after_ms(&health);
    }
}

fn is_alerting(fingerprint: &SystemAlertFingerprint) -> bool {
    fingerprint.risk != RiskStatusSlot::Ok || fingerprint.degraded
}

const fn alert_kind(severity_escalated: bool) -> WebhookEventKind {
    if severity_escalated {
        WebhookEventKind::RiskAlert
    } else {
        WebhookEventKind::SystemDegradation
    }
}

fn is_unnotified_escalation(
    notified_risk: Option<RiskStatusSlot>,
    notified_degraded: bool,
    next: &SystemAlertFingerprint,
) -> bool {
    let notified_risk = notified_risk.unwrap_or(RiskStatusSlot::Ok);
    risk_rank(next.risk) > risk_rank(notified_risk) || (!notified_degraded && next.degraded)
}

fn record_notified_severity(cursor: &mut Cursor, next: &SystemAlertFingerprint) {
    let notified_risk = cursor.system_notified_risk.unwrap_or(RiskStatusSlot::Ok);
    if risk_rank(next.risk) > risk_rank(notified_risk) {
        cursor.system_notified_risk = Some(next.risk);
    }
    cursor.system_notified_degraded |= next.degraded;
}

const fn risk_rank(risk: RiskStatusSlot) -> u8 {
    match risk {
        RiskStatusSlot::Ok => 0,
        RiskStatusSlot::Warn => 1,
        RiskStatusSlot::Block => 2,
    }
}

const fn event_key(kind: WebhookEventKind) -> &'static str {
    match kind {
        WebhookEventKind::RiskAlert => "risk",
        WebhookEventKind::SystemDegradation => "degradation",
        _ => "system",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ApiHealthSlot, RuntimeProblem, WsHealthSlot};

    #[test]
    fn fingerprint_ignores_heartbeat_fields_and_problem_order() {
        let mut first = health(RiskStatusSlot::Warn, true, 1_000);
        first
            .problems
            .push(problem("account", "positions", "TIMEOUT", "gate", 1_000));
        let mut second = first.clone();
        second.updated_at_ms = 9_000;
        second.net_delta_usd = 42.0;
        second.order_elapsed_ms = Some(999);
        second.problems.reverse();
        for problem in &mut second.problems {
            problem.observed_at_ms = 9_000;
            problem.message = "same typed issue with refreshed timing".to_owned();
        }

        assert_eq!(fingerprint(&first), fingerprint(&second));
    }

    #[test]
    fn only_unnotified_top_level_severity_bypasses_cooldown() {
        let warn = fingerprint(&health(RiskStatusSlot::Warn, true, 2));
        let blocked = fingerprint(&health(RiskStatusSlot::Block, true, 4));

        assert!(is_unnotified_escalation(
            Some(RiskStatusSlot::Ok),
            false,
            &warn
        ));
        assert!(!is_unnotified_escalation(
            Some(RiskStatusSlot::Warn),
            true,
            &warn
        ));
        assert!(is_unnotified_escalation(
            Some(RiskStatusSlot::Warn),
            true,
            &blocked
        ));
    }

    #[test]
    fn one_snapshot_selects_one_highest_priority_event_kind() {
        assert_eq!(alert_kind(true), WebhookEventKind::RiskAlert);
        assert_eq!(alert_kind(false), WebhookEventKind::SystemDegradation);
    }

    #[test]
    fn degraded_unknown_risk_does_not_rearm_warn_alert() {
        let mut cursor = Cursor {
            system_notified_risk: Some(RiskStatusSlot::Warn),
            system_notified_degraded: true,
            ..Cursor::default()
        };
        let degraded_unknown = fingerprint(&health(RiskStatusSlot::Ok, true, 2));
        record_notified_severity(&mut cursor, &degraded_unknown);

        let recovered_sample = fingerprint(&health(RiskStatusSlot::Warn, true, 3));
        assert!(!is_unnotified_escalation(
            cursor.system_notified_risk,
            cursor.system_notified_degraded,
            &recovered_sample
        ));
    }

    #[test]
    fn same_severity_details_are_bounded_to_fifteen_minutes() {
        assert_eq!(
            initial_notify_after_ms(&health(RiskStatusSlot::Warn, true, 1_000)),
            901_000
        );
    }

    #[test]
    fn system_payload_adds_readable_bark_message_and_keeps_evidence() {
        let mut health = health(RiskStatusSlot::Warn, true, 1_000);
        health.net_delta_usd = 14_575.786_870_087_36;
        health.net_delta_pct_of_nav = 228.825_442_446_703_05;
        health.next_funding = Some(shared_types::NextFundingSlot {
            symbol: "BTC".to_owned(),
            venue: "binance".to_owned(),
            minutes_to_settle: 138,
            estimated_outflow_usd: 1.457_578_687,
        });

        let payload = system_payload(&health, WebhookEventKind::RiskAlert);
        let message = payload["message"].as_str().unwrap_or_default();

        assert!(message.starts_with("风险 WARN\n净 Delta +$14575.79（+228.8% NAV）"));
        assert!(message.contains("资金费率 BTC@binance · 138m · 预估流出 $1.46"));
        assert!(message.contains("运行健康 API 1/1 · WS 1/1"));
        assert!(message.contains("问题 1 项 · TIMEOUT x1"));
        assert_eq!(payload["risk"], "warn");
        assert_eq!(payload["problems"][0]["code"], "TIMEOUT");
    }

    fn health(risk: RiskStatusSlot, degraded: bool, updated_at_ms: i64) -> SystemHealth {
        SystemHealth {
            api_version: "test".to_owned(),
            api: ApiHealthSlot {
                healthy: 1,
                total: 1,
                failed_venues: Vec::new(),
            },
            ws: WsHealthSlot {
                channels: 1,
                disconnected: Vec::new(),
            },
            order_elapsed_ms: Some(12),
            risk,
            net_delta_usd: 0.0,
            net_delta_pct_of_nav: 0.0,
            next_funding: None,
            updated_at_ms,
            degraded,
            problems: vec![problem("market", "ticker", "TIMEOUT", "okx", updated_at_ms)],
        }
    }

    fn problem(
        scope: &str,
        operation: &str,
        code: &str,
        venue: &str,
        observed_at_ms: i64,
    ) -> RuntimeProblem {
        RuntimeProblem {
            scope: scope.to_owned(),
            operation: operation.to_owned(),
            code: code.to_owned(),
            message: "timeout".to_owned(),
            venue: Some(venue.to_owned()),
            retry_after_ms: None,
            problem: None,
            observed_at_ms,
        }
    }
}
