use chrono::{DateTime, Datelike, Days, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Deserialize;
use shared_types::stocks::*;
use std::collections::BTreeSet;

pub(super) const TTL_MS: i64 = 300_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Schedule {
    name: String,
    start_time: NaiveTime,
    end_time: NaiveTime,
    timezone: String,
    start_weekday: u32,
    end_weekday: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Holiday {
    market: String,
    name: String,
    date: NaiveDate,
    start_time: Option<NaiveTime>,
    end_time: Option<NaiveTime>,
    timezone: String,
}

#[derive(Debug, Clone)]
pub(super) struct Calendar {
    schedules: Vec<Schedule>,
    holidays: Vec<Holiday>,
    pub(super) fetched_at_ms: i64,
}

fn known_group(name: &str) -> Option<&'static str> {
    match name {
        "US_EQUITIES_PRE_MARKET"
        | "US_EQUITIES_REGULAR"
        | "US_EQUITIES_POST_MARKET"
        | "US_EQUITIES_OVERNIGHT" => Some("US_EQUITIES"),
        _ => None,
    }
}

impl Calendar {
    pub(super) fn parse(sessions: &[u8], holidays: &[u8], now: i64) -> Result<Self, String> {
        let schedules: Vec<Schedule> =
            serde_json::from_slice(sessions).map_err(|_| "股票时段响应无效")?;
        let holidays: Vec<Holiday> =
            serde_json::from_slice(holidays).map_err(|_| "股票假日响应无效")?;
        if schedules.is_empty() || schedules.len() > 128 || holidays.len() > 4096 {
            return Err("交易日历条数无效".into());
        }
        let mut names = BTreeSet::new();
        for s in &schedules {
            if !names.insert(&s.name)
                || !(1..=7).contains(&s.start_weekday)
                || !(1..=7).contains(&s.end_weekday)
                || s.timezone.parse::<Tz>().is_err()
                || s.start_time == s.end_time
            {
                return Err("股票时段重复、星期/时区无效或起止时间相同".into());
            }
        }
        for h in &holidays {
            if h.timezone.parse::<Tz>().is_err() || h.start_time.is_some() != h.end_time.is_some() {
                return Err("休市日历的时区或闭市区间不完整".into());
            }
        }
        Ok(Self {
            schedules,
            holidays,
            fetched_at_ms: now,
        })
    }

    pub(super) fn current(&self, now: i64) -> bool {
        now >= self.fetched_at_ms && now.saturating_sub(self.fetched_at_ms) < TTL_MS
    }

    pub(super) fn route(&self, security: &StockSecurity, now: i64) -> StockTradingRoute {
        match self.resolve(security, now) {
            Ok(route) => route,
            Err(reason) => unknown(reason, Some(self.fetched_at_ms)),
        }
    }

    fn resolve(&self, security: &StockSecurity, now: i64) -> Result<StockTradingRoute, String> {
        if !self.current(now) {
            return Err("官方交易日历已陈旧，等待更新".into());
        }
        if security.sessions.is_empty() {
            return Err("该证券未提供交易时段".into());
        }
        let instant = DateTime::from_timestamp_millis(now).ok_or("当前时间无效")?;
        let mut boundary = self.fetched_at_ms.saturating_add(TTL_MS);
        let mut active = Vec::new();
        let mut holiday_name = None;
        let mut timezone = None;
        for constraints in &security.sessions {
            let schedule = self
                .schedules
                .iter()
                .find(|s| s.name == constraints.name)
                .ok_or("证券时段在官方日历中缺失")?;
            let group = known_group(&schedule.name).ok_or("新时段的休市市场映射尚未核实")?;
            let tz: Tz = schedule.timezone.parse().map_err(|_| "官方时区无法识别")?;
            timezone = Some(schedule.timezone.clone());
            let today = instant.with_timezone(&tz).date_naive();
            let mut closed = false;
            for holiday in self.holidays.iter().filter(|h| h.market == group) {
                let (start, end) = holiday.interval()?;
                next_boundary(&mut boundary, now, start, end);
                if start <= now && now < end {
                    closed = true;
                    holiday_name = Some(holiday.name.clone());
                }
            }
            for offset in -1_i64..=8 {
                let date = today
                    .checked_add_signed(chrono::Duration::days(offset))
                    .ok_or("交易日期超出范围")?;
                let weekday = date.weekday().number_from_monday();
                // Backpack uses inclusive opening weekdays, including wrapped Sun..Thu overnight sessions.
                if (weekday + 7 - schedule.start_weekday) % 7
                    > (schedule.end_weekday + 7 - schedule.start_weekday) % 7
                {
                    continue;
                }
                let end_date = if schedule.end_time < schedule.start_time {
                    date.succ_opt().ok_or("跨日时段无效")?
                } else {
                    date
                };
                let start = local_ms(tz, date, schedule.start_time)?;
                let end = local_ms(tz, end_date, schedule.end_time)?;
                next_boundary(&mut boundary, now, start, end);
                if start <= now && now < end && !closed {
                    active.push(constraints.clone());
                }
            }
        }
        if active.len() > 1 {
            return Err("官方证券时段重叠，暂不选择交易通道".into());
        }
        let (kind, session, symbol, reason) = if let Some(session) = active.pop() {
            (
                StockRouteKind::Rfq,
                Some(session),
                Some(security.rfq_symbol.clone()),
                "当前官方交易时段使用 RFQ；现货盘口不代表此时可成交价格".into(),
            )
        } else if let Some(book) = security
            .order_books
            .iter()
            .find(|m| m.quote == "USDC" && m.state == "Open")
        {
            (
                StockRouteKind::OrderBook,
                None,
                Some(book.symbol.clone()),
                holiday_name
                    .map(|name| format!("{name} · 时段外使用现货订单簿"))
                    .unwrap_or_else(|| "官方时段外 · 使用现货订单簿".into()),
            )
        } else {
            (
                StockRouteKind::Closed,
                None,
                None,
                holiday_name
                    .map(|name| format!("{name} · 无开放股票订单簿"))
                    .unwrap_or_else(|| "官方交易时段已结束，且没有开放的股票订单簿".into()),
            )
        };
        Ok(StockTradingRoute {
            kind,
            session,
            symbol,
            reason,
            timezone,
            calendar_at_ms: Some(self.fetched_at_ms),
            valid_until_ms: boundary,
        })
    }
}

fn next_boundary(next: &mut i64, now: i64, start: i64, end: i64) {
    for value in [start, end] {
        if value > now {
            *next = (*next).min(value);
        }
    }
}

fn local_ms(tz: Tz, date: NaiveDate, time: NaiveTime) -> Result<i64, String> {
    tz.from_local_datetime(&date.and_time(time))
        .single()
        .map(|d| d.with_timezone(&Utc).timestamp_millis())
        .ok_or_else(|| "交易时段落在夏令时重复或不存在的本地时间，需重新核实".into())
}

impl Holiday {
    fn interval(&self) -> Result<(i64, i64), String> {
        let tz: Tz = self.timezone.parse().map_err(|_| "休市时区无效")?;
        let midnight = NaiveTime::from_hms_opt(0, 0, 0).unwrap();
        match (self.start_time, self.end_time) {
            (None, None) => Ok((
                local_ms(tz, self.date, midnight)?,
                local_ms(
                    tz,
                    self.date
                        .checked_add_days(Days::new(1))
                        .ok_or("休市日期无效")?,
                    midnight,
                )?,
            )),
            (Some(start), Some(end)) => {
                let day_end = NaiveTime::from_hms_opt(23, 59, 59).unwrap();
                let (date, end) = if end == day_end || end < start {
                    (
                        self.date.succ_opt().ok_or("跨日休市无效")?,
                        if end == day_end { midnight } else { end },
                    )
                } else {
                    (self.date, end)
                };
                if start == end && date == self.date {
                    return Err("休市起止时间相同".into());
                }
                Ok((local_ms(tz, self.date, start)?, local_ms(tz, date, end)?))
            }
            _ => Err("休市区间不完整".into()),
        }
    }
}

pub(super) fn unknown(reason: impl Into<String>, at: Option<i64>) -> StockTradingRoute {
    StockTradingRoute {
        kind: StockRouteKind::Unknown,
        session: None,
        symbol: None,
        reason: reason.into(),
        timezone: None,
        calendar_at_ms: at,
        valid_until_ms: 0,
    }
}

#[cfg(test)]
pub(super) mod tests;
