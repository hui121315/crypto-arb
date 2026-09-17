use shared_types::VenueId;

pub(super) struct FieldSpec {
    pub(super) key: &'static str,
    pub(super) label: &'static str,
    pub(super) env_key: &'static str,
    pub(super) secret: bool,
    pub(super) required: bool,
}

pub(super) struct VenueSpec {
    pub(super) id: VenueId,
    pub(super) venue: &'static str,
    pub(super) label: &'static str,
    pub(super) fields: &'static [FieldSpec],
    pub(super) private_read: bool,
    pub(super) testnet_write: bool,
    pub(super) live_write: bool,
    pub(super) note: &'static str,
}

const BINANCE_FIELDS: &[FieldSpec] = &[
    field("api_key", "API Key", "BINANCE_API_KEY", true),
    field("api_secret", "API Secret", "BINANCE_API_SECRET", true),
];
const OKX_FIELDS: &[FieldSpec] = &[
    field("api_key", "API Key", "OKX_API_KEY", true),
    field("api_secret", "API Secret", "OKX_API_SECRET", true),
    field("passphrase", "Passphrase", "OKX_PASSPHRASE", true),
    field("live_key", "Live Key", "OKX_LIVE_API_KEY", true),
    field("live_secret", "Live Secret", "OKX_LIVE_API_SECRET", true),
    field(
        "live_passphrase",
        "Live Passphrase",
        "OKX_LIVE_PASSPHRASE",
        true,
    ),
];
const BYBIT_FIELDS: &[FieldSpec] = &[
    field("api_key", "API Key", "BYBIT_API_KEY", true),
    field("api_secret", "API Secret", "BYBIT_API_SECRET", true),
];
const BITGET_FIELDS: &[FieldSpec] = &[
    field("api_key", "API Key", "BITGET_API_KEY", true),
    field("api_secret", "API Secret", "BITGET_API_SECRET", true),
    field("passphrase", "Passphrase", "BITGET_PASSPHRASE", true),
];
const GATE_FIELDS: &[FieldSpec] = &[
    field("api_key", "API Key", "GATE_API_KEY", true),
    field("api_secret", "API Secret", "GATE_API_SECRET", true),
];
const GATE_CROSSEX_FIELDS: &[FieldSpec] = &[
    field("api_key", "CrossEx API Key", "GATE_CROSSEX_API_KEY", true),
    field(
        "api_secret",
        "CrossEx API Secret",
        "GATE_CROSSEX_API_SECRET",
        true,
    ),
];
const KRAKEN_FIELDS: &[FieldSpec] = &[
    optional_field("spot_api_key", "Spot API Key", "KRAKEN_SPOT_API_KEY", true),
    optional_field(
        "spot_api_secret",
        "Spot API Secret",
        "KRAKEN_SPOT_API_SECRET",
        true,
    ),
    optional_field(
        "futures_api_key",
        "Futures API Key",
        "KRAKEN_FUTURES_API_KEY",
        true,
    ),
    optional_field(
        "futures_api_secret",
        "Futures API Secret",
        "KRAKEN_FUTURES_API_SECRET",
        true,
    ),
];
const KUCOIN_FIELDS: &[FieldSpec] = &[
    field("api_key", "API Key", "KUCOIN_API_KEY", true),
    field("api_secret", "API Secret", "KUCOIN_API_SECRET", true),
    field("passphrase", "Passphrase", "KUCOIN_PASSPHRASE", true),
];
const HYPERLIQUID_FIELDS: &[FieldSpec] = &[
    field(
        "account_address",
        "余额读取地址（主账户 / 子账户）",
        "HYPERLIQUID_ACCOUNT_ADDRESS",
        false,
    ),
    field(
        "private_key",
        "已授权 API / Agent 钱包私钥",
        "HYPERLIQUID_PRIVATE_KEY",
        true,
    ),
    optional_field(
        "vault_address",
        "Vault 执行地址（可选）",
        "HYPERLIQUID_VAULT_ADDRESS",
        false,
    ),
];

pub(super) const SPECS: &[VenueSpec] = &[
    spec(
        VenueId::Binance,
        "binance",
        "Binance",
        BINANCE_FIELDS,
        true,
        "USDT-M live guarded",
    ),
    spec(VenueId::Okx, "okx", "OKX", OKX_FIELDS, true, "Live guarded"),
    spec(
        VenueId::Bybit,
        "bybit",
        "Bybit",
        BYBIT_FIELDS,
        true,
        "linear live guarded",
    ),
    spec(
        VenueId::Bitget,
        "bitget",
        "Bitget",
        BITGET_FIELDS,
        true,
        "USDT-FUTURES live guarded",
    ),
    spec(
        VenueId::Gate,
        "gate",
        "Gate",
        GATE_FIELDS,
        true,
        "USDT futures live guarded",
    ),
    spec(
        VenueId::GateCrossEx,
        "gate_crossex",
        "Gate CrossEx",
        GATE_CROSSEX_FIELDS,
        true,
        "CrossEx private WS account and trading; bounded REST query bootstrap",
    ),
    spec(
        VenueId::Kucoin,
        "kucoin",
        "KuCoin",
        KUCOIN_FIELDS,
        true,
        "Futures live guarded",
    ),
    spec(
        VenueId::Hyperliquid,
        "hyperliquid",
        "Hyperliquid",
        HYPERLIQUID_FIELDS,
        true,
        "account address for reads, agent key for WS post/action",
    ),
    spec(
        VenueId::Kraken,
        "kraken",
        "Kraken",
        KRAKEN_FIELDS,
        true,
        "Spot and Futures credentials are independent; configure at least one complete pair",
    ),
];

const fn field(
    key: &'static str,
    label: &'static str,
    env_key: &'static str,
    secret: bool,
) -> FieldSpec {
    FieldSpec {
        key,
        label,
        env_key,
        secret,
        required: true,
    }
}

const fn optional_field(
    key: &'static str,
    label: &'static str,
    env_key: &'static str,
    secret: bool,
) -> FieldSpec {
    FieldSpec {
        key,
        label,
        env_key,
        secret,
        required: false,
    }
}

const fn spec(
    id: VenueId,
    venue: &'static str,
    label: &'static str,
    fields: &'static [FieldSpec],
    live_write: bool,
    note: &'static str,
) -> VenueSpec {
    VenueSpec {
        id,
        venue,
        label,
        fields,
        private_read: true,
        testnet_write: false,
        live_write,
        note,
    }
}
