use super::*;
use shared_types::stocks::StockMintEvidence;

const CACHE_MS: i64 = 20_000;
type MintRows = Result<Vec<Result<StockMintEvidence, String>>, String>;

pub(super) struct MintCache {
    targets: Vec<(String, u8)>,
    state: tokio::sync::Mutex<Option<(i64, MintRows)>>,
}

impl MintCache {
    pub(super) fn new(targets: Vec<(String, u8)>) -> Self {
        Self {
            targets,
            state: tokio::sync::Mutex::new(None),
        }
    }

    pub(super) async fn get(&self, address: &str, source: &stock_quotes::Source) -> Result<StockMintEvidence, String> {
        self.get_with(address, || async {
            tokio::time::timeout(
                Duration::from_secs(15),
                source.batch_mints(&self.targets),
            )
            .await
            .map_err(|_| "批量合约核验超时".to_owned())?
        })
        .await
    }

    async fn get_with<F, Fut>(&self, address: &str, load: F) -> Result<StockMintEvidence, String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = MintRows>,
    {
        let matches = self
            .targets
            .iter()
            .enumerate()
            .filter(|(_, (a, _))| a == address)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let [index] = matches.as_slice() else {
            return Err("重复或缺失的官方合约".into());
        };
        // Only one refresh may run. Later quote jobs reuse this batch, not the round's old clock.
        let mut state = self.state.lock().await;
        let now = common::time::now_ms();
        let fresh = state.as_ref().is_some_and(|(at, rows)| {
            now >= *at
                && now - at < CACHE_MS
                && !rows
                    .as_ref()
                    .ok()
                    .and_then(|r| r.get(*index))
                    .and_then(|m| m.as_ref().ok())
                    .is_some_and(|m| m.next_change_at_ms.is_some_and(|at| now >= at))
        });
        if !fresh {
            let result = load().await.and_then(|rows| {
                if rows.len() != self.targets.len() {
                    Err("批量合约返回数量不匹配".into())
                } else {
                    Ok(rows)
                }
            });
            *state = Some((common::time::now_ms(), result));
        }
        let rows = &state.as_ref().unwrap().1;
        match rows {
            Ok(rows) => rows[*index].clone(),
            Err(error) => Err(error.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn mint(at: i64) -> StockMintEvidence {
        StockMintEvidence {
            address: "mint".into(),
            decimals: 6,
            ui_multiplier: "1".into(),
            slot: 1,
            chain_time_ms: at,
            checked_at_ms: at,
            next_change_at_ms: None,
            extensions: vec![],
        }
    }

    #[tokio::test]
    async fn batch_mint_cache_coalesces_refresh_and_does_not_reuse_old_or_changed_context() {
        let cache = MintCache::new(vec![("mint".into(), 6)]);
        let calls = AtomicUsize::new(0);
        let load = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(1)).await;
            Ok(vec![Ok(mint(common::time::now_ms()))])
        };
        let (a, b) = tokio::join!(cache.get_with("mint", load), cache.get_with("mint", load));
        assert_eq!(a.unwrap(), b.unwrap());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache.state.lock().await.as_mut().unwrap().0 -= CACHE_MS;
        assert!(cache.get_with("mint", load).await.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        {
            let mut state = cache.state.lock().await;
            state.as_mut().unwrap().1.as_mut().unwrap()[0]
                .as_mut()
                .unwrap()
                .next_change_at_ms = Some(common::time::now_ms());
        }
        assert!(cache.get_with("mint", load).await.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn batch_mint_cache_bounds_failure_retries_and_isolates_rows() {
        let cache = MintCache::new(vec![("mint".into(), 6), ("missing".into(), 6)]);
        let result = cache
            .get_with("mint", || async {
                Ok(vec![
                    Ok(mint(common::time::now_ms())),
                    Err("missing token".into()),
                ])
            })
            .await;
        assert!(result.is_ok());
        assert_eq!(
            cache
                .get_with("missing", || async { panic!("must reuse batch") })
                .await
                .unwrap_err(),
            "missing token"
        );
        cache.state.lock().await.as_mut().unwrap().0 -= CACHE_MS;
        assert!(cache
            .get_with("mint", || async { Err("RPC down".into()) })
            .await
            .is_err());
        assert_eq!(
            cache
                .get_with("missing", || async { panic!("must back off") })
                .await
                .unwrap_err(),
            "RPC down"
        );
        assert!(cache
            .get_with("unknown", || async { panic!("unknown target") })
            .await
            .is_err());
    }
}
