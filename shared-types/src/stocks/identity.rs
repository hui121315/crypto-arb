use super::{StockMarketSnapshot, StockSecurity};

#[derive(Debug)]
pub struct StockIssuerProfile {
    pub asset: &'static str,
    pub cusip: Option<&'static str>,
    pub native_name: &'static str,
    pub solana_mint: &'static str,
    pub decimals: u8,
    pub redemption_source: &'static str,
    pub kraken: Option<StockXstockProfile>,
}

#[derive(Debug)]
pub struct StockXstockProfile {
    pub base: &'static str,
    pub underlying_isin: &'static str,
    pub product_isin: &'static str,
    pub source: &'static str,
}

// Reviewed issuer relationships, not symbol guesses. Live catalog and Mint checks
// remain necessary; these records grant neither transfer nor order permission.
const ISSUERS: &[StockIssuerProfile] = &[
    StockIssuerProfile {
        asset: "MU.US",
        cusip: Some("595112103"),
        native_name: "Micron Technology, Inc.",
        solana_mint: "MUxEsUKSMACyw5fZf68wxf5FLnZVhtU9CwH8uNNGay1",
        decimals: 6,
        redemption_source: "https://learn.backpack.exchange/blog/tokenized-micron-mu",
        kraken: Some(StockXstockProfile {
            base: "MUx",
            underlying_isin: "US5951121038",
            product_isin: "CH1473121320",
            source: "https://assets.backed.fi/products/micron-technology-xstock",
        }),
    },
    StockIssuerProfile {
        asset: "SNDK.US",
        cusip: Some("80004C200"),
        native_name: "Sandisk Corporation",
        solana_mint: "SNDKbwMUQvZhnLnxLduradgLHG5KrPuKwpnrkkGRhfH",
        decimals: 6,
        redemption_source: "https://learn.backpack.exchange/blog/tokenized-sandisk-sndk",
        kraken: Some(StockXstockProfile {
            base: "SNDKx",
            underlying_isin: "US80004C2008",
            product_isin: "CH1500008748",
            source: "https://assets.backed.fi/products/sandisk-corporation-xstock",
        }),
    },
    StockIssuerProfile {
        asset: "SPCX.US",
        cusip: None,
        native_name: "SpaceX",
        solana_mint: "SPCXxcqXj6e5dJDVNovHN8744zkbhM2bYudU45BimGb",
        decimals: 6,
        redemption_source: "https://learn.backpack.exchange/blog/tokenized-spacex-spcx",
        kraken: None,
    },
];

pub fn backpack_issuer(
    security: &StockSecurity,
) -> Result<&'static StockIssuerProfile, &'static str> {
    let profile = backpack_issuer_profile(&security.asset)
        .ok_or("该证券的发行方合约与份额兑换关系尚未核齐；可继续查看官方行情")?;
    // SPCX currently has no published CUSIP. Its exact issuer-native asset/name
    // and independently checked API mint are used only within Backpack/Solana.
    if security.cusip.as_deref() != profile.cusip
        || (profile.cusip.is_none() && security.name != profile.native_name)
    {
        return Err("官方证券身份与已核实发行资料不一致，请重新核对");
    }
    Ok(profile)
}

pub fn backpack_issuer_profile(asset: &str) -> Option<&'static StockIssuerProfile> {
    ISSUERS.iter().find(|p| p.asset == asset)
}

pub fn backpack_token_identity(
    snapshot: &StockMarketSnapshot,
) -> Result<&'static StockIssuerProfile, &'static str> {
    let profile = backpack_issuer(snapshot.security.as_ref().ok_or("请先选择股票")?)?;
    let mut tokens = snapshot.tokens.iter().filter(|t| t.blockchain == "Solana");
    let token = tokens.next().ok_or("官方目录缺少该股票的 Solana 映射")?;
    if tokens.next().is_some()
        || token.contract_address.as_deref() != Some(profile.solana_mint)
        || token.native_decimals != Some(profile.decimals)
    {
        return Err("官方资产目录与发行方合约/精度不一致");
    }
    Ok(profile)
}

#[cfg(test)]
mod tests;
