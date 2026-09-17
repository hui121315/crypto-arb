//! Kraken Derivatives signed REST writes and bounded reconciliation reads.

use super::kraken_config::KrakenFuturesCredentials;
use super::kraken_futures_private_data::{parse_order, parse_position};
use super::kraken_symbols::{canonical_asset, futures_symbol};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::signing::kraken::futures_rest_sign;
use reqwest::{header::CONTENT_TYPE, Method, RequestBuilder};
use serde_json::Value;
use shared_types::{
    CancelOrderRequest, LiveOrderState, OrderAck, OrderInfo, OrderIntent, OrderSide, OrderStatus,
    OrderType, PositionInfo, TimeInForce, VenueBalanceInfo, VenueOrderIdentityUpdate,
};
use std::sync::atomic::{AtomicU64, Ordering};

const SEND_ORDER_PATH: &str = "/derivatives/api/v3/sendorder";
const CANCEL_ORDER_PATH: &str = "/derivatives/api/v3/cancelorder";
const OPEN_ORDERS_PATH: &str = "/derivatives/api/v3/openorders";
const OPEN_POSITIONS_PATH: &str = "/derivatives/api/v3/openpositions";
const ACCOUNTS_PATH: &str = "/derivatives/api/v3/accounts";
const ORDER_STATUS_PATH: &str = "/derivatives/api/v3/orders/status";

pub(super) async fn place_order(
    http: &HttpClient,
    base_url: &str,
    credentials: &KrakenFuturesCredentials,
    intent: &OrderIntent,
    venue_client_id: &str,
) -> ExchangeResult<OrderAck> {
    let parameters = compile_order(intent, venue_client_id)?;
    let body = request_text(
        http,
        base_url,
        credentials,
        RequestSpec::write(SEND_ORDER_PATH, parameters),
    )
    .await?;
    let value = parse_success(&body, "sendorder")?;
    let status = value
        .pointer("/sendStatus/status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if !matches!(status, "placed" | "partiallyFilled" | "filled") {
        return Err(ExchangeError::Api {
            exchange: "kraken".to_owned(),
            code: status.to_owned(),
            message: format!("Kraken Futures sendorder rejected: {status}"),
        });
    }
    let exchange_order_id = value
        .pointer("/sendStatus/order_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ExchangeError::Parse("kraken sendStatus.order_id missing".to_owned()))?
        .to_owned();
    Ok(OrderAck {
        internal_order_id: intent.id.clone(),
        exchange_order_id: Some(exchange_order_id.clone()),
        client_order_id: intent.client_order_id.clone(),
        identity_update: VenueOrderIdentityUpdate::from_ids(
            intent.client_order_id.clone(),
            venue_client_id.to_owned(),
            Some(exchange_order_id),
        ),
        state: match status {
            "filled" => LiveOrderState::Filled,
            "partiallyFilled" => LiveOrderState::PartiallyFilled,
            _ => LiveOrderState::Submitted,
        },
        accepted_at_ms: common::time::now_ms(),
        message: Some(format!("Kraken Futures REST sendorder {status}")),
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    })
}

pub(super) async fn cancel_order(
    http: &HttpClient,
    base_url: &str,
    credentials: &KrakenFuturesCredentials,
    request: &CancelOrderRequest,
    venue_client_id: &str,
) -> ExchangeResult<OrderAck> {
    let parameters = if let Some(order_id) = request.exchange_order_id.as_ref() {
        vec![("order_id".to_owned(), order_id.clone())]
    } else {
        vec![("cliOrdId".to_owned(), venue_client_id.to_owned())]
    };
    let body = request_text(
        http,
        base_url,
        credentials,
        RequestSpec::write(CANCEL_ORDER_PATH, parameters),
    )
    .await?;
    let value = parse_success(&body, "cancelorder")?;
    let status = value
        .pointer("/cancelStatus/status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if !matches!(status, "cancelled" | "filled") {
        return Err(ExchangeError::Api {
            exchange: "kraken".to_owned(),
            code: status.to_owned(),
            message: format!("Kraken Futures cancelorder failed: {status}"),
        });
    }
    let exchange_order_id = request.exchange_order_id.clone().or_else(|| {
        value
            .pointer("/cancelStatus/order_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    Ok(OrderAck {
        internal_order_id: request.internal_order_id.clone(),
        exchange_order_id: exchange_order_id.clone(),
        client_order_id: request.client_order_id.clone(),
        identity_update: VenueOrderIdentityUpdate::from_ids(
            request.client_order_id.clone(),
            venue_client_id.to_owned(),
            exchange_order_id,
        ),
        state: if status == "filled" {
            LiveOrderState::Filled
        } else {
            LiveOrderState::CancelRequested
        },
        accepted_at_ms: common::time::now_ms(),
        message: Some(format!("Kraken Futures REST cancelorder {status}")),
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    })
}

pub(super) async fn fetch_open_orders(
    http: &HttpClient,
    base_url: &str,
    credentials: &KrakenFuturesCredentials,
) -> ExchangeResult<Vec<OrderInfo>> {
    let body = request_text(
        http,
        base_url,
        credentials,
        RequestSpec::read(OPEN_ORDERS_PATH),
    )
    .await?;
    let value = parse_success(&body, "openorders")?;
    value
        .get("openOrders")
        .and_then(Value::as_array)
        .ok_or_else(|| ExchangeError::Parse("kraken openOrders missing".to_owned()))?
        .iter()
        .map(parse_order)
        .collect()
}

pub(super) async fn fetch_positions(
    http: &HttpClient,
    base_url: &str,
    credentials: &KrakenFuturesCredentials,
) -> ExchangeResult<Vec<PositionInfo>> {
    let body = request_text(
        http,
        base_url,
        credentials,
        RequestSpec::read(OPEN_POSITIONS_PATH),
    )
    .await?;
    let value = parse_success(&body, "openpositions")?;
    value
        .get("openPositions")
        .and_then(Value::as_array)
        .ok_or_else(|| ExchangeError::Parse("kraken openPositions missing".to_owned()))?
        .iter()
        .map(parse_position)
        .collect()
}

pub(super) async fn fetch_balances(
    http: &HttpClient,
    base_url: &str,
    credentials: &KrakenFuturesCredentials,
) -> ExchangeResult<Vec<VenueBalanceInfo>> {
    let body = request_text(
        http,
        base_url,
        credentials,
        RequestSpec::read(ACCOUNTS_PATH),
    )
    .await?;
    parse_account_balances(&parse_success(&body, "accounts")?)
}

pub(super) async fn fetch_order(
    http: &HttpClient,
    base_url: &str,
    credentials: &KrakenFuturesCredentials,
    order_id: Option<&str>,
    client_order_id: Option<&str>,
) -> ExchangeResult<Option<OrderInfo>> {
    let (name, value) = if let Some(order_id) = order_id {
        ("orderIds", order_id)
    } else if let Some(client_order_id) = client_order_id {
        ("cliOrdIds", client_order_id)
    } else {
        return Ok(None);
    };
    let body = request_text(
        http,
        base_url,
        credentials,
        RequestSpec::retrying_post(ORDER_STATUS_PATH, vec![(name.to_owned(), value.to_owned())]),
    )
    .await?;
    let value = parse_success(&body, "orders/status")?;
    let Some(row) = value
        .get("orders")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
    else {
        return Ok(None);
    };
    let mut order = parse_order(row.get("order").unwrap_or(row))?;
    if let Some(status) = row.get("status").and_then(Value::as_str) {
        order.status = status_from_text(status);
    }
    Ok(Some(order))
}

fn compile_order(
    intent: &OrderIntent,
    venue_client_id: &str,
) -> ExchangeResult<Vec<(String, String)>> {
    if !intent.quantity.is_finite() || intent.quantity <= 0.0 {
        return Err(ExchangeError::Parse(
            "Kraken Futures size must be positive".to_owned(),
        ));
    }
    let order_type = match (intent.order_type, intent.time_in_force, intent.post_only) {
        (OrderType::Market, _, _) => "mkt",
        (_, TimeInForce::Ioc, _) => "ioc",
        (_, TimeInForce::Fok, _) => "fok",
        (_, TimeInForce::Gtx, _) | (OrderType::PostOnly, _, _) | (_, _, true) => "post",
        _ => "lmt",
    };
    let mut parameters = vec![
        ("orderType".to_owned(), order_type.to_owned()),
        ("symbol".to_owned(), futures_symbol(&intent.symbol)),
        (
            "side".to_owned(),
            match intent.side {
                OrderSide::Buy => "buy",
                OrderSide::Sell => "sell",
            }
            .to_owned(),
        ),
        ("size".to_owned(), intent.quantity.to_string()),
        ("cliOrdId".to_owned(), venue_client_id.to_owned()),
        ("reduceOnly".to_owned(), intent.reduce_only.to_string()),
    ];
    if order_type != "mkt" {
        let price = intent
            .price
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| ExchangeError::Parse("Kraken Futures limitPrice missing".to_owned()))?;
        parameters.push(("limitPrice".to_owned(), price.to_string()));
    }
    Ok(parameters)
}

async fn request_text(
    http: &HttpClient,
    base_url: &str,
    credentials: &KrakenFuturesCredentials,
    request: RequestSpec<'_>,
) -> ExchangeResult<String> {
    let RequestSpec {
        method,
        path,
        parameters,
        retry_safe,
    } = request;
    let url = format!("{base_url}{path}");
    let response = if retry_safe {
        http.execute_with_retry_fresh(method.clone(), &url, || {
            signed_request(http, &url, method.clone(), path, &parameters, credentials)
        })
        .await?
    } else {
        let signed = PreparedRequest::new(&url, method, path, &parameters, credentials)?;
        http.execute_once(|| signed.builder(http)).await?
    };
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| ExchangeError::Network(error.to_string()))?;
    if !status.is_success() {
        return Err(ExchangeError::Http {
            status: status.as_u16(),
            body,
        });
    }
    Ok(body)
}

struct RequestSpec<'a> {
    method: Method,
    path: &'a str,
    parameters: Vec<(String, String)>,
    retry_safe: bool,
}

impl<'a> RequestSpec<'a> {
    fn read(path: &'a str) -> Self {
        Self {
            method: Method::GET,
            path,
            parameters: Vec::new(),
            retry_safe: true,
        }
    }

    fn write(path: &'a str, parameters: Vec<(String, String)>) -> Self {
        Self {
            method: Method::POST,
            path,
            parameters,
            retry_safe: false,
        }
    }

    fn retrying_post(path: &'a str, parameters: Vec<(String, String)>) -> Self {
        Self {
            method: Method::POST,
            path,
            parameters,
            retry_safe: true,
        }
    }
}

struct PreparedRequest {
    url: String,
    method: Method,
    body: String,
    nonce: String,
    api_key: String,
    authent: String,
}

impl PreparedRequest {
    fn new(
        url: &str,
        method: Method,
        path: &str,
        parameters: &[(String, String)],
        credentials: &KrakenFuturesCredentials,
    ) -> ExchangeResult<Self> {
        let body = encode(parameters);
        let nonce = next_nonce().to_string();
        let authent = futures_rest_sign(&credentials.api_secret, &body, &nonce, path)
            .map_err(|error| ExchangeError::Auth(format!("kraken futures signing: {error}")))?;
        Ok(Self {
            url: url.to_owned(),
            method,
            body,
            nonce,
            api_key: credentials.api_key.clone(),
            authent,
        })
    }

    fn builder(&self, http: &HttpClient) -> RequestBuilder {
        let mut url = self.url.clone();
        let mut request = http
            .request(self.method.clone(), &url)
            .header("APIKey", &self.api_key)
            .header("Authent", &self.authent)
            .header("Nonce", &self.nonce);
        if self.method == Method::GET && !self.body.is_empty() {
            url.push('?');
            url.push_str(&self.body);
            request = http
                .request(self.method.clone(), &url)
                .header("APIKey", &self.api_key)
                .header("Authent", &self.authent)
                .header("Nonce", &self.nonce);
        } else if self.method == Method::POST {
            request = request
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(self.body.clone());
        }
        request
    }
}

fn signed_request(
    http: &HttpClient,
    url: &str,
    method: Method,
    path: &str,
    parameters: &[(String, String)],
    credentials: &KrakenFuturesCredentials,
) -> ExchangeResult<RequestBuilder> {
    Ok(PreparedRequest::new(url, method, path, parameters, credentials)?.builder(http))
}

fn encode(parameters: &[(String, String)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in parameters {
        serializer.append_pair(name, value);
    }
    serializer.finish()
}

fn parse_success(text: &str, operation: &str) -> ExchangeResult<Value> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken futures {operation}: {error}")))?;
    if value.get("result").and_then(Value::as_str) == Some("error") {
        let error = value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(ExchangeError::Api {
            exchange: "kraken".to_owned(),
            code: error.to_owned(),
            message: error.to_owned(),
        });
    }
    Ok(value)
}

fn parse_account_balances(value: &Value) -> ExchangeResult<Vec<VenueBalanceInfo>> {
    let accounts = value
        .get("accounts")
        .and_then(Value::as_object)
        .ok_or_else(|| ExchangeError::Parse("kraken futures accounts missing".to_owned()))?;
    let mut rows = Vec::new();
    if let Some(cash) = accounts
        .get("cash")
        .and_then(|value| value.get("balances"))
        .and_then(Value::as_object)
    {
        for (currency, total) in cash {
            if let Some(total) = json_number(total) {
                rows.push(balance(canonical_asset(currency), total, total, 0.0));
            }
        }
    }
    if let Some(flex) = accounts
        .get("flex")
        .and_then(|value| value.get("currencies"))
        .and_then(Value::as_object)
    {
        for (currency, item) in flex {
            let total = item
                .get("quantity")
                .and_then(json_number)
                .unwrap_or_default();
            let available = item
                .get("available")
                .and_then(json_number)
                .unwrap_or_default();
            rows.retain(|row: &VenueBalanceInfo| row.currency != canonical_asset(currency));
            rows.push(balance(canonical_asset(currency), total, available, 0.0));
        }
    }
    rows.sort_by(|left, right| left.currency.cmp(&right.currency));
    Ok(rows)
}

fn balance(currency: String, total: f64, available: f64, pnl: f64) -> VenueBalanceInfo {
    VenueBalanceInfo {
        venue: "kraken:futures".to_owned(),
        currency,
        total,
        available,
        frozen: (total - available).max(0.0),
        unrealized_pnl: pnl,
    }
}

fn json_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn status_from_text(value: &str) -> OrderStatus {
    match value {
        "open" | "placed" | "untouched" => OrderStatus::Open,
        "partiallyFilled" => OrderStatus::PartiallyFilled,
        "filled" => OrderStatus::Filled,
        "cancelled" => OrderStatus::Canceled,
        "expired" => OrderStatus::Expired,
        _ => OrderStatus::Rejected,
    }
}

fn next_nonce() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = u64::try_from(common::time::now_ms()).unwrap_or_default() * 1_000;
    let mut previous = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(previous.saturating_add(1));
        match LAST.compare_exchange_weak(previous, next, Ordering::SeqCst, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(actual) => previous = actual,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ExecutionMode, MarginMode, OrderSource};

    #[test]
    fn compiler_uses_official_futures_parameter_names() {
        let intent = OrderIntent {
            id: "i".to_owned(),
            source: OrderSource::Manual,
            strategy: None,
            mode: ExecutionMode::Live,
            exchange: "kraken".to_owned(),
            symbol: "BTC".to_owned(),
            side: OrderSide::Sell,
            order_type: OrderType::PostOnly,
            quantity: 2.0,
            price: Some(40_000.0),
            slippage_tolerance_bps: None,
            reduce_only: true,
            time_in_force: TimeInForce::Gtc,
            post_only: true,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "client".to_owned(),
            client_order_id_policy: None,
            created_at_ms: 1,
        };
        let encoded = encode(&compile_order(&intent, "venue-client").unwrap());
        assert!(encoded.contains("orderType=post"));
        assert!(encoded.contains("symbol=PF_XBTUSD"));
        assert!(encoded.contains("cliOrdId=venue-client"));
        assert!(encoded.contains("reduceOnly=true"));
    }

    fn signed_test_request(
        method: Method,
        path: &str,
        parameters: &[(String, String)],
    ) -> reqwest::Request {
        let http = HttpClient::builder("kraken-futures-test").build().unwrap();
        let credentials = KrakenFuturesCredentials {
            api_key: "key".to_owned(),
            api_secret: "c2VjcmV0".to_owned(),
        };
        PreparedRequest::new(
            &format!("https://futures.kraken.com{path}"),
            method,
            path,
            parameters,
            &credentials,
        )
        .unwrap()
        .builder(&http)
        .build()
        .unwrap()
    }

    #[test]
    fn signed_write_requests_bind_paths_headers_and_payloads() {
        let send = signed_test_request(
            Method::POST,
            SEND_ORDER_PATH,
            &[("orderType".to_owned(), "mkt".to_owned())],
        );
        assert_eq!(send.method(), Method::POST);
        assert_eq!(send.url().path(), SEND_ORDER_PATH);
        assert_eq!(send.headers()["APIKey"], "key");
        assert!(!send.headers()["Authent"].is_empty());
        assert_eq!(
            send.body().and_then(|body| body.as_bytes()),
            Some(b"orderType=mkt".as_slice())
        );

        let cancel = signed_test_request(
            Method::POST,
            CANCEL_ORDER_PATH,
            &[("order_id".to_owned(), "order-1".to_owned())],
        );
        assert_eq!(cancel.url().path(), CANCEL_ORDER_PATH);
        assert!(cancel.body().and_then(|body| body.as_bytes()).is_some());

        let status = signed_test_request(
            Method::POST,
            ORDER_STATUS_PATH,
            &[("orderIds".to_owned(), "order-1".to_owned())],
        );
        assert_eq!(status.url().path(), ORDER_STATUS_PATH);
        assert!(status.body().and_then(|body| body.as_bytes()).is_some());
    }

    #[test]
    fn signed_read_requests_bind_official_paths() {
        let open = signed_test_request(Method::GET, OPEN_ORDERS_PATH, &[]);
        assert_eq!(open.method(), Method::GET);
        assert_eq!(open.url().path(), OPEN_ORDERS_PATH);
        assert!(open.url().query().is_none());

        let positions = signed_test_request(Method::GET, OPEN_POSITIONS_PATH, &[]);
        assert_eq!(positions.url().path(), OPEN_POSITIONS_PATH);

        let accounts = signed_test_request(Method::GET, ACCOUNTS_PATH, &[]);
        assert_eq!(accounts.url().path(), ACCOUNTS_PATH);
    }

    #[test]
    fn parses_official_trade_write_and_order_status_responses() {
        let send = parse_success(
            include_str!("../../fixtures/kraken/futures_send_order_ack.json"),
            "sendorder",
        )
        .unwrap();
        assert_eq!(
            send.pointer("/sendStatus/order_id").and_then(Value::as_str),
            Some("179f9af8-e45e-469d-b3e9-2fd4675cb7d0")
        );
        assert_eq!(
            send.pointer("/sendStatus/status").and_then(Value::as_str),
            Some("placed")
        );

        let cancel = parse_success(
            include_str!("../../fixtures/kraken/futures_cancel_order_ack.json"),
            "cancelorder",
        )
        .unwrap();
        assert_eq!(
            cancel
                .pointer("/cancelStatus/order_id")
                .and_then(Value::as_str),
            Some("cb4e34f6-4eb3-4d4b-9724-4c3035b99d47")
        );
        assert_eq!(
            cancel
                .pointer("/cancelStatus/status")
                .and_then(Value::as_str),
            Some("cancelled")
        );

        let status = parse_success(
            include_str!("../../fixtures/kraken/futures_order_status.json"),
            "orders/status",
        )
        .unwrap();
        let order = parse_order(&status["orders"][0]["order"]).unwrap();
        assert_eq!(order.order_id, "3c90c3cc-0d44-4b50-8888-8dd25736052a");
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn parses_official_rest_accounts_and_open_rows() {
        let balances = parse_account_balances(
            &parse_success(
                include_str!("../../fixtures/kraken/futures_accounts.json"),
                "accounts",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            balances
                .iter()
                .find(|row| row.currency == "BTC")
                .unwrap()
                .available,
            0.1185308247
        );

        let value = parse_success(
            include_str!("../../fixtures/kraken/futures_rest_open_orders.json"),
            "openorders",
        )
        .unwrap();
        let order = parse_order(&value["openOrders"][0]).unwrap();
        assert_eq!(order.quantity, 304.0);
        assert_eq!(order.side, OrderSide::Sell);

        let positions = parse_success(
            include_str!("../../fixtures/kraken/futures_rest_open_positions.json"),
            "openpositions",
        )
        .unwrap();
        let position = parse_position(&positions["openPositions"][0]).unwrap();
        assert_eq!(position.symbol, "BTC");
        assert_eq!(position.side, "short");
        assert!(position.quantity > 0.0);
    }
}
