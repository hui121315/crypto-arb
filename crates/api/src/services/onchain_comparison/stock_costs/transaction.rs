use base64::{engine::general_purpose::STANDARD, Engine};

pub(super) fn artifact(
    value: &serde_json::Value,
    quote: &shared_types::stocks::StockDexQuote,
) -> Result<shared_types::OnchainUnsignedTransaction, String> {
    let request_id = value["requestId"].as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256).ok_or("链上构建缺少原始请求编号")?;
    let mode = value["mode"].as_str()
        .filter(|s| ["ultra", "manual"].contains(s)).ok_or("链上构建模式未知")?;
    let height = value.get("lastValidBlockHeight").filter(|v| !v.is_null())
        .map(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()).filter(|n| *n > 0)
            .ok_or("链上构建区块有效期无效")).transpose()?;
    Ok(shared_types::OnchainUnsignedTransaction::SolanaVersioned {
        transaction_base64: value["transaction"].as_str().ok_or("链上构建没有交易")?.into(),
        request_id: request_id.into(), router: quote.router.clone(), mode: mode.into(),
        last_valid_block_height: height, expire_at_ms: quote.expires_at_ms,
    })
}

pub(super) struct Inspection {
    pub message: String,
    pub fingerprint: String,
    pub wallet_index: usize,
    pub static_accounts: usize,
    pub fee_payer: String,
    pub keys: Vec<String>,
    pub loaded_counts: [usize; 2],
    pub instructions: Vec<Instruction>,
}

pub(super) struct Instruction {
    pub program: usize,
    pub accounts: Vec<usize>,
    pub data: Vec<u8>,
}

pub(super) fn inspect(encoded: &str, wallet: &str) -> Result<Inspection, String> {
    if encoded.len() > 1644 {
        return Err("Solana 交易超出包长度限制".into());
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "Solana 交易 base64 无效")?;
    if bytes.len() > 1232 {
        return Err("Solana 交易超过 1232 字节".into());
    }
    let decode = crate::services::onchain_signer::decode_short_vec;
    let (signatures, prefix) = decode(&bytes, 0)?;
    if signatures == 0 || signatures > 19 {
        return Err("交易签名槽数量无效".into());
    }
    let offset = prefix + signatures * 64;
    let message = bytes
        .get(offset..)
        .filter(|m| !m.is_empty())
        .ok_or("交易消息缺失")?;
    let header = if message[0] & 0x80 == 0 {
        0
    } else if message[0] == 0x80 {
        1
    } else {
        return Err("不支持的 Solana 消息版本".into());
    };
    let h = message
        .get(header..header + 3)
        .ok_or("交易 header 不完整")?;
    let (count, len) = decode(message, header + 3)?;
    if usize::from(h[0]) != signatures
        || count < signatures
        || count > 256
        || usize::from(h[1]) >= signatures
        || usize::from(h[2]) > count - signatures
    {
        return Err("交易签名与账户数量不一致".into());
    }
    let start = header + 3 + len;
    let keys = message
        .get(start..start + count * 32)
        .ok_or("交易账户列表不完整")?;
    if message.len() < start + count * 32 + 33 {
        return Err("交易 blockhash 或指令缺失".into());
    }
    let expected = bs58::decode(wallet)
        .into_vec()
        .map_err(|_| "钱包地址无效")?;
    let wallet_index = keys
        .chunks_exact(32)
        .take(signatures)
        .position(|k| k == expected)
        .ok_or("未签名交易的签名者不包含所选钱包")?;
    if bytes[prefix + wallet_index * 64..prefix + (wallet_index + 1) * 64]
        .iter()
        .any(|n| *n != 0)
    {
        return Err("费用试算拒绝已包含钱包签名的交易".into());
    }
    let keys: Vec<_> = keys
        .chunks_exact(32)
        .map(|k| bs58::encode(k).into_string())
        .collect();
    let mut reader = Reader {
        bytes: message,
        offset: start + count * 32 + 32,
    };
    let instruction_count = reader.count()?;
    let mut instructions = Vec::new();
    for _ in 0..instruction_count {
        let program = usize::from(reader.take(1)?[0]);
        let n = reader.count()?;
        let accounts = reader.take(n)?.iter().map(|n| usize::from(*n)).collect();
        let n = reader.count()?;
        instructions.push(Instruction {
            program,
            accounts,
            data: reader.take(n)?.to_vec(),
        });
    }
    let mut loaded_counts = [0; 2];
    if header == 1 {
        for _ in 0..reader.count()? {
            reader.take(32)?;
            for loaded in &mut loaded_counts {
                let n = reader.count()?;
                reader.take(n)?;
                *loaded += n;
            }
        }
    }
    let total = count + loaded_counts.iter().sum::<usize>();
    if reader.offset != message.len()
        || total > 256
        || instructions.is_empty()
        || instructions
            .iter()
            .any(|i| i.program >= count || i.accounts.iter().any(|n| *n >= total))
    {
        return Err("交易指令、查找表或账户索引不完整".into());
    }
    Ok(Inspection {
        message: STANDARD.encode(message),
        fingerprint: common::signing::hmac_sha256_hex(b"stock-chain-cost-v1", &bytes),
        wallet_index,
        static_accounts: count,
        fee_payer: keys[0].clone(),
        keys,
        loaded_counts,
        instructions,
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.offset.checked_add(n).ok_or("交易指令长度溢出")?;
        let result = self.bytes.get(self.offset..end).ok_or("交易指令不完整")?;
        self.offset = end;
        Ok(result)
    }

    fn count(&mut self) -> Result<usize, String> {
        let (n, size) = crate::services::onchain_signer::decode_short_vec(self.bytes, self.offset)?;
        if n > 65535 || (size > 1 && n < 1 << (7 * (size - 1))) {
            return Err("交易 compact-u16 非规范编码".into());
        }
        self.offset += size;
        Ok(n)
    }
}
