use super::*;
use serde_json::json;

const RENT: u64 = 2_039_280;
const INITIAL: u64 = 1_000_000_000;
const FEE: u64 = 5_000;
const WSOL: &str = super::super::SOLANA_WRAPPED_SOL_MINT;

fn transaction() -> Value {
    json!({"slot":120,"transaction":{"signatures":["signature"],"message":{
        "accountKeys":[{"pubkey":"wallet","signer":true},"account","pool","sponsor",SYSTEM,TOKEN],
        "instructions":[]}},"meta":{"err":null,"fee":FEE,"innerInstructions":[],
        "preBalances":[INITIAL,0,INITIAL,INITIAL,1,1],
        "postBalances":[INITIAL-FEE,0,INITIAL,INITIAL,1,1],
        "preTokenBalances":[],"postTokenBalances":[]}})
}
fn token(mint: &str, amount: u64) -> Value {
    json!({"accountIndex":1,"mint":mint,"owner":"wallet","programId":TOKEN,
        "uiTokenAmount":{"amount":amount.to_string(),"decimals":9}})
}
fn close() -> Value {
    json!({"programId":TOKEN,"parsed":{"type":"closeAccount","info":{
        "account":"account","destination":"wallet","owner":"wallet"}}})
}
fn create(source: &str) -> Value {
    json!({"programId":SYSTEM,"parsed":{"type":"createAccount","info":{
        "source":source,"newAccount":"account","lamports":RENT,"space":165,"owner":TOKEN}}})
}

#[test]
fn network_fee_is_added_back_only_for_the_actual_fee_payer() {
    let mut tx = transaction();
    tx["meta"]["postBalances"][0] = json!(INITIAL + 1_000_000 - FEE);
    assert_eq!(native_credit(&tx, "wallet").unwrap(), 1_000_000);
    tx["meta"]["postBalances"][2] = json!(INITIAL + 1_000_000);
    assert_eq!(native_credit(&tx, "pool").unwrap(), 1_000_000);
    tx["meta"]["fee"] = Value::Null;
    assert!(native_credit(&tx, "wallet").is_err());
    assert_eq!(native_credit(&tx, "pool").unwrap(), 1_000_000);
}

#[test]
fn old_account_rent_and_existing_wsol_are_not_new_swap_output() {
    for (mint, old_amount) in [("USDC", 0), (WSOL, 1_000_000)] {
        let mut tx = transaction();
        tx["transaction"]["message"]["instructions"] = json!([close()]);
        tx["meta"]["preTokenBalances"] = json!([token(mint, old_amount)]);
        tx["meta"]["preBalances"][1] = json!(RENT + old_amount);
        tx["meta"]["postBalances"][0] = json!(INITIAL + RENT + old_amount - FEE);
        assert_eq!(native_credit(&tx, "wallet").unwrap(), 0);
    }
    let mut tx = transaction();
    tx["meta"]["preTokenBalances"] = json!([token(WSOL, 1_000_000)]);
    tx["meta"]["preBalances"][1] = json!(RENT + 1_000_000);
    tx["meta"]["postBalances"][0] = json!(INITIAL + RENT + 3_000_000 - FEE);
    tx["transaction"]["message"]["instructions"] = json!([
        {"programId":TOKEN,"parsed":{"type":"transferChecked","info":{
            "source":"pool","destination":"account","mint":WSOL,
            "tokenAmount":{"amount":"2000000","decimals":9}}}}, close()
    ]);
    assert_eq!(native_credit(&tx, "wallet").unwrap(), 2_000_000);
    let mut wrong_mint = tx.clone();
    wrong_mint["transaction"]["message"]["instructions"][0]["parsed"]["info"]["mint"] =
        json!("USDC");
    assert!(native_credit(&wrong_mint, "wallet").is_err());
    tx["transaction"]["message"]["instructions"][0]["parsed"]["info"]["source"] = json!("account");
    tx["transaction"]["message"]["instructions"][0]["parsed"]["info"]["destination"] =
        json!("pool");
    tx["transaction"]["message"]["instructions"][0]["parsed"]["info"]["tokenAmount"]["amount"] =
        json!("1000000");
    tx["meta"]["postBalances"][0] = json!(INITIAL + RENT - FEE);
    assert_eq!(native_credit(&tx, "wallet").unwrap(), 0);
}

#[test]
fn wrapped_output_is_not_reported_as_spendable_native_sol() {
    let mut tx = transaction();
    tx["meta"]["preBalances"][1] = json!(RENT);
    tx["meta"]["postBalances"][1] = json!(RENT + 1_000_000);
    tx["meta"]["preTokenBalances"] = json!([token(WSOL, 0)]);
    tx["meta"]["postTokenBalances"] = json!([token(WSOL, 1_000_000)]);
    let problem = native_credit(&tx, "wallet").unwrap_err();
    assert!(problem.contains("WSOL"));
    assert!(problem.contains("1000000"));
    tx["meta"]["postBalances"][0] = json!(INITIAL + 500_000 - FEE);
    assert_eq!(native_credit(&tx, "wallet").unwrap(), 500_000);
}

#[test]
fn wallet_funded_rent_is_not_deducted_twice_and_sponsored_rent_is_not_income() {
    let mut tx = transaction();
    tx["transaction"]["message"]["instructions"] = json!([create("wallet")]);
    tx["meta"]["postTokenBalances"] = json!([token("USDC", 0)]);
    tx["meta"]["postBalances"][1] = json!(RENT);
    tx["meta"]["postBalances"][0] = json!(INITIAL + 1_000_000 - RENT - FEE);
    assert_eq!(native_credit(&tx, "wallet").unwrap(), 1_000_000);
    let mut mixed_funding = tx.clone();
    mixed_funding["transaction"]["message"]["instructions"][0]["parsed"]["info"]["lamports"] =
        json!(1);
    assert!(native_credit(&mixed_funding, "wallet").is_err());
    tx["transaction"]["message"]["instructions"] = json!([create("sponsor")]);
    tx["meta"]["postBalances"][0] = json!(INITIAL + 1_000_000 - FEE);
    assert_eq!(native_credit(&tx, "wallet").unwrap(), 1_000_000);
    tx["meta"]["postTokenBalances"] = json!([]);
    tx["meta"]["postBalances"][1] = json!(0);
    tx["transaction"]["message"]["instructions"] = json!([create("wallet"), close()]);
    assert_eq!(native_credit(&tx, "wallet").unwrap(), 1_000_000);
    tx["transaction"]["message"]["instructions"] = json!([create("sponsor"), close()]);
    assert!(native_credit(&tx, "wallet")
        .unwrap_err()
        .contains("其他地址"));
}

#[test]
fn inner_instructions_count_once_and_malformed_identity_cannot_prove_credit() {
    let mut tx = transaction();
    tx["transaction"]["message"]["instructions"] = json!([{"programId":"router","data":"opaque"}]);
    tx["meta"]["innerInstructions"] =
        json!([{"index":0,"instructions":[create("wallet"),close()]}]);
    tx["meta"]["postBalances"][0] = json!(INITIAL + 1_000_000 - FEE);
    assert_eq!(native_credit(&tx, "wallet").unwrap(), 1_000_000);
    let mut malformed = tx.clone();
    malformed["meta"]["innerInstructions"] = Value::Null;
    assert!(native_credit(&malformed, "wallet").is_err());
    malformed = tx.clone();
    malformed["meta"]["innerInstructions"] = json!([
        tx["meta"]["innerInstructions"][0],
        tx["meta"]["innerInstructions"][0]
    ]);
    assert!(native_credit(&malformed, "wallet").is_err());
    assert!(native_credit(&tx, "Wallet").is_err());
    tx["meta"]["preTokenBalances"] = json!([token(WSOL, 1), token(WSOL, 1)]);
    assert!(native_credit(&tx, "wallet").is_err());
}

#[test]
fn token_credit_totals_distinct_accounts_without_counting_other_owners_or_mints() {
    let mut tx = transaction();
    let mut second = token("USDC", 70);
    second["accountIndex"] = json!(2);
    let mut other_owner = token("USDC", 999);
    other_owner["accountIndex"] = json!(3);
    other_owner["owner"] = json!("someone-else");
    let mut other_mint = token("USDT", 999);
    other_mint["accountIndex"] = json!(4);
    tx["meta"]["preTokenBalances"] = json!([token("USDC", 10)]);
    tx["meta"]["postTokenBalances"] = json!([token("USDC", 50), second, other_owner, other_mint]);
    assert_eq!(token_credit(&tx, "wallet", "USDC").unwrap(), 110);
    tx["meta"]["preTokenBalances"][0]["uiTokenAmount"]["amount"] = json!("200");
    assert_eq!(token_credit(&tx, "wallet", "USDC").unwrap(), 0);
}

#[test]
fn token_credit_rejects_incomplete_prior_identity_and_duplicate_or_invalid_accounts() {
    let mut tx = transaction();
    tx["meta"]["preTokenBalances"] = json!([token("USDC", 100)]);
    tx["meta"]["postTokenBalances"] = json!([token("USDC", 100)]);
    assert_eq!(token_credit(&tx, "wallet", "USDC").unwrap(), 0);
    for path in [
        "/meta/preTokenBalances/0/owner",
        "/meta/postTokenBalances/0/owner",
        "/meta/preTokenBalances/0/accountIndex",
    ] {
        let mut incomplete = tx.clone();
        *incomplete.pointer_mut(path).unwrap() = Value::Null;
        assert!(
            token_credit(&incomplete, "wallet", "USDC").is_err(),
            "{path}"
        );
    }
    let mut changed = tx.clone();
    changed["meta"]["preTokenBalances"][0]["mint"] = json!("USDT");
    assert!(token_credit(&changed, "wallet", "USDC").is_err());
    changed = tx.clone();
    changed["meta"]["postTokenBalances"][0]["owner"] = json!("someone-else");
    assert!(token_credit(&changed, "wallet", "USDC").is_err());
    changed = tx.clone();
    changed["meta"]["postTokenBalances"] = json!([token("USDC", 100), token("USDC", 100)]);
    assert!(token_credit(&changed, "wallet", "USDC").is_err());
    changed = tx.clone();
    changed["meta"]["postTokenBalances"][0]["accountIndex"] = json!(99);
    assert!(token_credit(&changed, "wallet", "USDC").is_err());
    changed = tx.clone();
    changed["meta"]["postTokenBalances"][0]["uiTokenAmount"]["amount"] =
        json!(u128::MAX.to_string());
    assert!(token_credit(&changed, "wallet", "USDC").is_err());
    tx["transaction"]["message"]["accountKeys"][2] = json!("account");
    assert!(token_credit(&tx, "wallet", "USDC").is_err());
}
