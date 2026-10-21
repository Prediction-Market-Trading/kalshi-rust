# API coverage and migration notes

Audit date: October 20, 2026. The pinned Predictions REST spec identifies itself as **OpenAPI 3.34.0**; the WebSocket spec is **AsyncAPI 3.0.0**, API information version **2.0.0**. URL versions remain `/trade-api/v2` and `/trade-api/ws/v2`.

## Predictions REST

All 118 operations in the downloaded REST snapshot have generated typed methods and models. The complete method-to-path table is [rest-coverage.md](rest-coverage.md). Every method invokes the shared HTTP transport; none is a placeholder. All query arrays in this snapshot use `style: form, explode: true` and are encoded as repeated keys. Other list filters that the spec defines as strings remain comma-separated strings supplied by the caller.

| Area | Implemented | Verification |
| --- | --- | --- |
| Exchange | Status, schedule, user data timestamp, fee changes | Status live read; shared transport mocked |
| Markets | Lists/details, orderbooks (single/batch), trades, candlesticks (single/batch/event) | Market list/detail, single orderbook, trades live reads; captured fixtures |
| Events and series | Lists/details, metadata, event fees, percentile history | Event list live read; other methods compile/spec checked |
| Trading | V2 create/cancel/amend/decrease, batches, cancel-all, order reads, queue positions | Create/cancel/amend/decrease mocked; optional demo test not run |
| Portfolio | Balance, positions, fills, settlements, deposits, withdrawals, resting value | Typed models and transport; credentials needed for live checks |
| Order groups | List/create/get/delete/reset/trigger/limit | Typed models and transport; credentials needed |
| Subaccounts and transfers | Create/transfer/balances/history/netting; intra-exchange transfers and target allocation | Typed models and transport; credentials needed |
| Historical | Cutoff, markets/details/candlesticks/trades/orders/fills/positions | Cutoff live read; other methods compile/spec checked |
| Communications | RFQs, quotes, block proposals and lifecycle operations | Typed models and transport; credentials/permissions needed |
| Account and API keys | Limits/costs/usage progress/upgrades; list/create/generate/delete keys | Typed models and transport; credentials needed |
| Discovery/live data | Search filters, milestones, structured targets, multivariate collections, incentives, weather/game/live data | Typed models and transport; flexible data remains schema-defined JSON |
| FCM | Fills, orders, positions, subtraders, caps, categories, position transfers | Typed bindings; FCM permissions and live verification required |

Some documentation-only endpoints may be absent from the manual OpenAPI snapshot, whose description explicitly says endpoints are being migrated to spec-first definitions. This SDK claims coverage of the pinned spec, not every service Kalshi offers. Compare the [documentation index](https://docs.kalshi.com/llms.txt) when updating coverage.

## Predictions WebSockets

Connection/authentication, subscribe/unsubscribe/list/update commands, typed acknowledgments/errors, ping/pong and close handshakes are implemented. All 38 server examples in the downloaded AsyncAPI document decode in offline tests; a local server exercises the signed handshake and command lifecycle. Live streams have not been tested with an account.

| Subscription channel | Typed message variants | Coverage |
| --- | --- | --- |
| `ticker` | `Ticker` | Implemented |
| `orderbook_delta` | `OrderbookSnapshot`, `OrderbookDelta` | Implemented; exact price/count tuples, signed deltas, sid/seq preserved |
| `trade` | `Trade` | Implemented |
| `fill` | `Fill` | Implemented; account channel |
| `user_orders` | `UserOrder` | Implemented; account channel |
| `market_positions` | `MarketPosition` | Implemented; account channel |
| `market_lifecycle_v2` | `MarketLifecycleV2`, `MarketMetadataUpdated`, `EventLifecycle`, `EventFeeUpdate` | Implemented |
| `multivariate_market_lifecycle` | `MultivariateMarketLifecycle` | Implemented |
| `communications` | `RfqCreated`, `RfqDeleted`, `QuoteCreated`, `QuoteAccepted`, `QuoteExecuted` | Implemented |
| `order_group_updates` | `OrderGroupUpdates` | Implemented |
| Shared external-feed schemas | CF Benchmarks/Pyth messages and command models | Decode support only; dedicated feed transport/entitlements not verified |

Commands expose all documented subscription parameters, including market UUIDs, tickers, initial snapshots, pricing convention, user filters and communication shards. `send_command` accepts the typed wire command structures for specialized operations.

Unknown message types preserve full JSON. Known malformed messages fail rather than being silently treated as unknown. The SDK does not add reconnection, book reconstruction, sequence-gap repair, durable buffering or trading decisions.

## Breaking changes handled

- Ed25519 alongside RSA-PSS, detected by parsing PKCS#8 contents. RSA PKCS#1 is also supported. No legacy password login.
- Fixed-point dollar and quantity strings replace old integer-cent/integer-count response assumptions. Dollar values allow six decimals; quantities allow two. Request market tick/scale constraints remain server-validated.
- Current V2 order writes use `/portfolio/events/orders`, book side and a single price. Legacy request schemas remain available because they occur in the source specification, but no method sends them to retired order-write paths.
- Order/fill/trade direction uses canonical `outcome_side`/`book_side` (or taker variants). Deprecated legacy fields remain optional. Bid means yes exposure and ask means no exposure; response price is not inverted based on exposure.
- WebSocket `use_yes_price` explicitly selects unified yes-leg orderbook pricing. Its future default change/removal is announced as a plan, not assumed already deployed.
- Historical data is split from recent endpoints. No hidden pagination or automatic merging.
- Shard routing and subaccounts are explicit. Ticker is necessary for cancellation auto-routing when an order ID cannot determine the exchange shard.
- Recommended dedicated REST and WebSocket hosts are used. Shared older hosts can be configured explicitly.
- Nullable response fields and removed legacy fields do not require placeholder defaults. Missing/empty cursors both terminate pagination.

## Separate or missing functionality

| Surface | Status |
| --- | --- |
| Perps/Margin REST (`perps_openapi.yaml`, `/margin/...`) | Not implemented; do not use Predictions models or presets for Perps |
| Perps/Margin WebSocket (`perps_asyncapi.yaml`) | Not implemented; timestamps, prices and contracts differ |
| FIX | Not implemented |
| Dedicated external feeds connection | Not implemented; shared wire types alone do not grant access |
| CF Benchmarks REST passthrough | Not implemented; separate surface |
| Undocumented REST endpoints | Not exposed |
| Legacy login/logout and old order writes | Not exposed |
| Automatic retry/rate limiting/reconnection | Caller responsibility |
| Full business-rule validation | Exchange responsibility; structs describe the wire format |

The official [Perps overview](https://docs.kalshi.com/margin) and its separate specs must be researched before extending this SDK into that area.
