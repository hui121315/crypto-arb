use super::*;

pub(crate) fn parsed_finalized(
    cost: &StockChainCost,
    row: &StockChainSubmission,
    failed: bool,
) -> Result<Lookup, String> {
    let (id, value) = finalized(cost, failed);
    Ok(Lookup {
        receipt: receipt::read(cost, row, &id, &value)?,
        before: None,
    })
}

pub(crate) fn attach(cost: &mut StockChainCost, sponsored: bool) {
    let wallet = common::signing::ed25519_public_key(&[7; 32]).unwrap();
    cost.wallet_address = bs58::encode(&wallet).into_string();
    let mut keys = vec![];
    if sponsored {
        keys.push(
            common::signing::ed25519_public_key(&[9; 32])
                .unwrap()
                .to_vec(),
        );
    }
    keys.extend([wallet.to_vec(), vec![4; 32], vec![5; 32], vec![11; 32]]);
    let count = if sponsored { 2 } else { 1 };
    let mut bytes = vec![count];
    bytes.extend(vec![0; usize::from(count) * 64]);
    bytes.extend([0x80, count, 0, 1, keys.len() as u8]);
    for key in &keys {
        bytes.extend(key);
    }
    bytes.extend([42; 32]);
    bytes.extend([
        1,
        (keys.len() - 1) as u8,
        3,
        count - 1,
        count,
        count + 1,
        1,
        0,
        0,
    ]);
    let encoded = STANDARD.encode(bytes);
    cost.quote.router = if sponsored { "jupiterz" } else { "metis" }.into();
    cost.transaction_fingerprint = transaction::inspect(&encoded, &cost.wallet_address)
        .unwrap()
        .fingerprint;
    cost.transaction = Some(OnchainUnsignedTransaction::SolanaVersioned {
        transaction_base64: encoded,
        request_id: "original-stock-request".into(),
        router: cost.quote.router.clone(),
        mode: "ultra".into(),
        last_valid_block_height: Some(1000),
        expire_at_ms: cost.quote.expires_at_ms,
    });
    cost.network_fee_lamports = Some("7000".into());
    cost.wallet_debit_lamports = Some(if sponsored { "0" } else { "7000" }.into());
    cost.simulation_slot = Some(12);
}

pub(crate) fn signed(cost: &StockChainCost) -> Result<String, String> {
    let Some(OnchainUnsignedTransaction::SolanaVersioned {
        transaction_base64, ..
    }) = &cost.transaction
    else {
        panic!()
    };
    let mut bytes = STANDARD.decode(transaction_base64).unwrap();
    let (count, prefix) = crate::services::onchain_signer::decode_short_vec(&bytes, 0).unwrap();
    let offset = prefix + count * 64;
    let sig = common::signing::ed25519_sign_bytes(&[7; 32], &bytes[offset..]).unwrap();
    let i = transaction::inspect(transaction_base64, &cost.wallet_address)
        .unwrap()
        .wallet_index;
    bytes[prefix + i * 64..prefix + (i + 1) * 64].copy_from_slice(&sig);
    Ok(STANDARD.encode(bytes))
}

pub(crate) fn attach_variant(cost: &mut StockChainCost, salt: u8) {
    variant(cost, salt, false);
}

pub(crate) fn attach_sponsored_variant(cost: &mut StockChainCost, salt: u8) {
    variant(cost, salt, true);
}

fn variant(cost: &mut StockChainCost, salt: u8, sponsored: bool) {
    attach(cost, sponsored);
    let Some(OnchainUnsignedTransaction::SolanaVersioned {
        transaction_base64,
        request_id,
        ..
    }) = cost.transaction.as_mut()
    else {
        panic!()
    };
    let mut bytes = STANDARD.decode(&*transaction_base64).unwrap();
    let index = bytes.len() - 2;
    bytes[index] = salt;
    *transaction_base64 = STANDARD.encode(bytes);
    *request_id = format!("local-native-topup-{salt}");
    cost.transaction_fingerprint = transaction::inspect(transaction_base64, &cost.wallet_address)
        .unwrap()
        .fingerprint;
}

pub(crate) fn finalized_native(cost: &StockChainCost, failed: bool) -> (String, Value) {
    let (id, mut value) = finalized(cost, failed);
    for name in ["preTokenBalances", "postTokenBalances"] {
        value["meta"][name][1]["uiTokenAmount"]["decimals"] = 9.into();
        value["meta"][name][1]["uiTokenAmount"]["amount"] = "0".into();
    }
    if !failed {
        let output = cost.quote.output_raw.parse::<u64>().unwrap();
        value["meta"]["postBalances"][0] = (10_000_000 + output - 7000).into();
    }
    (id, value)
}

pub(crate) fn parsed_finalized_native(cost:&StockChainCost,row:&StockChainSubmission,failed:bool)->Result<Lookup,String>{
    let (id,value)=finalized_native(cost,failed);
    Ok(Lookup{receipt:receipt::read(cost,row,&id,&value)?,before:None})
}

#[test]
fn stock_native_receipt_requires_real_sol_not_wsol_and_preserves_failed_fees() {
    let mut cost = super::super::tests::cost();
    cost.quote.output_mint = STOCK_WRAPPED_SOL.into();
    cost.quote.output_raw = "21000".into();
    cost.quote.minimum_output_raw = "21000".into();
    cost.wallet_budget_lamports = Some("7000".into());
    attach_variant(&mut cost, 1);
    let intent = intent(&cost, &signed(&cost).unwrap(), 1000).unwrap();
    let (id, mut value) = finalized_native(&cost, false);
    let r = receipt::read(&cost, &intent, &id, &value).unwrap().unwrap();
    assert!(r.within_plan && r.succeeded);
    assert_eq!(r.wallet_native_change_lamports, "14000");
    value["meta"]["postBalances"][0] = 9_993_000.into();
    value["meta"]["postTokenBalances"][1]["uiTokenAmount"]["amount"] = "21000".into();
    assert!(
        !receipt::read(&cost, &intent, &id, &value)
            .unwrap()
            .unwrap()
            .within_plan
    );
    let (id, mut failed) = finalized_native(&cost, true);
    assert_eq!(
        receipt::read(&cost, &intent, &id, &failed)
            .unwrap()
            .unwrap()
            .wallet_native_change_lamports,
        "-7000"
    );
    failed["meta"]["postTokenBalances"][1]["uiTokenAmount"]["decimals"] = 6.into();
    assert!(receipt::read(&cost, &intent, &id, &failed).is_err());
}

pub(crate) fn finalized(cost: &StockChainCost, failed: bool) -> (String, Value) {
    let mut bytes = STANDARD.decode(signed(cost).unwrap()).unwrap();
    let (count, prefix) = crate::services::onchain_signer::decode_short_vec(&bytes, 0).unwrap();
    if count == 2 {
        let sig =
            common::signing::ed25519_sign_bytes(&[9; 32], &bytes[prefix + count * 64..]).unwrap();
        bytes[prefix..prefix + 64].copy_from_slice(&sig);
    }
    let id = bs58::encode(&bytes[prefix..prefix + 64]).into_string();
    let token = |index, mint: &str, raw: &str| {
        json!({"accountIndex":index,"mint":mint,"owner":cost.wallet_address,
        "programId":"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
        "uiTokenAmount":{"amount":raw,"decimals":if mint==comparison::SOLANA_USDC {6} else {cost.mint.decimals}}})
    };
    let before = vec![
        token(count, &cost.quote.input_mint, &cost.quote.input_raw),
        token(count + 1, &cost.quote.output_mint, "0"),
    ];
    let after = if failed {
        before.clone()
    } else {
        vec![
            token(count, &cost.quote.input_mint, "0"),
            token(count + 1, &cost.quote.output_mint, &cost.quote.output_raw),
        ]
    };
    let pre = vec![10000000u64; count + 3];
    let mut post = pre.clone();
    post[0] -= 7000;
    (
        id,
        json!({"slot":13,"transaction":[STANDARD.encode(bytes),"base64"],"meta":{
        "err":if failed {json!({"InstructionError":[0,{"Custom":6001}]})} else {Value::Null},"fee":7000,
        "preBalances":pre,"postBalances":post,"loadedAddresses":{"writable":[],"readonly":[]},
        "preTokenBalances":before,"postTokenBalances":after}}),
    )
}

#[test]
fn stock_chain_receipt_matches_message_wallet_signature_and_real_native_changes() {
    for sponsored in [false, true] {
        let mut cost = super::super::tests::cost();
        attach(&mut cost, sponsored);
        let signed = signed(&cost).unwrap();
        let intent = intent(&cost, &signed, 1000).unwrap();
        assert_eq!(intent.transaction_id.is_none(), sponsored);
        let (id, value) = finalized(&cost, false);
        let r = receipt::read(&cost, &intent, &id, &value).unwrap().unwrap();
        assert!(r.succeeded && r.within_plan);
        assert_eq!(r.network_fee_lamports, "7000");
        let mut recorded = intent.clone();
        recorded.transaction_id = Some(id.clone());
        recorded.receipt = Some(r.clone());
        validate_record(&cost, &recorded).unwrap();
        recorded.receipt.as_mut().unwrap().fee_payer = bs58::encode([99u8; 32]).into_string();
        assert!(
            validate_record(&cost, &recorded).is_err(),
            "stored payer must match the original signed message"
        );
        assert_eq!(
            r.wallet_native_change_lamports,
            if sponsored { "0" } else { "-7000" }
        );
        assert!(r.asset_changes.iter().any(|a| a.raw_change == "-10000000"));
        let (id, failed) = finalized(&cost, true);
        let r = receipt::read(&cost, &intent, &id, &failed)
            .unwrap()
            .unwrap();
        assert!(!r.succeeded && !r.within_plan);
        assert_eq!(r.network_fee_lamports, "7000");
        assert!(r.asset_changes.iter().all(|a| a.raw_change == "0"));
        let mut tampered = STANDARD.decode(&signed).unwrap();
        *tampered.last_mut().unwrap() = 1;
        assert!(signed_identity(&cost, &STANDARD.encode(tampered)).is_err());
        let wrong_id = bs58::encode([88; 64]).into_string();
        assert!(receipt::read(&cost, &intent, &wrong_id, &value)
            .unwrap()
            .is_none());
        let mut no_fee = value.clone();
        no_fee["meta"]["fee"] = Value::Null;
        assert!(receipt::read(&cost, &intent, &id, &no_fee).is_err());
        let mut less_output = value.clone();
        less_output["meta"]["postTokenBalances"][1]["uiTokenAmount"]["amount"] = "1".into();
        let r = receipt::read(&cost, &intent, &id, &less_output)
            .unwrap()
            .unwrap();
        assert!(r.succeeded && !r.within_plan);
    }
    assert!(!valid_signature(&bs58::encode([0; 64]).into_string()));
}
