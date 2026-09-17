use super::*;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::stocks) struct AlertData {
    pub enabled: RwSignal<bool>,
    pub include_peer: RwSignal<bool>,
    pub threshold: RwSignal<String>,
    pub cooldown: RwSignal<String>,
}

impl AlertData {
    pub(super) fn new(market: RwSignal<LoadState<StockMarketSnapshot>>) -> Self {
        let data = Self::defaults();
        let applied = StoredValue::new(None::<(String, StockAlertConfig)>);
        Effect::new(move |_| {
            let incoming = market.with(|m| {
                m.value().and_then(|s| {
                    s.security
                        .as_ref()
                        .map(|security| (security.asset.clone(), s.monitor.alerts.clone()))
                })
            });
            if applied.with_value(|old| old != &incoming) {
                let config = incoming
                    .as_ref()
                    .map(|(_, config)| config.clone())
                    .unwrap_or_default();
                data.enabled.set(config.enabled);
                data.include_peer.set(config.include_peer);
                data.threshold.set(config.min_spread_pct);
                data.cooldown.set(config.cooldown_secs.to_string());
                applied.set_value(incoming);
            }
        });
        data
    }

    pub(in crate::panels::modules::stocks) fn defaults() -> Self {
        let config = StockAlertConfig::default();
        Self {
            enabled: RwSignal::new(false),
            include_peer: RwSignal::new(false),
            threshold: RwSignal::new(config.min_spread_pct),
            cooldown: RwSignal::new(config.cooldown_secs.to_string()),
        }
    }

    pub(super) fn config(self, running: bool) -> Result<StockAlertConfig, String> {
        let enabled = self.enabled.get_untracked();
        let cooldown = self.cooldown.get_untracked().parse::<u32>();
        if running && enabled && cooldown.is_err() {
            return Err("提醒间隔必须是整数秒".into());
        }
        let config = StockAlertConfig {
            enabled,
            include_peer: self.include_peer.get_untracked(),
            min_spread_pct: self.threshold.get_untracked(),
            cooldown_secs: cooldown.unwrap_or(60),
        };
        if running && enabled {
            config.threshold()?;
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_alert_draft_accepts_decimal_threshold_and_invalid_draft_cannot_block_stop() {
        Owner::new().with(|| {
            let data = AlertData::defaults();
            data.enabled.set(true);
            assert!(!data.config(true).unwrap().include_peer);
            data.include_peer.set(true);
            assert!(data.config(true).unwrap().include_peer);
            data.threshold.set("0.01".into());
            assert_eq!(data.config(true).unwrap().min_spread_pct, "0.01");
            data.cooldown.set("broken".into());
            assert!(data.config(true).is_err());
            assert!(data.config(false).is_ok());
        });
    }
}
