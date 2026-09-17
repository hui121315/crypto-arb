use super::*;
use serde_json::json;

pub(crate) fn sessions() -> Vec<u8> {
    serde_json::to_vec(&json!([
        {"name":"US_EQUITIES_PRE_MARKET","startTime":"04:00:00","endTime":"09:30:00","timezone":"America/New_York","startWeekday":1,"endWeekday":5},
        {"name":"US_EQUITIES_REGULAR","startTime":"09:30:00","endTime":"16:00:00","timezone":"America/New_York","startWeekday":1,"endWeekday":5},
        {"name":"US_EQUITIES_POST_MARKET","startTime":"16:00:00","endTime":"20:00:00","timezone":"America/New_York","startWeekday":1,"endWeekday":5},
        {"name":"US_EQUITIES_OVERNIGHT","startTime":"20:00:00","endTime":"04:00:00","timezone":"America/New_York","startWeekday":7,"endWeekday":4}
    ])).unwrap()
}

pub(crate) fn security() -> StockSecurity {
    let mut s = super::super::comparison::tests::snapshot()
        .security
        .unwrap();
    s.sessions = [
        "US_EQUITIES_PRE_MARKET",
        "US_EQUITIES_REGULAR",
        "US_EQUITIES_POST_MARKET",
        "US_EQUITIES_OVERNIGHT",
    ]
    .map(|name| StockSession {
        name: name.into(),
        min_quantity: "0.01".into(),
        max_quantity: None,
        step_size: "0.01".into(),
    })
    .to_vec();
    s
}

fn ms(time: &str) -> i64 {
    DateTime::parse_from_rfc3339(time)
        .unwrap()
        .timestamp_millis()
}

#[test]
fn backpack_stock_calendar_handles_daily_windows_week_wrap_and_dst() {
    for (time, session) in [
        ("2026-09-13T23:59:59Z", None),
        ("2026-09-14T00:00:00Z", Some("US_EQUITIES_OVERNIGHT")),
        ("2026-09-18T07:59:59Z", Some("US_EQUITIES_OVERNIGHT")),
        ("2026-09-18T08:00:00Z", Some("US_EQUITIES_PRE_MARKET")),
        ("2026-09-19T00:00:00Z", None),
        ("2026-03-06T14:29:59Z", Some("US_EQUITIES_PRE_MARKET")),
        ("2026-03-06T14:30:00Z", Some("US_EQUITIES_REGULAR")),
        ("2026-03-09T13:30:00Z", Some("US_EQUITIES_REGULAR")),
        ("2026-11-02T14:30:00Z", Some("US_EQUITIES_REGULAR")),
    ] {
        let now = ms(time);
        let calendar = Calendar::parse(&sessions(), b"[]", now).unwrap();
        let route = calendar.route(&security(), now);
        assert_eq!(
            route.session.as_ref().map(|s| s.name.as_str()),
            session,
            "{time}: {}",
            route.reason
        );
        assert_eq!(
            route.kind,
            if session.is_some() {
                StockRouteKind::Rfq
            } else {
                StockRouteKind::OrderBook
            },
            "{time}"
        );
    }
}

#[test]
fn backpack_stock_calendar_closures_are_not_trading_windows_and_boundaries_expire() {
    let holidays=serde_json::to_vec(&json!([
        {"market":"US_EQUITIES","name":"Thanksgiving","date":"2026-11-26","timezone":"America/New_York","startTime":null,"endTime":null},
        {"market":"US_EQUITIES","name":"Early close","date":"2026-11-27","timezone":"America/New_York","startTime":"13:00:00","endTime":"23:59:59"}
    ])).unwrap();
    for (time, kind) in [
        ("2026-11-26T15:00:00Z", StockRouteKind::OrderBook),
        ("2026-11-27T17:59:59Z", StockRouteKind::Rfq),
        ("2026-11-27T18:00:00Z", StockRouteKind::OrderBook),
        ("2026-11-28T04:59:59.999Z", StockRouteKind::OrderBook),
    ] {
        let now = ms(time);
        let calendar = Calendar::parse(&sessions(), &holidays, now).unwrap();
        assert_eq!(calendar.route(&security(), now).kind, kind, "{time}");
    }
    let before = ms("2026-11-27T17:59:59Z");
    let calendar = Calendar::parse(&sessions(), &holidays, before).unwrap();
    assert_eq!(
        calendar.route(&security(), before).valid_until_ms,
        before + 1000
    );
    assert_eq!(
        calendar.route(&security(), before + TTL_MS).kind,
        StockRouteKind::Unknown
    );
    let mut rfq_only = security();
    rfq_only.order_books.clear();
    assert_eq!(
        calendar.route(&rfq_only, before + 1000).kind,
        StockRouteKind::Closed
    );
}

#[test]
fn backpack_stock_calendar_rejects_missing_sessions_invalid_timezone_and_ambiguous_local_time() {
    let now = ms("2026-09-14T12:00:00Z");
    let calendar = Calendar::parse(&sessions(), b"[]", now).unwrap();
    let mut security = security();
    security.sessions[0].name = "UNVERIFIED_MARKET".into();
    assert_eq!(calendar.route(&security, now).kind, StockRouteKind::Unknown);
    let mut raw: serde_json::Value = serde_json::from_slice(&sessions()).unwrap();
    raw[0]["timezone"] = "Unknown/Zone".into();
    assert!(Calendar::parse(&serde_json::to_vec(&raw).unwrap(), b"[]", now).is_err());
    assert!(Calendar::parse(&sessions(),br#"[{"market":"US_EQUITIES","name":"bad","date":"2026-01-01","timezone":"UTC","startTime":"12:00:00"}]"#,now).is_err());
    assert!(local_ms(
        chrono_tz::America::New_York,
        NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
        NaiveTime::from_hms_opt(1, 30, 0).unwrap()
    )
    .is_err());
    assert!(local_ms(
        chrono_tz::America::New_York,
        NaiveDate::from_ymd_opt(2026, 3, 8).unwrap(),
        NaiveTime::from_hms_opt(2, 30, 0).unwrap()
    )
    .is_err());
}
