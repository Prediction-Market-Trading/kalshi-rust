# Kalshi Rust SDK

An independent async Rust SDK for Kalshi's **Predictions** REST and WebSocket APIs. Query markets, trade, inspect your account, and consume typed streams. There is no trading engine, strategy layer, portfolio accounting, or local orderbook implementation.

This is an unpublished `0.1.0` crate named `kalshi-sdk-rs` (Rust import: `kalshi_sdk_rs`). It contains typed bindings for all **118 operations in the pinned official Predictions OpenAPI 3.34.0 snapshot**, retrieved October 20, 2026, and the Predictions channels in AsyncAPI 2.0.0. Specification coverage does not imply live verification of every endpoint. Public market-data calls have been exercised against Kalshi; authenticated live calls need your credentials.

## Install

Requires **Rust 1.95 or newer**, Cargo, and a C compiler for `ring`. Rust 1.95 is the supported and tested baseline; earlier toolchains are not promised. TLS uses Rustls and your OS trust store, without an OpenSSL runtime dependency. The default features include REST and WebSocket support.

Clone this repository, then add a local path dependency:

```toml
[dependencies]
kalshi-sdk-rs = { path = "../kalshi-rust" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Or run `cargo add kalshi-sdk-rs --path /absolute/path/to/kalshi-rust`. There is no crates.io release to install yet. Examples in this repository run directly with `cargo run --example NAME`.

## Environments

Choose an environment explicitly; demo and production credentials are separate. [Official environment documentation](https://docs.kalshi.com/getting_started/api_environments).

| Environment | REST root | WebSocket URL |
| --- | --- | --- |
| `Environment::Demo` | `https://external-api.demo.kalshi.co/trade-api/v2` | `wss://external-api-ws.demo.kalshi.co/trade-api/ws/v2` |
| `Environment::Production` | `https://external-api.kalshi.com/trade-api/v2` | `wss://external-api-ws.kalshi.com/trade-api/ws/v2` |

The older `api.elections.kalshi.com` and `demo-api.kalshi.co` hosts remain supported by Kalshi and can be selected with builder URL overrides. Predictions still uses the `/trade-api/v2` URL prefix; the OpenAPI package version `3.34.0` is a separate version number.

The read-only public examples use production market data. Every authenticated example and integration test is hardcoded to **Demo**. The library itself permits production trading when you explicitly choose `Environment::Production`.

## Public market data

No credentials are required for public endpoints. The following complete program lists open markets:

```rust,no_run
use kalshi_sdk_rs::{Client, Environment, models::{GetMarketsQuery, GetMarketsQueryStatus}};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Environment::Production)?;
    let page = client.get_markets(&GetMarketsQuery {
        status: Some(GetMarketsQueryStatus::Open),
        limit: Some(20),
        ..Default::default()
    }).await?;
    for market in page.markets {
        println!("{}: {}", market.ticker, market.title.as_deref().unwrap_or(""));
    }
    Ok(())
}
```

`get_exchange_status`, `get_events`, `get_event`, `get_market`, `get_market_orderbook`, `get_trades`, series/candlestick methods, and historical methods are also available. Methods follow snake_case official operation IDs, with arguments ordered as path parameters, typed query, and typed body. A query-bearing operation takes a query structure even when all parameters are optional; use `&Default::default()` to omit them. See [every REST method and endpoint](docs/rest-coverage.md).

Orderbook prices and quantities are `(FixedPointDollars, FixedPointCount)` tuples under `orderbook_fp.yes_dollars` and `no_dollars`. REST orderbooks contain **YES bids and NO bids**, each in its own outcome price convention. A NO bid at $0.30 corresponds to a YES ask at $0.70. The SDK exposes the wire data without rebuilding or transforming a book.

## Authentication

Create a demo or production API key in that environment's account settings. Ed25519 PKCS#8 PEM and RSA PKCS#1/PKCS#8 PEM are supported. `BEGIN PRIVATE KEY` alone does not identify the algorithm. Encrypted PEMs and obsolete email/password login are not supported. [Official key and signing instructions](https://docs.kalshi.com/getting_started/api_keys).

```sh
export KALSHI_API_KEY_ID='your-demo-key-id'
export KALSHI_PRIVATE_KEY_PATH='/absolute/path/to/demo-private-key.pem'
cargo run --example balance
```

Alternatively, set `KALSHI_PRIVATE_KEY_PEM` to the PEM text with actual newlines. It takes precedence over the path variable. The SDK does not load `.env` files automatically.

```rust,no_run
use kalshi_sdk_rs::{Client, Credentials, Environment};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
let credentials = Credentials::from_env()?;
// Also available: Credentials::from_file(key_id, path) and
// Credentials::from_pem(key_id, pem_text) for programmatic configuration.
let client = Client::builder(Environment::Demo)
    .credentials(credentials)
    .timeout(std::time::Duration::from_secs(15))
    .build()?;
# Ok(()) }
```

Parsed keys are shared across cloned clients. Debug output redacts credentials and generated-key responses. The SDK never logs keys or responses. Treat account responses and API error bodies as sensitive if you log them yourself. API-key generation methods return private key material by design; store that material securely.

Signing is Base64 of a signature over `timestamp_ms + UPPERCASE_METHOD + full_path`, excluding the hostname, query and body. RSA-PSS uses SHA-256, MGF1-SHA256 and a 32-byte salt; Ed25519 signs the raw pre-sign bytes. Public REST requests are also signed when credentials are configured, allowing Kalshi to apply the authenticated account's limits. Keep your system clock synchronized.

## Exact values and wire models

`FixedPointDollars` and `FixedPointCount` wrap exact `rust_decimal::Decimal` values and always serialize as JSON **strings**. Parse from decimal text: `"0.1234".parse::<FixedPointDollars>()?` or `"2.50".parse::<FixedPointCount>()?`. Dollar responses allow up to six decimal places, quantities up to two; negative quantities support positions and deltas. `decimal()` exposes the exact value for checked arithmetic. Scientific notation, JSON numeric values, invalid syntax and excess precision are rejected.

Request prices generally require two to four decimal places and must respect each market's `price_ranges` and `price_level_structure`. Fixed-point parsing checks representation, not market tick validity, quantity positivity or business rules. Other JSON numeric fields (for example strikes and weather measurements) use `serde_json::Number` with arbitrary precision, not `f64`. Timestamps retain the documented wire representation: strings, Unix seconds or milliseconds; see each field's Rustdoc.

Requests and responses use public structs under `models`. Optional fields are omitted when `None`. Required response fields are checked during deserialization. Deprecated fields are optional to tolerate their removal. String enums retain future values as `Unknown(String)`. Extensible, schema-free metadata uses maps/JSON where the official spec leaves its shape undefined. **`Default` provides construction convenience; it does not create a complete or valid order.** Supply all documented required fields.

## Place, cancel and amend orders

Order writes use the current `/portfolio/events/orders` endpoints, **`bid`/`ask`**, and fixed-point `price`/`count` strings. Reads such as `get_orders` and `get_order` still use `/portfolio/orders` as specified by Kalshi. Legacy order-write paths are not exposed.

```rust,no_run
use kalshi_sdk_rs::{Client, Credentials, Environment, models::*};
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let client = Client::builder(Environment::Demo)
    .credentials(Credentials::from_env()?).build()?;
let ticker = "YOUR-OPEN-DEMO-MARKET";
let order = client.create_order_v2(&CreateOrderV2Request {
    ticker: ticker.into(),
    side: BookSide::Bid,
    count: "1.00".parse()?,
    price: "0.0100".parse()?,
    time_in_force: CreateOrderV2RequestTimeInForce::GoodTillCanceled,
    self_trade_prevention_type: SelfTradePreventionType::TakerAtCross,
    post_only: Some(true),
    ..Default::default()
}).await?;
client.cancel_order_v2(&order.order_id, &CancelOrderV2Query {
    market_ticker: Some(ticker.into()), // route to this market's exchange shard
    ..Default::default()
}).await?;
# Ok(()) }
```

Supply a unique UUID `client_order_id` in real applications to reconcile uncertain results. The SDK never retries orders automatically. A transport timeout can occur after the exchange accepted an order; inspect orders before deciding what to send next.

`amend_order_v2` accepts `AmendOrderV2Request` with ticker, book side, price and **total/max fillable** count (already filled plus desired remaining quantity). `decrease_order_v2` accepts exactly one of `reduce_by` or `reduce_to`. Batch creation/cancellation and order groups are supported. There is no separate invented replace operation; use the documented amendment API. `cancel_all_orders` can affect newly placed orders during the minute after the call, per the official spec.

Exchange sharding matters: pass `market_ticker` when canceling/decreasing, or an explicit `exchange_index`. An order ID alone cannot auto-route. Creation/amendment can auto-route by ticker. Account queries can scope by `exchange_index` and/or subaccount as documented on each method.

Run the complete guarded example, which includes a one-minute expiry:

```sh
export KALSHI_MARKET_TICKER='YOUR-OPEN-DEMO-MARKET'
export KALSHI_DEMO_ORDER_OPT_IN=YES
export KALSHI_DEMO_ORDER_PRICE=0.0100  # choose a valid price for this market
cargo run --example demo_order
```

Post-only orders can still fill after resting. Use a suitable demo market and price. The library's ordinary order methods do not require the example's opt-in variable; that guard belongs to the example and tests.

## Account and portfolio

```rust,no_run
use kalshi_sdk_rs::{Client, Credentials, Environment, models::*};
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let client = Client::builder(Environment::Demo)
    .credentials(Credentials::from_env()?).build()?;
let balance = client.get_balance(&GetBalanceQuery::default()).await?;
println!("Available dollars: {}", balance.balance_dollars);
let orders = client.get_orders(&GetOrdersQuery::default()).await?;
let positions = client.get_positions(&GetPositionsQuery::default()).await?;
let fills = client.get_fills(&GetFillsQuery::default()).await?;
# let _ = (orders, positions, fills);
# Ok(()) }
```

For open orders, set `GetOrdersQuery.status` to `Some("resting".into())`. Settlements, deposits, withdrawals, subaccounts, transfers and API limits have their own typed methods. Some institutional operations require additional permissions.

## Pagination and history

Methods return one page. Pass `response.next_cursor()` to the next query's `cursor` and stop on `None`. Both absent and empty cursors mean completion. The `list_markets` example demonstrates a bounded pagination loop. The SDK does not eagerly fetch all data or hide extra requests.

Recent endpoints omit data older than the exchange's historical cutoff. Use `get_historical_cutoff` to discover the boundary, then `get_trades_historical`, `get_fills_historical`, `get_historical_orders`, `get_historical_positions`, `get_historical_markets`, `get_historical_market` or `get_market_candlesticks_historical` as appropriate. The SDK leaves merging recent/historical pages and choosing time ranges to the caller.

## WebSockets

All Predictions WebSocket handshakes need credentials. Subscribe with a caller-assigned command `id`; use the `sid` from the server's `subscribed` acknowledgment to manage that subscription.

```rust,no_run
use kalshi_sdk_rs::{Client, Credentials, Environment};
use kalshi_sdk_rs::websocket::{Channel, Message, SubscribeParams};
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let client = Client::builder(Environment::Demo)
    .credentials(Credentials::from_env()?).build()?;
let mut socket = client.connect_websocket().await?;
socket.subscribe(1, SubscribeParams {
    channels: vec![Channel::Ticker, Channel::OrderbookDelta, Channel::Trade],
    market_tickers: Some(vec!["YOUR-OPEN-DEMO-MARKET".into()]),
    use_yes_price: Some(true),
    ..Default::default()
}).await?;
while let Some(message) = socket.next_message().await? {
    match message {
        Message::OrderbookDelta(delta) => {
            println!("seq={} price={} change={}", delta.seq,
                delta.msg.price_dollars, delta.msg.delta_fp);
        }
        Message::Error(error) => eprintln!("Subscription error: {:?}", error.msg),
        _ => {}
    }
}
# Ok(()) }
```

The subscription channel `user_orders` emits messages tagged **`user_order`**. `fill` and `user_orders` can subscribe account-wide without tickers. The [user stream example](examples/user_stream.rs) receives both.

`subscribe`, `unsubscribe`, `list_subscriptions` and `update_subscription` send commands; the next received messages contain the server's acknowledgment or error. You must consume those responses. Full typed command models and `send_command` support the documented less common parameters.

The client handles server pings with matching pongs while `next_message` runs. It returns `None` after a clean close and preserves transport/decode errors. Use `close()` then continue reading until `None` to complete a client-initiated close handshake. The runnable stream examples handle Ctrl-C this way. There is no background reader, reconnect loop or in-memory event queue. Keep driving reads while idle. Wrap reads in `tokio::time::timeout` if you need idle timeouts.

Track subscription `sid` and `seq` according to the channel's sequencing rules. After a gap or disconnect, reconnect, resubscribe and request a fresh snapshot before applying more deltas. The SDK does not reconstruct books. Unknown top-level message types retain their JSON; malformed known messages return an error. Metadata lifecycle updates and the different `ok` shapes are decoded separately.

`use_yes_price: true` chooses yes-leg pricing on both sides of WebSocket orderbook messages. The documented default is currently false with a future migration planned, so choose the convention explicitly. This flag does not transform REST orderbooks. [Official order direction and pricing guide](https://docs.kalshi.com/getting_started/order_direction).

## Errors and rate limits

`Error::Api` preserves HTTP `status`, structured boxed `error` (`code`, `message`, `details`, `service`), raw `body`, and `retry_after`. Non-JSON failures still carry status and body. A malformed success body is `Error::Json`; missing credentials fail before a request is sent. Transport, WebSocket, configuration, I/O and fixed-point errors are distinct variants.

The SDK sends each request once. It has no automatic retries or rate limiter. Handle 429 with account-appropriate pacing, `Retry-After` and backoff in your application. `Retry-After` is retained as text because it may be seconds or an HTTP date. Do not replay writes blindly. Redirects are disabled. The default request/handshake timeout is 30 seconds and is configurable. Builder destination overrides are intended for mocks/proxies and receive your credentials if configured.

## Runnable examples

| Command | Purpose | Credentials |
| --- | --- | --- |
| `cargo run --example list_markets` | Two pages of active production markets | None |
| `cargo run --example orderbook` | Production orderbook for `KALSHI_MARKET_TICKER` | None |
| `cargo run --example balance` | Demo account balance | Demo |
| `cargo run --example demo_order` | Submit/cancel guarded demo order | Demo, explicit opt-in |
| `cargo run --example market_stream` | Demo ticker, trades, orderbook stream | Demo |
| `cargo run --example user_stream` | Demo order and fill updates | Demo |
| `cargo run --example public_smoke` | Read-only public compatibility probe | None |

## Verification and maintenance

```sh
cargo fmt --check
cargo check --all-targets
cargo test
cargo clippy --all-targets -- -D warnings
cargo doc --no-deps
```

Ordinary tests use ephemeral keys, recorded public fixtures and local HTTP/WebSocket servers; they require no account and never place real orders. See [verification results and limits](docs/verification.md).

Optional integration tests are **ignored by default** and hardcode Demo:

```sh
cargo test --test demo demo_authenticated_read_only -- --ignored --exact
KALSHI_DEMO_ORDER_OPT_IN=YES cargo test --test demo demo_create_and_cancel_order -- --ignored --exact
```

The second test also requires `KALSHI_MARKET_TICKER` and optionally `KALSHI_DEMO_ORDER_PRICE`. Never use production credentials in demo tests.

Pinned machine-readable specifications live in `spec/`. `python3 tools/generate.py && cargo fmt` regenerates REST models/methods, WebSocket models/dispatch and the REST coverage table without network access. Python 3.9+ is needed only for development, not to build or use the crate. There is no build-time generator. See [research and design rationale](docs/research.md), [coverage and exclusions](docs/coverage.md), [snapshot provenance](spec/README.md) and [contribution guidance](CONTRIBUTING.md).

## Limits

Perps/Margin APIs, FIX, dedicated external feeds transport, CF Benchmarks REST passthrough and undocumented endpoints are separate coverage areas. This crate's environment presets and examples target Predictions. Some external-feed wire models appear in the shared AsyncAPI snapshot but that does not establish feed entitlements or provide a separate feed connection API.

Authenticated trading, account operations, institutional methods and live WebSocket streams remain unverified without credentials. The server enforces endpoint permissions, field ranges, market ticks, mutual exclusions and other business constraints. There is no full OpenAPI validator. New undocumented response fields are ignored; future enum values and unknown WebSocket types are retained. An incompatible change to a required field can require a spec/model update.

MIT licensed. This is an independent client, not an official Kalshi SDK. Community libraries were inspected for architecture and failure modes; their source code was not copied.
