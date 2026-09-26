use super::*;
use std::sync::atomic::AtomicBool;

type AccountScopes = HashMap<(String, shared_types::FeeProduct), String>;

#[derive(Clone)]
pub(crate) struct PrivateWsAccountLease(Arc<AtomicBool>);

impl PrivateWsAccountLease {
    pub(crate) fn is_current(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub(super) struct PrivateWsAccount {
    scopes: AccountScopes,
    lease: PrivateWsAccountLease,
}

impl TradingService {
    pub(crate) fn configured_account_scope(
        venue: &str,
        product: shared_types::FeeProduct,
    ) -> Option<String> {
        let credentials = crate::services::trading_credentials::current_adapter_credentials();
        live_adapters::account_scopes::from_credentials(&credentials)
            .remove(&(normalized_venue_name(venue), product))
    }

    pub(crate) fn private_ws_credentials_match(&self, credentials: &AdapterCredentials) -> bool {
        let expected = live_adapters::account_scopes::from_credentials(credentials);
        self.account_reader
            .load_full()
            .map(|reader| reader.account_scopes == expected)
            .unwrap_or_else(|| expected.is_empty())
    }

    pub(crate) fn private_ws_account_lease(&self, venue: &str) -> PrivateWsAccountLease {
        self.private_ws_accounts
            .read()
            .get(&family(venue))
            .map(|account| account.lease.clone())
            .unwrap_or_else(|| PrivateWsAccountLease(Arc::new(AtomicBool::new(false))))
    }

    pub(crate) fn invalidate_private_ws_account(&self, venue: &str) {
        if let Some(account) = self.private_ws_accounts.write().remove(&family(venue)) {
            account.lease.0.store(false, Ordering::Release);
        }
    }

    pub(super) fn replace_private_ws_accounts(&self, scopes: &AccountScopes) {
        let mut grouped = HashMap::<String, AccountScopes>::new();
        for (key, value) in scopes {
            grouped
                .entry(family(&key.0))
                .or_default()
                .insert(key.clone(), value.clone());
        }
        let mut accounts = self.private_ws_accounts.write();
        accounts.retain(|venue, previous| {
            if grouped.get(venue) == Some(&previous.scopes) {
                grouped.remove(venue);
                true
            } else {
                // A -> B -> A must not revive a connection that belonged to the first A.
                previous.lease.0.store(false, Ordering::Release);
                false
            }
        });
        for (venue, scopes) in grouped {
            accounts.insert(
                venue,
                PrivateWsAccount {
                    scopes,
                    lease: PrivateWsAccountLease(Arc::new(AtomicBool::new(true))),
                },
            );
        }
    }
}

fn family(venue: &str) -> String {
    VenueId::from_exchange_name(venue)
        .map(|venue| venue.as_str().to_owned())
        .unwrap_or_else(|| normalized_venue_name(venue))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::FeeProduct;

    #[test]
    fn account_lease_rotation_preserves_only_unchanged_venues() {
        let service = TradingService::new_mock();
        let mut scopes = AccountScopes::from([
            (("binance".into(), FeeProduct::Perp), "account-a".into()),
            (("kraken".into(), FeeProduct::Spot), "spot-a".into()),
            (("kraken".into(), FeeProduct::Perp), "futures-a".into()),
        ]);
        service.replace_private_ws_accounts(&scopes);
        let binance = service.private_ws_account_lease("binance");
        let kraken = service.private_ws_account_lease("kraken");
        service.replace_private_ws_accounts(&scopes);
        assert!(binance.is_current() && kraken.is_current());
        scopes.insert(("kraken".into(), FeeProduct::Spot), "spot-b".into());
        service.replace_private_ws_accounts(&scopes);
        assert!(binance.is_current());
        assert!(!kraken.is_current());
        let replacement = service.private_ws_account_lease("kraken");
        scopes.insert(("kraken".into(), FeeProduct::Spot), "spot-a".into());
        service.replace_private_ws_accounts(&scopes);
        assert!(!kraken.is_current() && !replacement.is_current());
        assert!(service.private_ws_account_lease("kraken").is_current());
        service.replace_private_ws_accounts(&AccountScopes::new());
        assert!(!binance.is_current());
        assert!(!service.private_ws_account_lease("kraken").is_current());
    }
}
