#![allow(clippy::print_stdout)]

use exchange::{
    Binance, BinanceConfig, Bitget, BitgetConfig, Bybit, BybitConfig, ExchangeAdapter, Gate,
    GateConfig, Hyperliquid, HyperliquidConfig, Kucoin, KucoinConfig, Okx, OkxConfig,
};
use shared_types::SpotTick;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

const PROBE_TIMEOUT: Duration = Duration::from_secs(18);

#[derive(Debug)]
struct Probe {
    venue: &'static str,
    count: usize,
    sample: Option<String>,
    error: Option<String>,
}

impl Probe {
    fn ok(venue: &'static str, ticks: &[SpotTick]) -> Self {
        Self {
            venue,
            count: ticks.len(),
            sample: ticks.first().map(sample_tick),
            error: None,
        }
    }

    fn fail(venue: &'static str, error: impl Into<String>) -> Self {
        Self {
            venue,
            count: 0,
            sample: None,
            error: Some(error.into()),
        }
    }

    fn is_ok(&self) -> bool {
        self.error.is_none() && self.count > 0
    }

    fn line(&self) -> String {
        match &self.error {
            Some(error) => format!("{} spot fail error={}", self.venue, error),
            None => format!(
                "{} spot ok count={} sample={}",
                self.venue,
                self.count,
                self.sample.as_deref().unwrap_or("-")
            ),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let probes = futures::future::join_all(adapters()?.into_iter().map(probe_adapter)).await;
    let failed = probes.iter().filter(|probe| !probe.is_ok()).count();
    for probe in probes {
        println!("{}", probe.line());
    }
    if failed == 0 {
        Ok(())
    } else {
        Err(format!("{failed} spot API probe(s) failed").into())
    }
}

async fn probe_adapter(adapter: Arc<dyn ExchangeAdapter>) -> Probe {
    let venue = adapter.name();
    match timeout(PROBE_TIMEOUT, adapter.get_spot_tickers(None)).await {
        Ok(Ok(ticks)) if ticks.is_empty() => Probe::fail(venue, "empty spot ticks"),
        Ok(Ok(ticks)) => Probe::ok(venue, &ticks),
        Ok(Err(error)) => Probe::fail(venue, error.to_string()),
        Err(_) => Probe::fail(venue, format!("timeout {}s", PROBE_TIMEOUT.as_secs())),
    }
}

fn adapters() -> exchange::ExchangeResult<Vec<Arc<dyn ExchangeAdapter>>> {
    Ok(vec![
        Arc::new(Binance::new(BinanceConfig {
            timeout_secs: 12,
            ..Default::default()
        })?),
        Arc::new(Okx::new(OkxConfig {
            timeout_secs: 12,
            ..Default::default()
        })?),
        Arc::new(Bybit::new(BybitConfig {
            timeout_secs: 12,
            ..Default::default()
        })?),
        Arc::new(Bitget::new(BitgetConfig {
            timeout_secs: 12,
            ..Default::default()
        })?),
        Arc::new(Gate::new(GateConfig {
            timeout_secs: 12,
            allow_live_writes: false,
            ..Default::default()
        })?),
        Arc::new(Kucoin::new(KucoinConfig {
            timeout_secs: 12,
            ..Default::default()
        })?),
        Arc::new(Hyperliquid::new(HyperliquidConfig {
            timeout_secs: 12,
            ..Default::default()
        })?),
    ])
}

fn sample_tick(tick: &SpotTick) -> String {
    format!(
        "{}:{} last={}",
        tick.venue,
        tick.symbol,
        tick.last.normalize()
    )
}
