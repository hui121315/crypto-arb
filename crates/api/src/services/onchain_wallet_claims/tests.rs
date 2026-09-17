use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Barrier,
};

#[test]
fn wallet_claims_atomically_choose_one_owner_without_serializing_other_wallets_lifetimes() {
    let claims = Arc::new(WalletClaims::default());
    let barrier = Arc::new(Barrier::new(2));
    let writes = Arc::new(AtomicUsize::new(0));
    let handles = [Module::Recovery, Module::Execution].map(|module| {
        let (claims, barrier, writes) = (claims.clone(), barrier.clone(), writes.clone());
        std::thread::spawn(move || {
            barrier.wait();
            claims.commit(
                Owner::new(module, "same-wallet"),
                Some(Hold::wallet("ethereum", "0xAbC", None).unwrap()),
                1000,
                || {
                    writes.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
            )
        })
    });
    assert_eq!(
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(Result::is_ok)
            .count(),
        1
    );
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert!(claims.check("ethereum", "0xabc", 1001).is_err());
    assert!(claims.check("base", "0xabc", 1001).is_ok());
    assert!(claims.check("ethereum", "0xdef", 1001).is_ok());
    claims
        .commit(
            Owner::new(Module::Approval, "another"),
            Some(Hold::wallet("solana", "ABC", None).unwrap()),
            1001,
            || Ok(()),
        )
        .unwrap();
    assert!(claims.check("solana", "abc", 1001).is_ok());
}

#[test]
fn wallet_claims_only_expire_unsubmitted_leases_and_allow_safe_release_after_foreign_corruption() {
    let claims = WalletClaims::default();
    let owner = Owner::new(Module::Recovery, "lease");
    claims.restore(
        Module::Recovery,
        Ok(vec![(
            owner.clone(),
            Hold::wallet("solana", "wallet", Some(2000)).unwrap(),
        )]),
        None,
    );
    assert!(claims.check("solana", "wallet", 1999).is_err());
    assert!(claims.check("solana", "wallet", 2000).is_ok());
    let active = Owner::new(Module::Execution, "pending");
    claims
        .commit(
            active.clone(),
            Some(Hold::wallet("solana", "wallet", None).unwrap()),
            2000,
            || Ok(()),
        )
        .unwrap();
    assert!(claims.check("solana", "wallet", i64::MAX).is_err());
    claims.restore(Module::Approval, Err("truncated journal".into()), None);
    assert!(claims.check("base", "other", 3000).is_err());
    let mut called = false;
    assert!(claims
        .commit(
            owner,
            Some(Hold::wallet("base", "other", None).unwrap()),
            3000,
            || {
                called = true;
                Ok(())
            }
        )
        .is_err());
    assert!(!called);
    claims.commit(active, None, 4000, || Ok(())).unwrap();
    assert!(claims.inner.lock().holds.values().all(|h| !h.active(4000)));
}

#[test]
fn wallet_claims_failed_fsync_blocks_new_owners_and_process_lock_is_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wallets.lock");
    let claims = WalletClaims::exclusive(&path).unwrap();
    assert!(WalletClaims::exclusive(&path).is_err());
    assert!(claims
        .commit(
            Owner::new(Module::Replenishment, "write-failed"),
            Some(Hold::wallet("ethereum", "0xabc", None).unwrap()),
            1000,
            || Err("disk full".into())
        )
        .is_err());
    assert!(claims.check("solana", "other", 1001).is_err());
    drop(claims);
    assert!(WalletClaims::exclusive(&path).is_ok());
}
