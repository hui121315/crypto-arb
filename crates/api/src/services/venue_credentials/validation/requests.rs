use super::*;

#[cfg(not(test))]
pub(super) async fn validate_balance<A>(
    adapter: &A,
    timeout_secs: u64,
) -> Result<(), CredentialUpdateError>
where
    A: exchange::ExchangeAdapter + ?Sized,
{
    match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        adapter.get_balance(None),
    )
    .await
    {
        Ok(Ok(balances)) => ensure_balance_probe_is_well_formed(&balances),
        Ok(Err(error)) => Err(validation_error(&error)),
        Err(_) => Err(CredentialUpdateError::ValidationTimeout),
    }
}

pub(super) fn ensure_balance_probe_is_well_formed(
    balances: &std::collections::HashMap<String, BalanceInfo>,
) -> Result<(), CredentialUpdateError> {
    let malformed = balances.iter().any(|(key, balance)| {
        key.trim().is_empty()
            || balance.currency.trim().is_empty()
            || !balance.total.is_finite()
            || !balance.available.is_finite()
            || !balance.frozen.is_finite()
            || !balance.unrealized_pnl.is_finite()
    });
    if malformed {
        Err(CredentialUpdateError::Validation(
            "balance probe returned a malformed row".into(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(test))]
pub(super) async fn validate_exchange_request<F, T>(
    request: F,
    timeout_secs: u64,
) -> Result<(), CredentialUpdateError>
where
    F: std::future::Future<Output = exchange::ExchangeResult<T>>,
{
    match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), request).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(validation_error(&error)),
        Err(_) => Err(CredentialUpdateError::ValidationTimeout),
    }
}

#[cfg(not(test))]
pub(super) fn validation_error(error: &exchange::ExchangeError) -> CredentialUpdateError {
    match error {
        exchange::ExchangeError::Auth(_) => {
            CredentialUpdateError::PermissionDenied(error.to_string())
        }
        _ => CredentialUpdateError::Validation(error.to_string()),
    }
}
