use tracing::{info, warn};

pub(super) fn main_p0_snapshot_query_key() -> String {
    "scope=main_p0;strategy=perp_cross,perp_price_spread,spot_perp,cross_spot_perp,spot_cross;symbol=*;minYield=*;limit=*;fast=false;fresh=false".into()
}

pub(super) fn log_scan_timing(
    scan_elapsed: std::time::Duration,
    publish_elapsed: std::time::Duration,
    slow_threshold: std::time::Duration,
    count: usize,
    prewarm: bool,
) {
    let scan_ms = scan_elapsed.as_millis() as u64;
    let publish_ms = publish_elapsed.as_millis() as u64;
    if scan_elapsed >= slow_threshold {
        log_slow_scan(scan_ms, publish_ms, count, prewarm);
    } else {
        log_done_scan(scan_ms, publish_ms, count, prewarm);
    }
}

fn log_slow_scan(scan_ms: u64, publish_ms: u64, count: usize, prewarm: bool) {
    warn!(scan_ms, publish_ms, count, prewarm, "arbitrage scan slow");
}

fn log_done_scan(scan_ms: u64, publish_ms: u64, count: usize, prewarm: bool) {
    info!(scan_ms, publish_ms, count, prewarm, "arbitrage scan done");
}
