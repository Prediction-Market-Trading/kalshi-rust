# Perpetuals (margin)

Kalshi calls perpetual futures "margin" throughout its trading API. This SDK exposes the 55 REST operations and eight WebSocket channels in the pinned perps trading specifications through `Client::margin()`. The facade borrows the client and shares its credentials, HTTP connection pool, REST root and timeout. [Official perps overview](https://docs.kalshi.com/margin).

## Connect and select types

```rust,no_run
use kalshi_sdk_rs::{Client, Credentials, Environment};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let client = Client::builder(Environment::Demo)
    .credentials(Credentials::from_env()?)
    .build()?;
let margin = client.margin();
let enabled = margin.get_enabled().await?;
let markets = margin.get_markets(&Default::default()).await?;
# let _ = (enabled, markets);
# Ok(()) }
```

Demo and production have separate credentials and account enablement. REST uses the same `/trade-api/v2` root as predictions. Most perps operations live under `/margin`, but perps API limits use `/account/limits/perps`, while subaccount and inter-product transfers use documented `/portfolio/...` paths. A `base_url(...)` override must point to the API root, without an appended `/margin`.

Methods remove `Margin` from the upstream operation ID before converting it to snake case. For example, `GetMarginOrders` becomes `get_orders`; `SetCrossMarginExitTrigger` becomes `set_cross_exit_trigger`. `GetPerpsAccountApiLimits` becomes `get_account_api_limits`. Models retain upstream names under `margin::models`, including `GetMarginOrdersQuery` and `CreateMarginOrderRequest`. Arguments follow the existing path, query, body order. See the [complete method-to-path table](margin-rest-coverage.md).

Paginated calls return one page. Feed `response.next_cursor()` into the next query's `cursor`; absent or empty cursors mean completion. Schedule and multiple-orderbook queries accept `Vec<String>` tickers and encode them as the comma-separated query value required by those endpoints. They do not use the repeated-key convention of predictions array queries.

## Values and trading

Perps reuse `FixedPointDollars` and `FixedPointCount`. Prices can exceed $1; dollar response fields allow six decimal places, and counts allow two. The wrappers preserve exact decimal strings and check representation. Numeric funding rates and leverage values use `serde_json::Number`; fields that the schema defines as plain strings retain that representation.

Perps books contain bids and asks priced in dollars, without the predictions YES/NO transformation. Market metadata provides `tick_size`, `minimum_quantity`, `fractional_trading_enabled`, contract size and underlying multiplier. When fractional trading is disabled, submit whole-contract counts such as `"3.00"`. Market tick and count rules remain exchange-validated.

Order creation, reads, cancellation, amendment, decrease, cancel-all and order groups have their own margin methods. Supply a unique `client_order_id`. An amendment's count is the maximum total fillable quantity, including already filled contracts; a decrease uses exactly one of `reduce_by` or `reduce_to`. A reduce-only order must use immediate-or-cancel or fill-or-kill. `market_version` can reject an order when a corporate action has changed the market since it was read. The SDK passes these fields through without adding business-rule validation.

Some markets have scheduled trading hours. `market.schedule` is `None` when the wire value is `null`, including markets that trade continuously. A market outside its session can return `409 market_inactive`, as can a halted market. Funding times vary by asset class and daylight saving time: read the next funding time from the funding estimate or ticker message instead of calculating it from a fixed UTC schedule.

Orders placed by the exchange for liquidation, exit triggers or auto-rebalance carry `order_source: system`. REST orders additionally expose `order_reason`. Fills and streamed user orders expose the source alone; look up the order to learn the reason.

## Funding, risk, transfers and triggers

The margin facade includes funding payment history, historical rates, estimates, premium index, risk parameters, account/position risk, notional limits, effective fees, fee schedules, subaccounts, transfers and the published FCM trading operations. Endpoint permissions and account capabilities still apply.

Transfers between predictions and margin have a binding even where the rollout has not enabled the operation. Withdrawals from margin and transfers between margin subaccounts can return `403 permission_denied` with details `margin withdrawals are temporarily disabled during settlement and funding` during funding, settlement or a cash corporate action. The application owns backoff and reconciliation; this error does not necessarily indicate a key-permission problem.

Isolated and cross-margin exit-trigger methods expose bracket and trailing-stop parameters, including partial and order-anchored cross brackets. A fired trigger submits a reduce-only, immediate-or-cancel limit order rather than a market order, so it can partially fill or fail to fill. A trigger can remain active while the exchange cannot place its order. The exchange validates liquidation thresholds, trigger geometry and conflicts with resting closing orders; acceptance is not a guarantee of execution before liquidation. Preserve HTTP status and the returned message for exit-trigger validation failures whose error codes are not stable. [Trading behavior and caveats](https://docs.kalshi.com/margin#trading-behavior).

## WebSockets

```rust,no_run
use kalshi_sdk_rs::{Client, Credentials, Environment};
use kalshi_sdk_rs::margin::websocket::{Channel, SubscribeParams};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let client = Client::builder(Environment::Demo)
    .credentials(Credentials::from_env()?)
    .build()?;
let mut socket = client.margin().connect_websocket().await?;
socket.subscribe(1, SubscribeParams {
    channels: vec![Channel::Ticker, Channel::OrderbookDelta, Channel::Trade],
    market_tickers: Some(vec!["YOUR-DEMO-PERPS-TICKER".into()]),
    send_initial_snapshot: Some(true),
    ..Default::default()
}).await?;
while let Some(message) = socket.next_message().await? {
    println!("{message:?}");
}
# Ok(()) }
```

| Channel | Typed messages |
| --- | --- |
| `orderbook_delta` | `OrderbookSnapshot`, `OrderbookDelta` |
| `ticker` | `Ticker` |
| `trade` | `Trade` |
| `fill` | `Fill` |
| `user_orders` | `UserOrder` |
| `order_group_updates` | `OrderGroupUpdates` |
| `corporate_actions` | `CorporateActionsDelta` |
| `market_lifecycle_v2` | `MarketLifecycleV2` |

The handshake signs the full `/trade-api/ws/v2/margin` path and requires credentials, including for market-data channels. `Environment::margin_websocket_url()` supplies the appropriate host. `ClientBuilder::margin_websocket_url(...)` overrides only margin connections; `websocket_url(...)` continues to select the predictions destination.

For lower-latency fills and user-order updates, override the URL with `wss://margin-user-ws.demo.kalshi.co/trade-api/ws/v2/margin` in demo or `wss://margin-user-ws.kalshi.com/trade-api/ws/v2/margin` in production. These hosts currently serve only `fill` and `user_orders`.

Use the margin subscription types: there is no predictions `use_yes_price` setting or required exchange `clock`. Timestamp fields use Unix milliseconds and `_ms` names; REST timestamps retain their individual documented wire units. Preserve `sid` and subscription `seq` and apply deltas in sequence order. Ticker messages can include reference and mark prices and the next funding time. Corporate actions publish changes with no snapshot on subscription; REST supplies the available notices.

`update_interval_ms` opts an orderbook subscription into the documented conflation interval. A snapshot replaces the corresponding book. To recover a sequence gap, request fresh snapshots for all subscribed markets or unsubscribe and subscribe again; repeating a subscription alone does not resynchronize markets already subscribed. Conflated timestamps can decrease and do not prove completeness. [Orderbook subscription semantics](https://docs.kalshi.com/margin-ws/websockets/orderbook-updates).

The receive and control APIs mirror predictions: subscribe/unsubscribe/list/update commands, typed acknowledgments and errors, `next_event()` for messages and heartbeat frames, `next_message()` to filter control frames, `ping()` and `close()`. Matching pongs are flushed automatically while reads run. After `close()`, continue reading until `None`. Reads preserve pending control replies when cancelled. Unknown message types retain JSON; malformed known messages return a decode error. Applications own reconnect, resubscription, sequence recovery and book reconstruction.

## Examples and verification

`cargo run --example margin_account` reads enablement, markets and balance. `cargo run --example margin_stream` streams ticker, trades and orderbook updates until Ctrl-C; set `KALSHI_MARGIN_MARKET_TICKER`. Both select demo by default, accept `KALSHI_ENV=production`, load credentials through the existing environment variables, and never submit an order.

### Record a short orderbook feed

`margin_orderbook_log` records a perps orderbook for five seconds by default, then closes the connection. It writes one typed `orderbook_snapshot` or `orderbook_delta` envelope per JSONL line, retaining price/count strings, subscription IDs, sequence numbers and available timestamps. The console reports the selected market, log path and message counts.

With a margin-enabled demo account, run this from PowerShell:

```powershell
$env:KALSHI_ENV = 'demo'
$env:KALSHI_API_KEY_ID = 'your-demo-key-id'
$env:KALSHI_PRIVATE_KEY_PATH = 'C:\path\to\demo-private-key.pem'
cargo run --example margin_orderbook_log -- --seconds 5
```

WebSocket market data requires credentials. The existing `KALSHI_PRIVATE_KEY_PEM` alternative is supported; `.env` files are not loaded automatically. Without a ticker, the example chooses the active, currently open perps market with the highest available 24-hour volume. `--ticker` overrides `KALSHI_MARGIN_MARKET_TICKER`:

```powershell
cargo run --example margin_orderbook_log -- --ticker YOUR-PERPS-TICKER --seconds 10 --output target/perps-book.jsonl
```

The default path is `target/margin-orderbook-<unix-ms>.jsonl`. Existing files are never truncated: choose another output path for a subsequent run. Use `--help` for options. A run with an initial snapshot and no deltas is valid when the book does not change; receiving no snapshot reports an error. The example never submits an order.

### Place and cancel a perps order

`margin_order_lifecycle` prints margin balances, then posts a **one-contract buy limit order at $0.0100 on `KXETHPERP`**. It requires positive available collateral in the selected cross-margin subaccount, subscribes to `user_orders` and waits for the acknowledgment before submitting. It continues reading the stream during REST requests, lets the order rest for up to five seconds by default, then requests cancellation.

Run it with matching credentials and an explicit submission opt-in:

```powershell
$env:KALSHI_ENV = 'demo' # Use 'production' with production credentials to trade real funds.
$env:KALSHI_API_KEY_ID = 'your-demo-key-id'
$env:KALSHI_PRIVATE_KEY_PATH = 'C:\path\to\demo-private-key.pem'
cargo run --example margin_order_lifecycle -- --submit --seconds 5
```

The balance check sets `compute_available_balance=true`; a positive balance does not guarantee that the exchange will accept the order's margin requirement. The example checks market status, tick and quantity constraints. The exchange can reject the $0.01 price under its price-band rules; the example reports the rejection and does not choose a different price.

The order uses `post_only` and good-till-canceled with a 60-second expiry as a fallback if the process stops. A post-only order can fill after it rests, and cancellation only removes the remaining quantity; it does not reverse fills. The example attempts cancellation on errors or Ctrl-C, but the expiry is not a guarantee that a cancellation request succeeds.

Only this order's `user_order` envelopes are written as JSONL to `target/margin-order-updates-<unix-ms>.jsonl`. Balances are printed to the console. Existing log files are not overwritten. `--output PATH` changes the log destination, `--subaccount N` selects a cross-margin subaccount from 0-63 (default `0`), and `--seconds N` accepts 1-30 seconds. Use `--help` to inspect the options without submitting an order.

### Transfer one dollar from predictions to perps

`margin_transfer` transfers exactly **$1.00 (10,000 centicents)** from the primary predictions account on shard `0` to the primary perps account. Before a new submission, it prints balances and requires at least $1 available in the source account. It polls an accepted transfer for up to 15 seconds and prints balances after confirmed completion; an accepted request alone is not reported as a completed transfer.

```powershell
$env:KALSHI_ENV = 'demo' # Use 'production' with production credentials to move real funds.
$env:KALSHI_API_KEY_ID = 'your-demo-key-id'
$env:KALSHI_PRIVATE_KEY_PATH = 'C:\path\to\demo-private-key.pem'
cargo run --example margin_transfer -- --submit
```

The key needs `read` and `write::transfer` (or the parent `write` scope). A failed perps balance read is reported without blocking the transfer. Kalshi's public overview still describes cross-product API transfers as rollout-gated; the example reports API rejection or unavailable status without substituting another operation.

Before sending, the example saves its client transfer ID in `target/margin-transfer-one-dollar-<environment>.json`, then saves the server transfer ID when available. **Keep this state file and rerun with the same path** to resume or reconcile that transfer after a timeout. A saved server transfer ID resumes status reads without another transfer POST or a new source-balance requirement. It does not generate a new transfer ID and blindly resend another $1, or automatically transfer the funds back. An unresolved/pending result is reported for later inspection.

To deliberately start a separate $1 test, choose a fresh state path:

```powershell
cargo run --example margin_transfer -- --submit --state target/another-margin-transfer.json
```

The offline tests use local HTTP/WebSocket servers, schema examples and generated ephemeral signing keys. Optional ignored integration tests select demo explicitly; order submission requires a separate margin opt-in. No authenticated live perps verification is claimed. See [verification results](verification.md).

With margin-enabled demo credentials configured, select only the read-only check:

```sh
cargo test --test margin_demo margin_demo_authenticated_read_only -- --ignored --exact
```

The order check additionally requires `KALSHI_MARGIN_DEMO_ORDER_OPT_IN=YES`, `KALSHI_MARGIN_MARKET_TICKER` and `KALSHI_MARGIN_DEMO_ORDER_PRICE`. Optional `KALSHI_MARGIN_DEMO_ORDER_COUNT` defaults to `1.00`. Choose a valid tick, minimum size and whole/fractional count for that demo market. The test creates a post-only order with a one-minute expiry and then cancels it; a resting post-only order can still fill.

```sh
cargo test --test margin_demo margin_demo_create_and_cancel_order -- --ignored --exact
```

The separate Klear clearing-member API uses its own hosts and bearer credentials and is outside this SDK extension. FIX, automatic retries, rate limiting, trading strategies and portfolio accounting are also outside the SDK. Existing predictions APIs and their pinned snapshots remain available unchanged.

Margin supports multiple-orderbook reads, but does not offer batch order creation/cancellation or queue positions. Predictions-only events, series, RFQs/quotes and archived market/order endpoints are not projected onto margin. Historical funding rates and payment history are included through their documented margin endpoints.
