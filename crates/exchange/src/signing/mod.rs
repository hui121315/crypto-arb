//! 各交易所的具体签名规则。
//!
//! 共享底层原语（HMAC、Base64、Ed25519）来自 [`common::signing`]，本目录把它们
//! 组装成各家所需的具体签名格式（query 字符串、HTTP 头部布局、prehash 拼接顺序等）。

pub mod binance;
pub mod bitget;
pub mod bybit;
pub mod gate;
pub mod hyperliquid;
pub mod kraken;
pub mod kucoin;
pub mod okx;
