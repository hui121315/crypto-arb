//! Minimal Gate futures SBE decoder for the two public ticker channels.
//!
//! Official schema and transport contract:
//! - <https://www.gate.com/docs/developers/futures/ws/en/#sbe-data-push>
//! - <https://github.com/gate/gatews/blob/master/sbe/schemas/prod/gate_fex_ws_latest.xml>

use super::gate_market_data::TickerItem;
use super::gate_ws_ticker_data::{
    CachedBookTicker, CachedMarket, BOOK_TICKER_CHANNEL, ORDER_BOOK_CHANNEL, TICKERS_CHANNEL,
};
use common::time::now_ms;

const SCHEMA_ID: u16 = 1;
const BBO_TEMPLATE_ID: u16 = 1;
const ORDER_BOOK_TEMPLATE_ID: u16 = 4;
const FUTURES_TICKER_TEMPLATE_ID: u16 = 9;
const BBO_ROOT_LENGTH: usize = 59;
const ORDER_BOOK_ROOT_LENGTH: usize = 28;
const ORDER_BOOK_ENTRY_LENGTH: usize = 16;
const TICKER_ROOT_LENGTH: usize = 9;
const TICKER_ENTRY_LENGTH: usize = 122;
const MAX_ORDER_BOOK_ROWS_PER_SIDE: usize = 400;
const MAX_TICKER_ROWS_PER_FRAME: usize = 256;

#[derive(Debug)]
pub(super) enum GateSbeUpdate {
    Book(String, CachedBookTicker),
    BookSnapshot(String, CachedBookTicker),
    Markets(Vec<CachedMarket>),
}

pub(super) fn parse_sbe_update(payload: &[u8]) -> Result<Option<GateSbeUpdate>, &'static str> {
    let mut cursor = Cursor::new(payload);
    let header = Header::read(&mut cursor)?;
    if header.schema_id != SCHEMA_ID || header.version == 0 {
        return Err("unsupported Gate SBE schema");
    }
    match header.template_id {
        BBO_TEMPLATE_ID => parse_bbo(cursor, header.block_length),
        ORDER_BOOK_TEMPLATE_ID => parse_order_book(cursor, header.block_length),
        FUTURES_TICKER_TEMPLATE_ID => parse_tickers(cursor, header.block_length),
        _ => Ok(None),
    }
}

fn parse_bbo(
    mut cursor: Cursor<'_>,
    block_length: usize,
) -> Result<Option<GateSbeUpdate>, &'static str> {
    if block_length < BBO_ROOT_LENGTH {
        return Err("Gate SBE BBO root is shorter than schema");
    }
    let root_start = cursor.offset();
    let server_time_us = cursor.read_i64()?;
    let event = cursor.read_i8()?;
    let engine_time_us = cursor.read_i64()?;
    let _update_id = cursor.read_i64()?;
    let price_exponent = cursor.read_i8()?;
    let _size_exponent = cursor.read_i8()?;
    let ask = scaled_f64(cursor.read_i64()?, price_exponent)?;
    let _ask_size = cursor.read_i64()?;
    let bid = scaled_f64(cursor.read_i64()?, price_exponent)?;
    let _bid_size = cursor.read_i64()?;
    cursor.seek(root_start.saturating_add(block_length))?;
    let channel = cursor.read_string()?;
    let symbol = cursor.read_string()?.to_owned();
    if !is_data_event(event) || channel != BOOK_TICKER_CHANNEL {
        return Ok(None);
    }
    if bid <= 0.0 || ask <= 0.0 || bid > ask {
        return Err("Gate SBE BBO contains invalid prices");
    }
    Ok(Some(GateSbeUpdate::Book(
        symbol,
        CachedBookTicker {
            bid,
            ask,
            cached_at_ms: now_ms(),
            data_timestamp_ms: timestamp_ms(engine_time_us, server_time_us),
        },
    )))
}

fn parse_order_book(
    mut cursor: Cursor<'_>,
    block_length: usize,
) -> Result<Option<GateSbeUpdate>, &'static str> {
    if block_length < ORDER_BOOK_ROOT_LENGTH {
        return Err("Gate SBE order-book root is shorter than schema");
    }
    let root_start = cursor.offset();
    let server_time_us = cursor.read_i64()?;
    let event = cursor.read_i8()?;
    let engine_time_us = cursor.read_i64()?;
    let _update_id = cursor.read_i64()?;
    let price_exponent = cursor.read_i8()?;
    let _size_exponent = cursor.read_i8()?;
    let _level = cursor.read_u8()?;
    cursor.seek(root_start.saturating_add(block_length))?;

    let ask = read_best_price(&mut cursor, price_exponent, false)?;
    let bid = read_best_price(&mut cursor, price_exponent, true)?;
    let channel = cursor.read_string()?;
    let symbol = cursor.read_string()?.to_owned();
    if event != 3 || channel != ORDER_BOOK_CHANNEL {
        return Ok(None);
    }
    let (Some(bid), Some(ask)) = (bid, ask) else {
        return Err("Gate SBE order-book snapshot has no two-sided top of book");
    };
    if bid <= 0.0 || ask <= 0.0 || bid > ask {
        return Err("Gate SBE order-book snapshot contains invalid prices");
    }
    Ok(Some(GateSbeUpdate::BookSnapshot(
        symbol,
        CachedBookTicker {
            bid,
            ask,
            cached_at_ms: now_ms(),
            data_timestamp_ms: timestamp_ms(engine_time_us, server_time_us),
        },
    )))
}

fn read_best_price(
    cursor: &mut Cursor<'_>,
    exponent: i8,
    choose_highest: bool,
) -> Result<Option<f64>, &'static str> {
    let entry_length = usize::from(cursor.read_u16()?);
    let row_count = usize::from(cursor.read_u16()?);
    if entry_length < ORDER_BOOK_ENTRY_LENGTH || row_count > MAX_ORDER_BOOK_ROWS_PER_SIDE {
        return Err("Gate SBE order-book group dimensions are invalid");
    }
    let mut best: Option<f64> = None;
    for _ in 0..row_count {
        let entry_start = cursor.offset();
        let price = scaled_f64(cursor.read_i64()?, exponent)?;
        let size = cursor.read_i64()?;
        cursor.seek(entry_start.saturating_add(entry_length))?;
        if price <= 0.0 || size <= 0 {
            continue;
        }
        best = Some(match best {
            Some(current) if choose_highest => current.max(price),
            Some(current) => current.min(price),
            None => price,
        });
    }
    Ok(best)
}

fn parse_tickers(
    mut cursor: Cursor<'_>,
    block_length: usize,
) -> Result<Option<GateSbeUpdate>, &'static str> {
    if block_length < TICKER_ROOT_LENGTH {
        return Err("Gate SBE ticker root is shorter than schema");
    }
    let root_start = cursor.offset();
    let server_time_us = cursor.read_i64()?;
    let event = cursor.read_i8()?;
    cursor.seek(root_start.saturating_add(block_length))?;
    let entry_length = usize::from(cursor.read_u16()?);
    let row_count = usize::from(cursor.read_u16()?);
    if entry_length < TICKER_ENTRY_LENGTH || row_count > MAX_TICKER_ROWS_PER_FRAME {
        return Err("Gate SBE ticker group dimensions are invalid");
    }
    let mut rows = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        rows.push(parse_ticker_row(&mut cursor, entry_length, server_time_us)?);
    }
    let channel = cursor.read_string()?;
    if !is_data_event(event) || channel != TICKERS_CHANNEL {
        return Ok(None);
    }
    Ok(Some(GateSbeUpdate::Markets(rows)))
}

fn parse_ticker_row(
    cursor: &mut Cursor<'_>,
    entry_length: usize,
    server_time_us: i64,
) -> Result<CachedMarket, &'static str> {
    let entry_start = cursor.offset();
    let data_time_us = cursor.read_i64()?;
    let price_exponent = cursor.read_i8()?;
    let last = decimal_string(cursor.read_i64()?, price_exponent)?;
    let _change_price = cursor.read_i64()?;
    let _low_24h = cursor.read_i64()?;
    let _high_24h = cursor.read_i64()?;
    let mark_exponent = cursor.read_i8()?;
    let mark_price = decimal_string(cursor.read_i64()?, mark_exponent)?;
    let index_exponent = cursor.read_i8()?;
    let index_price = decimal_string(cursor.read_i64()?, index_exponent)?;
    let _change_percentage_exponent = cursor.read_i8()?;
    let _change_percentage = cursor.read_i64()?;
    let funding_exponent = cursor.read_i8()?;
    let funding_rate = decimal_string(cursor.read_i64()?, funding_exponent)?;
    let size_exponent = cursor.read_i8()?;
    let total_size = decimal_string(cursor.read_i64()?, size_exponent)?;
    let _volume_exponent = cursor.read_i8()?;
    let _volume = cursor.read_i64()?;
    let _volume_base_exponent = cursor.read_i8()?;
    let _volume_base = cursor.read_i64()?;
    let quote_exponent = cursor.read_i8()?;
    let volume_24h_quote = decimal_string(cursor.read_i64()?, quote_exponent)?;
    let settle_exponent = cursor.read_i8()?;
    let volume_24h_settle = decimal_string(cursor.read_i64()?, settle_exponent)?;
    cursor.seek(entry_start.saturating_add(entry_length))?;
    let contract = cursor.read_string()?.to_owned();
    let _quanto_base_rate = cursor.read_string()?;
    let _price_type = cursor.read_string()?;
    let _change_from = cursor.read_string()?;
    Ok(CachedMarket {
        item: TickerItem {
            contract,
            last,
            highest_bid: String::new(),
            lowest_ask: String::new(),
            volume_24h_quote,
            volume_24h_settle,
            funding_rate,
            funding_rate_indicative: String::new(),
            funding_next_apply: 0,
            mark_price,
            index_price,
            total_size,
        },
        cached_at_ms: now_ms(),
        data_timestamp_ms: timestamp_ms(data_time_us, server_time_us),
    })
}

fn is_data_event(event: i8) -> bool {
    matches!(event, 2 | 3)
}

fn timestamp_ms(primary_us: i64, fallback_us: i64) -> i64 {
    let value = if primary_us > 0 {
        primary_us
    } else {
        fallback_us
    };
    value
        .checked_div(1_000)
        .filter(|value| *value > 0)
        .unwrap_or_else(now_ms)
}

fn scaled_f64(mantissa: i64, exponent: i8) -> Result<f64, &'static str> {
    if !(-18..=18).contains(&exponent) {
        return Err("Gate SBE decimal exponent is out of range");
    }
    let value = (mantissa as f64) * 10_f64.powi(i32::from(exponent));
    value
        .is_finite()
        .then_some(value)
        .ok_or("Gate SBE decimal is not finite")
}

fn decimal_string(mantissa: i64, exponent: i8) -> Result<String, &'static str> {
    if !(-18..=18).contains(&exponent) {
        return Err("Gate SBE decimal exponent is out of range");
    }
    let negative = mantissa < 0;
    let mut digits = i128::from(mantissa).abs().to_string();
    if exponent >= 0 {
        digits.extend(std::iter::repeat_n('0', exponent as usize));
    } else {
        let scale = usize::from(exponent.unsigned_abs());
        if digits.len() <= scale {
            let mut scaled = String::with_capacity(scale.saturating_add(2));
            scaled.push_str("0.");
            scaled.extend(std::iter::repeat_n('0', scale - digits.len()));
            scaled.push_str(&digits);
            digits = scaled;
        } else {
            digits.insert(digits.len() - scale, '.');
        }
    }
    if negative {
        digits.insert(0, '-');
    }
    Ok(digits)
}

#[derive(Debug, Clone, Copy)]
struct Header {
    block_length: usize,
    template_id: u16,
    schema_id: u16,
    version: u16,
}

impl Header {
    fn read(cursor: &mut Cursor<'_>) -> Result<Self, &'static str> {
        Ok(Self {
            block_length: usize::from(cursor.read_u16()?),
            template_id: cursor.read_u16()?,
            schema_id: cursor.read_u16()?,
            version: cursor.read_u16()?,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct Cursor<'a> {
    payload: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(payload: &'a [u8]) -> Self {
        Self { payload, offset: 0 }
    }

    const fn offset(self) -> usize {
        self.offset
    }

    fn seek(&mut self, offset: usize) -> Result<(), &'static str> {
        if offset > self.payload.len() {
            return Err("Gate SBE frame is truncated");
        }
        self.offset = offset;
        Ok(())
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], &'static str> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or("Gate SBE cursor overflow")?;
        let bytes = self
            .payload
            .get(self.offset..end)
            .ok_or("Gate SBE frame is truncated")?;
        self.offset = end;
        Ok(bytes)
    }

    fn read_u16(&mut self) -> Result<u16, &'static str> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| "Gate SBE uint16 is truncated")?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_i64(&mut self) -> Result<i64, &'static str> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| "Gate SBE int64 is truncated")?;
        Ok(i64::from_le_bytes(bytes))
    }

    fn read_i8(&mut self) -> Result<i8, &'static str> {
        Ok(i8::from_le_bytes([self.take(1)?[0]]))
    }

    fn read_u8(&mut self) -> Result<u8, &'static str> {
        Ok(self.take(1)?[0])
    }

    fn read_string(&mut self) -> Result<&'a str, &'static str> {
        let len = usize::from(self.take(1)?[0]);
        std::str::from_utf8(self.take(len)?).map_err(|_| "Gate SBE string is not UTF-8")
    }
}

#[cfg(test)]
#[path = "gate_ws_ticker_sbe_tests.rs"]
mod tests;
