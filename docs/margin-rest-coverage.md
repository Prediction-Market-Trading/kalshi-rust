# Margin REST coverage

Generated from the pinned official OpenAPI 0.0.1 snapshot, retrieved 2026-11-16. All 55 operations in that snapshot have typed request/query/response bindings and executable HTTP implementations. This is specification coverage, not a claim of live account verification. Server-specific or undocumented endpoints are outside this table.

| Method | Verb | Path | Authentication |
| --- | --- | --- | --- |
| `get_corporate_actions` | GET | `/margin/corporate_actions` | Optional |
| `create_fcm_subtrader` | POST | `/margin/fcm/subtraders` | Required |
| `get_fcm_subtrader_risk_controls` | GET | `/margin/fcm/subtraders/risk_controls` | Required |
| `update_fcm_subtrader_risk_controls` | PUT | `/margin/fcm/subtraders/risk_controls` | Required |
| `delete_fcm_subtrader_risk_controls` | DELETE | `/margin/fcm/subtraders/risk_controls` | Required |
| `create_fcm_position_transfer` | POST | `/margin/fcm/positions/transfers` | Required |
| `update_fcm_notional_risk_limit` | PUT | `/margin/fcm/notional_risk_limit` | Required |
| `delete_fcm_notional_risk_limit` | DELETE | `/margin/fcm/notional_risk_limit` | Required |
| `get_account_api_limits` | GET | `/account/limits/perps` | Required |
| `get_exchange_status` | GET | `/margin/exchange/status` | Optional |
| `get_risk_parameters` | GET | `/margin/risk_parameters` | Optional |
| `get_orders` | GET | `/margin/orders` | Required |
| `create_order` | POST | `/margin/orders` | Required |
| `cancel_all_orders` | DELETE | `/margin/orders` | Required |
| `get_order` | GET | `/margin/orders/{order_id}` | Required |
| `cancel_order` | DELETE | `/margin/orders/{order_id}` | Required |
| `decrease_order` | POST | `/margin/orders/{order_id}/decrease` | Required |
| `amend_order` | POST | `/margin/orders/{order_id}/amend` | Required |
| `get_markets` | GET | `/margin/markets` | Optional |
| `get_market_schedules` | GET | `/margin/markets/schedules` | Optional |
| `get_market_orderbooks` | GET | `/margin/markets/orderbooks` | Optional |
| `get_market` | GET | `/margin/markets/{ticker}` | Optional |
| `get_market_orderbook` | GET | `/margin/markets/{ticker}/orderbook` | Optional |
| `get_market_candlesticks` | GET | `/margin/markets/{ticker}/candlesticks` | Optional |
| `get_fills` | GET | `/margin/fills` | Required |
| `get_positions` | GET | `/margin/positions` | Required |
| `get_trades` | GET | `/margin/trades` | Optional |
| `get_enabled` | GET | `/margin/enabled` | Required |
| `get_notional_risk_limit` | GET | `/margin/notional_risk_limit` | Required |
| `get_balance` | GET | `/margin/balance` | Required |
| `get_risk` | GET | `/margin/risk` | Required |
| `get_fee_tiers` | GET | `/margin/fee_tiers` | Required |
| `get_fee_tier_rates` | GET | `/margin/fee_tier_rates` | Required |
| `get_funding_history` | GET | `/margin/funding_history` | Required |
| `get_historical_funding_rates` | GET | `/margin/funding_rates/historical` | Optional |
| `get_funding_rate_estimate` | GET | `/margin/funding_rates/estimate` | Optional |
| `get_premium_index` | GET | `/margin/funding_rates/premium_index` | Optional |
| `intra_exchange_instance_transfer` | POST | `/portfolio/intra_exchange_instance_transfer` | Required |
| `create_subaccount` | POST | `/portfolio/margin/subaccounts` | Required |
| `apply_subaccount_transfer` | POST | `/portfolio/margin/subaccounts/transfer` | Required |
| `get_order_groups` | GET | `/margin/order_groups` | Required |
| `create_order_group` | POST | `/margin/order_groups/create` | Required |
| `get_order_group` | GET | `/margin/order_groups/{order_group_id}` | Required |
| `delete_order_group` | DELETE | `/margin/order_groups/{order_group_id}` | Required |
| `reset_order_group` | PUT | `/margin/order_groups/{order_group_id}/reset` | Required |
| `trigger_order_group` | PUT | `/margin/order_groups/{order_group_id}/trigger` | Required |
| `set_isolated_exit_trigger` | PUT | `/margin/isolated/positions/{ticker}/exit_trigger` | Required |
| `get_isolated_exit_triggers` | GET | `/margin/isolated/positions/{ticker}/exit_trigger` | Required |
| `delete_isolated_exit_trigger` | DELETE | `/margin/isolated/positions/{ticker}/exit_trigger` | Required |
| `set_cross_exit_trigger` | PUT | `/margin/cross/positions/{ticker}/exit_trigger` | Required |
| `get_cross_exit_triggers` | GET | `/margin/cross/positions/{ticker}/exit_trigger` | Required |
| `delete_cross_exit_trigger` | DELETE | `/margin/cross/positions/{ticker}/exit_trigger` | Required |
| `update_cross_exit_trigger` | PUT | `/margin/cross/positions/{ticker}/exit_trigger/{trigger_id}` | Required |
| `delete_cross_exit_trigger_by_id` | DELETE | `/margin/cross/positions/{ticker}/exit_trigger/{trigger_id}` | Required |
| `update_order_group_limit` | PUT | `/margin/order_groups/{order_group_id}/limit` | Required |

Call these methods through `client.margin()`. Klear clearing-member APIs and FIX are separate surfaces and are not covered.
