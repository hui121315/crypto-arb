use super::{CachedBook, CHANNEL, EXCHANGE, MAX_CACHED_DEPTH};
use common::time::now_ms;
use serde::Deserialize;
use serde_json::json;
use shared_types::OrderBookInfo;

pub(super) fn subscription_payload(op: &str, symbol: &str) -> String {
    json!({
        "op": op,
        "args": [{"channel": CHANNEL, "instId": symbol}]
    })
    .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BookAction {
    Snapshot,
    Update,
}

#[derive(Debug)]
pub(super) struct ParsedUpdate {
    pub(super) action: BookAction,
    pub(super) symbol: String,
    pub(super) bids: Vec<[f64; 2]>,
    pub(super) asks: Vec<[f64; 2]>,
    pub(super) timestamp: i64,
    pub(super) seq_id: i64,
    pub(super) prev_seq_id: i64,
}

pub(super) fn parse_update(text: &str) -> Option<ParsedUpdate> {
    let envelope: BookEnvelope = serde_json::from_str(text).ok()?;
    if envelope.arg.channel != CHANNEL {
        return None;
    }
    let action = match envelope.action.as_str() {
        "snapshot" => BookAction::Snapshot,
        "update" => BookAction::Update,
        _ => return None,
    };
    let item = envelope.data.into_iter().next()?;
    Some(ParsedUpdate {
        action,
        symbol: envelope.arg.inst_id,
        bids: parse_levels(&item.bids),
        asks: parse_levels(&item.asks),
        timestamp: item.ts.parse().unwrap_or_else(|_| now_ms()),
        seq_id: item.seq_id,
        prev_seq_id: item.prev_seq_id,
    })
}

pub(super) fn snapshot_book(update: &ParsedUpdate) -> Option<CachedBook> {
    let mut book = OrderBookInfo {
        symbol: crate::spot::native_pair_symbol(&update.symbol, '/')?,
        exchange: EXCHANGE.into(),
        bids: positive_levels(&update.bids, true),
        asks: positive_levels(&update.asks, false),
        timestamp: update.timestamp,
    };
    trim_book(&mut book);
    book_is_complete(&book).then_some(CachedBook {
        book,
        observed_at_ms: now_ms(),
        last_seq_id: update.seq_id,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MergeOutcome {
    Applied,
    Stale,
    Gap,
}

pub(super) fn apply_incremental(cached: &mut CachedBook, update: &ParsedUpdate) -> MergeOutcome {
    if update.seq_id <= cached.last_seq_id {
        return MergeOutcome::Stale;
    }
    if update.prev_seq_id != cached.last_seq_id {
        return MergeOutcome::Gap;
    }
    merge_side(&mut cached.book.bids, &update.bids, true);
    merge_side(&mut cached.book.asks, &update.asks, false);
    if !book_is_complete(&cached.book) {
        return MergeOutcome::Gap;
    }
    cached.book.timestamp = update.timestamp;
    cached.observed_at_ms = now_ms();
    cached.last_seq_id = update.seq_id;
    MergeOutcome::Applied
}

fn parse_levels(levels: &[Vec<String>]) -> Vec<[f64; 2]> {
    levels
        .iter()
        .filter_map(|level| Some([level.first()?.parse().ok()?, level.get(1)?.parse().ok()?]))
        .collect()
}

fn positive_levels(levels: &[[f64; 2]], descending: bool) -> Vec<[f64; 2]> {
    let mut rows = levels
        .iter()
        .copied()
        .filter(|[price, size]| *price > 0.0 && *size > 0.0)
        .collect::<Vec<_>>();
    sort_side(&mut rows, descending);
    rows
}

fn merge_side(side: &mut Vec<[f64; 2]>, updates: &[[f64; 2]], descending: bool) {
    for &[price, size] in updates {
        if let Some(index) = side.iter().position(|level| level[0] == price) {
            if size > 0.0 {
                side[index][1] = size;
            } else {
                side.remove(index);
            }
        } else if price > 0.0 && size > 0.0 {
            side.push([price, size]);
        }
    }
    sort_side(side, descending);
    side.truncate(MAX_CACHED_DEPTH);
}

fn sort_side(side: &mut [[f64; 2]], descending: bool) {
    side.sort_by(|left, right| {
        if descending {
            right[0].total_cmp(&left[0])
        } else {
            left[0].total_cmp(&right[0])
        }
    });
}

fn trim_book(book: &mut OrderBookInfo) {
    book.bids.truncate(MAX_CACHED_DEPTH);
    book.asks.truncate(MAX_CACHED_DEPTH);
}

fn book_is_complete(book: &OrderBookInfo) -> bool {
    matches!((book.best_bid(), book.best_ask()), (Some(bid), Some(ask)) if bid < ask)
}

#[derive(Debug, Deserialize)]
struct BookEnvelope {
    arg: BookArg,
    #[serde(default)]
    action: String,
    #[serde(default)]
    data: Vec<BookData>,
}

#[derive(Debug, Deserialize)]
struct BookArg {
    channel: String,
    #[serde(rename = "instId")]
    inst_id: String,
}

#[derive(Debug, Deserialize)]
struct BookData {
    #[serde(default)]
    bids: Vec<Vec<String>>,
    #[serde(default)]
    asks: Vec<Vec<String>>,
    #[serde(default)]
    ts: String,
    #[serde(default, rename = "seqId")]
    seq_id: i64,
    #[serde(default, rename = "prevSeqId")]
    prev_seq_id: i64,
}
