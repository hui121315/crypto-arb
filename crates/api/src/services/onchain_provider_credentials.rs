use shared_types::{
    OnchainProviderCredentialClearRequest, OnchainProviderCredentialMutationResponse,
    OnchainProviderCredentialStatus, OnchainProviderCredentialUpdateRequest,
    OnchainProviderCredentialsResponse, VenueCredentialField, VenueCredentialFieldSource,
    VenueCredentialValue,
};
use thiserror::Error;

use super::venue_credentials::{self, CredentialUpdateError};
use super::backpack_stocks::credentials as stock_credentials;

type SecretUpdate = (String, String);
type SecretUpdates = (Vec<SecretUpdate>, Vec<String>);
type SecretFields = (Vec<String>, Vec<String>);

const ZEROEX_DOCS: &str = "https://docs.0x.org/api-reference/api-overview";
const OKX_DOCS: &str = "https://web3.okx.com/id/onchainos/dev-docs/home/api-access-and-usage";
const JUPITER_DOCS: &str = "https://developers.jup.ag/docs/portal/rate-limits";
const LIFI_DOCS: &str = "https://docs.li.fi/api-reference/introduction";
const SOLANA_SIGNER_DOCS: &str = "https://solana.com/docs/core/transactions/transaction-structure";
const EVM_SIGNER_DOCS: &str = "https://ethereum.org/developers/docs/transactions/";

#[derive(Clone, Copy)]
struct FieldSpec {
    key: &'static str,
    label: &'static str,
    env_key: &'static str,
    required: bool,
    secret: bool,
}

#[derive(Clone, Copy)]
struct ProviderSpec {
    id: &'static str,
    label: &'static str,
    official_docs_url: &'static str,
    note: &'static str,
    fields: &'static [FieldSpec],
}

const ZEROEX_FIELDS: &[FieldSpec] = &[FieldSpec {
    key: "api_key",
    label: "0x API Key",
    env_key: "ZEROX_API_KEY",
    required: true,
    secret: true,
}];

const JUPITER_FIELDS: &[FieldSpec] = &[FieldSpec {
    key: "api_key",
    label: "Jupiter API Key",
    env_key: "JUPITER_API_KEY",
    required: true,
    secret: true,
}];

const LIFI_FIELDS: &[FieldSpec] = &[FieldSpec {
    key: "api_key",
    label: "LI.FI API Key（可选）",
    env_key: "LIFI_API_KEY",
    required: false,
    secret: true,
}];

const OKX_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "api_key",
        label: "OKX DEX API Key",
        env_key: "OKX_DEX_API_KEY",
        required: true,
        secret: true,
    },
    FieldSpec {
        key: "secret_key",
        label: "OKX DEX Secret Key",
        env_key: "OKX_DEX_SECRET_KEY",
        required: true,
        secret: true,
    },
    FieldSpec {
        key: "passphrase",
        label: "OKX DEX Passphrase",
        env_key: "OKX_DEX_PASSPHRASE",
        required: true,
        secret: true,
    },
];

const SOLANA_SIGNER_FIELDS: &[FieldSpec] = &[FieldSpec {
    key: "private_key",
    label: "Solana 钱包私钥",
    env_key: super::onchain_signer::SOLANA_PRIVATE_KEY_ENV,
    required: true,
    secret: true,
}];

const EVM_SIGNER_FIELDS: &[FieldSpec] = &[FieldSpec {
    key: "private_key",
    label: "EVM 钱包私钥",
    env_key: super::onchain_signer::EVM_PRIVATE_KEY_ENV,
    required: true,
    secret: true,
}];

const PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        id: stock_credentials::PROVIDER,
        label: "Backpack 股票 RFQ",
        official_docs_url: "https://docs.backpack.exchange/",
        note: "同时保存同一组 Base64 API 公钥与 32 字节 Secret seed；保存仅核对密钥配对，不证明远程权限，也不下单。",
        fields: &[
            FieldSpec {key:"api_key",label:"Backpack API Key（Base64 公钥）",env_key:stock_credentials::API_KEY,required:true,secret:true},
            FieldSpec {key:"secret_key",label:"Backpack Secret Key（Base64 seed）",env_key:stock_credentials::SECRET_KEY,required:true,secret:true},
        ],
    },
    ProviderSpec {
        id: "jupiter_swap_v2_keyed",
        label: "Jupiter API Key",
        official_docs_url: JUPITER_DOCS,
        note: "仅在选择 Jupiter API Key 加速路由时使用；保存后按官方 Free 1 RPS 约 2.25 秒一轮双向询价。",
        fields: JUPITER_FIELDS,
    },
    ProviderSpec {
        id: "zeroex_swap_v2",
        label: "0x Swap API V2",
        official_docs_url: ZEROEX_DOCS,
        note: "0x 官方要求每次 API 请求携带 0x-api-key；保存状态不等于上游已验证。",
        fields: ZEROEX_FIELDS,
    },
    ProviderSpec {
        id: "okx_dex_v6",
        label: "OKX DEX Aggregator V6",
        official_docs_url: OKX_DOCS,
        note: "OKX DEX 签名需要 API Key、Secret Key 与 Passphrase 三项同时配置。",
        fields: OKX_FIELDS,
    },
    ProviderSpec {
        id: "lifi",
        label: "LI.FI 跨链路由",
        official_docs_url: LIFI_DOCS,
        note: "LI.FI 无 Key 也可报价；保存 API Key 仅用于提高服务端限频额度，页面永不暴露。",
        fields: LIFI_FIELDS,
    },
    ProviderSpec {
        id: super::onchain_signer::SOLANA_SIGNER_ID,
        label: "Solana 本地签名器",
        official_docs_url: SOLANA_SIGNER_DOCS,
        note: "支持 base58 或 Solana CLI JSON keypair；仅保存到后端安全存储，页面永不回填。",
        fields: SOLANA_SIGNER_FIELDS,
    },
    ProviderSpec {
        id: super::onchain_signer::EVM_SIGNER_ID,
        label: "EVM 本地签名器",
        official_docs_url: EVM_SIGNER_DOCS,
        note: "支持 32 字节 hex 私钥；保存和签名前都会核对派生地址与当前钱包。",
        fields: EVM_SIGNER_FIELDS,
    },
];

pub(crate) fn status() -> OnchainProviderCredentialsResponse {
    OnchainProviderCredentialsResponse {
        providers: PROVIDERS.iter().map(provider_status).collect(),
        secret_storage: venue_credentials::secret_storage_status(),
    }
}

pub(crate) async fn update(
    request: OnchainProviderCredentialUpdateRequest,
) -> Result<OnchainProviderCredentialMutationResponse, ProviderCredentialError> {
    let spec = find_provider(&request.provider)?;
    let (updates, affected_fields) = credential_updates(spec, request.fields)?;
    venue_credentials::persist_secrets(&updates).await?;
    Ok(mutation_response(
        spec,
        affected_fields,
        format!(
            "已保存 {} 个 {} 凭证字段；后续报价请求将使用新的安全存储值。",
            updates.len(),
            spec.label
        ),
    ))
}

pub(crate) async fn clear(
    request: OnchainProviderCredentialClearRequest,
) -> Result<OnchainProviderCredentialMutationResponse, ProviderCredentialError> {
    let spec = find_provider(&request.provider)?;
    let (env_keys, affected_fields) = clear_fields(spec, &request.fields)?;
    venue_credentials::clear_secrets(&env_keys).await?;
    Ok(mutation_response(
        spec,
        affected_fields,
        format!("已清除 {} 的选定凭证字段。", spec.label),
    ))
}

fn provider_status(spec: &ProviderSpec) -> OnchainProviderCredentialStatus {
    let fields = spec
        .fields
        .iter()
        .map(|field| {
            let source = venue_credentials::secret_source(field.env_key);
            VenueCredentialField {
                key: field.key.to_owned(),
                label: field.label.to_owned(),
                env_key: field.env_key.to_owned(),
                configured: source != VenueCredentialFieldSource::Missing,
                secret: field.secret,
                required: field.required,
                source,
            }
        })
        .collect::<Vec<_>>();
    let missing_fields = fields
        .iter()
        .filter(|field| field.required && !field.configured)
        .map(|field| field.key.clone())
        .collect::<Vec<_>>();
    let configured_count = fields.iter().filter(|field| field.configured).count();
    OnchainProviderCredentialStatus {
        provider: spec.id.to_owned(),
        label: spec.label.to_owned(),
        official_docs_url: spec.official_docs_url.to_owned(),
        field_count: fields.len(),
        fields,
        missing_fields: missing_fields.clone(),
        configured_count,
        ready: missing_fields.is_empty(),
        note: spec.note.to_owned(),
    }
}

fn mutation_response(
    spec: &ProviderSpec,
    affected_fields: Vec<String>,
    message: String,
) -> OnchainProviderCredentialMutationResponse {
    let current = provider_status(spec);
    OnchainProviderCredentialMutationResponse {
        provider: current.provider,
        label: current.label,
        configured_count: current.configured_count,
        field_count: current.field_count,
        affected_fields,
        missing_fields: current.missing_fields,
        message,
        secret_storage: venue_credentials::secret_storage_status(),
        action_run_id: None,
        request_id: None,
    }
}

fn credential_updates(
    spec: &ProviderSpec,
    fields: Vec<VenueCredentialValue>,
) -> Result<SecretUpdates, ProviderCredentialError> {
    let mut updates = Vec::with_capacity(fields.len());
    let mut affected_fields = Vec::with_capacity(fields.len());
    for field in fields {
        let value = field.value.trim();
        if value.is_empty() {
            continue;
        }
        let field_spec = find_field(spec, &field.key)?;
        if affected_fields.iter().any(|key| key == field_spec.key) {
            return Err(ProviderCredentialError::DuplicateField {
                provider: spec.id.to_owned(),
                field: field_spec.key.to_owned(),
            });
        }
        super::onchain_signer::validate_provider_secret(spec.id, value).map_err(|problem| {
            ProviderCredentialError::InvalidCredential {
                provider: spec.id.to_owned(),
                problem,
            }
        })?;
        updates.push((field_spec.env_key.to_owned(), value.to_owned()));
        affected_fields.push(field_spec.key.to_owned());
    }
    if updates.is_empty() {
        return Err(ProviderCredentialError::NoFields);
    }
    if spec.id==stock_credentials::PROVIDER {
        stock_credentials::validate_updates(&updates).map_err(|problem|ProviderCredentialError::InvalidCredential{provider:spec.id.into(),problem})?;
    }
    Ok((updates, affected_fields))
}

fn clear_fields(
    spec: &ProviderSpec,
    requested: &[String],
) -> Result<SecretFields, ProviderCredentialError> {
    let fields = if requested.is_empty() {
        spec.fields
            .iter()
            .map(|field| field.key)
            .collect::<Vec<_>>()
    } else {
        requested.iter().map(String::as_str).collect::<Vec<_>>()
    };
    let mut env_keys = Vec::with_capacity(fields.len());
    let mut affected_fields = Vec::with_capacity(fields.len());
    for key in fields {
        let field = find_field(spec, key)?;
        if affected_fields.iter().any(|existing| existing == field.key) {
            continue;
        }
        env_keys.push(field.env_key.to_owned());
        affected_fields.push(field.key.to_owned());
    }
    if env_keys.is_empty() {
        return Err(ProviderCredentialError::NoFields);
    }
    Ok((env_keys, affected_fields))
}

fn find_provider(provider: &str) -> Result<&'static ProviderSpec, ProviderCredentialError> {
    PROVIDERS
        .iter()
        .find(|spec| spec.id.eq_ignore_ascii_case(provider.trim()))
        .ok_or_else(|| ProviderCredentialError::UnknownProvider(provider.to_owned()))
}

fn find_field<'a>(
    spec: &'a ProviderSpec,
    key: &str,
) -> Result<&'a FieldSpec, ProviderCredentialError> {
    spec.fields
        .iter()
        .find(|field| field.key == key.trim())
        .ok_or_else(|| ProviderCredentialError::UnknownField {
            provider: spec.id.to_owned(),
            field: key.to_owned(),
        })
}

#[derive(Debug, Error)]
pub(crate) enum ProviderCredentialError {
    #[error("unknown on-chain quote provider: {0}")]
    UnknownProvider(String),
    #[error("unknown credential field {field} for on-chain provider {provider}")]
    UnknownField { provider: String, field: String },
    #[error("duplicate credential field {field} for on-chain provider {provider}")]
    DuplicateField { provider: String, field: String },
    #[error("at least one non-empty provider credential field is required")]
    NoFields,
    #[error("invalid credential for on-chain provider {provider}: {problem}")]
    InvalidCredential { provider: String, problem: String },
    #[error(transparent)]
    Storage(#[from] CredentialUpdateError),
}

#[cfg(test)]
#[path = "onchain_provider_credentials/tests.rs"]
mod tests;
