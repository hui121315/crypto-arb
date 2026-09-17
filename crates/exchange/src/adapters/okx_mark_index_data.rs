use crate::adapter::strip_common_suffixes;
use common::time::now_ms;
use serde::Deserialize;
use shared_types::MarkIndexInfo;
use std::collections::{HashMap, HashSet};

const EXCHANGE: &str = "okx";
const SWAP_SUFFIX: &str = "-SWAP";

#[derive(Debug, Clone, Deserialize)]
pub(super) struct OkxMarkPriceItem {
    #[serde(rename = "instId")]
    pub(super) inst_id: String,
    #[serde(default, rename = "markPx")]
    pub(super) mark_px: String,
    #[serde(default)]
    pub(super) ts: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct OkxIndexTickerItem {
    #[serde(rename = "instId")]
    pub(super) inst_id: String,
    #[serde(default, rename = "idxPx")]
    pub(super) idx_px: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct OkxOpenInterestItem {
    #[serde(rename = "instId")]
    pub(super) inst_id: String,
    #[serde(default)]
    pub(super) oi: String,
    #[serde(default, rename = "oiUsd")]
    pub(super) oi_usd: String,
}

pub(super) fn parse_mark_index_rows(
    marks: Vec<OkxMarkPriceItem>,
    indexes: Vec<OkxIndexTickerItem>,
    open_interest: Vec<OkxOpenInterestItem>,
    requested: Option<&HashSet<String>>,
) -> Vec<MarkIndexInfo> {
    let indexes = index_map(indexes);
    let open_interest = open_interest_map(open_interest);
    marks
        .into_iter()
        .filter(|mark| match requested {
            Some(ids) => ids.contains(&mark.inst_id),
            None => true,
        })
        .filter_map(|mark| {
            let index_id = index_id_for_swap(&mark.inst_id);
            parse_mark_index(
                &mark.inst_id,
                &mark.mark_px,
                indexes.get(&index_id).map(String::as_str),
                open_interest.get(&mark.inst_id).map(|row| row.0.as_str()),
                open_interest.get(&mark.inst_id).map(|row| row.1.as_str()),
                parse_millis(&mark.ts),
            )
        })
        .collect()
}

pub(super) fn parse_mark_index(
    inst_id: &str,
    mark_px: &str,
    index_px: Option<&str>,
    oi: Option<&str>,
    oi_usd: Option<&str>,
    timestamp: i64,
) -> Option<MarkIndexInfo> {
    Some(MarkIndexInfo {
        symbol: strip_common_suffixes(inst_id),
        exchange: EXCHANGE.into(),
        mark_price: parse_positive(mark_px)?,
        index_price: index_px.and_then(parse_positive),
        open_interest: oi.and_then(parse_positive),
        open_interest_value: oi_usd.and_then(parse_positive),
        timestamp: if timestamp == 0 { now_ms() } else { timestamp },
    })
}

pub(super) fn index_id_for_swap(inst_id: &str) -> String {
    inst_id
        .strip_suffix(SWAP_SUFFIX)
        .unwrap_or(inst_id)
        .to_owned()
}

pub(super) fn swap_id_for_index(inst_id: &str) -> String {
    if inst_id.ends_with(SWAP_SUFFIX) {
        inst_id.to_owned()
    } else {
        format!("{inst_id}{SWAP_SUFFIX}")
    }
}

fn index_map(rows: Vec<OkxIndexTickerItem>) -> HashMap<String, String> {
    rows.into_iter()
        .map(|row| (row.inst_id, row.idx_px))
        .collect()
}

fn open_interest_map(rows: Vec<OkxOpenInterestItem>) -> HashMap<String, (String, String)> {
    rows.into_iter()
        .map(|row| (row.inst_id, (row.oi, row.oi_usd)))
        .collect()
}

pub(super) fn parse_millis(raw: &str) -> i64 {
    raw.parse().unwrap_or(0)
}

fn parse_positive(raw: &str) -> Option<f64> {
    raw.parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > f64::EPSILON)
}

#[cfg(test)]
#[path = "okx_mark_index_data_tests.rs"]
mod tests;
