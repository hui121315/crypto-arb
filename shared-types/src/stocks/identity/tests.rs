use super::*;
use crate::stocks::StockChainToken;

#[test]
fn stock_identity_covers_reviewed_issuers_and_rejects_cross_product_aliases() {
    for profile in ISSUERS {
        let security = StockSecurity {
            asset: profile.asset.into(),
            ticker: profile.asset.trim_end_matches(".US").into(),
            name: profile.native_name.into(),
            cusip: profile.cusip.map(str::to_owned),
            sessions: vec![],
            order_books: vec![],
            rfq_symbol: format!("{}_USDC_RFQ", profile.asset),
        };
        let token = StockChainToken {
            blockchain: "Solana".into(),
            contract_address: Some(profile.solana_mint.into()),
            native_decimals: Some(profile.decimals),
            deposit_enabled: Some(false),
            withdraw_enabled: Some(false),
            minimum_deposit: None,
            minimum_withdrawal: None,
            maximum_withdrawal: None,
            withdrawal_fee: None,
        };
        let snapshot = StockMarketSnapshot {
            security: Some(security.clone()),
            tokens: vec![token.clone()],
            ..Default::default()
        };
        assert_eq!(
            backpack_token_identity(&snapshot).unwrap().asset,
            profile.asset
        );
        // A reviewed relationship is not evidence that current transfers are open.
        assert_eq!(snapshot.tokens[0].withdraw_enabled, Some(false));
        let mut wrong = snapshot.clone();
        wrong.tokens[0].native_decimals = Some(9);
        assert!(backpack_token_identity(&wrong).is_err());
        wrong = snapshot.clone();
        wrong.tokens[0].contract_address = Some("same-ticker-other-issuer".into());
        assert!(backpack_token_identity(&wrong).is_err());
        wrong = snapshot.clone();
        wrong.tokens.push(token);
        assert!(backpack_token_identity(&wrong).is_err());
        wrong = snapshot.clone();
        wrong.security.as_mut().unwrap().cusip = Some("different-security".into());
        assert!(backpack_token_identity(&wrong).is_err());
        wrong = snapshot.clone();
        wrong.security.as_mut().unwrap().asset = "SPCF.US".into();
        assert!(backpack_token_identity(&wrong).is_err());
        if profile.cusip.is_none() {
            assert!(profile.kraken.is_none());
            wrong = snapshot;
            wrong.security.as_mut().unwrap().name = "SpaceX Daily 2x ETF".into();
            assert!(backpack_token_identity(&wrong).is_err());
        } else {
            let x = profile.kraken.as_ref().unwrap();
            assert_eq!(&x.underlying_isin[2..11], profile.cusip.unwrap());
            assert_ne!(x.product_isin, x.underlying_isin);
        }
    }
}
