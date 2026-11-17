use kalshi_sdk_rs::{margin::models::*, FixedPointCount, FixedPointDollars};
use serde_json::json;

#[test]
fn margin_orders_keep_dollar_prices_and_fractional_counts_as_strings() {
    let request = CreateMarginOrderRequest {
        ticker: Some("BTC-PERP".into()),
        client_order_id: "client-order-1".into(),
        side: BookSide::Ask,
        price: "65432.1234".parse().unwrap(),
        count: "2.50".parse().unwrap(),
        time_in_force: CreateMarginOrderRequestTimeInForce::ImmediateOrCancel,
        self_trade_prevention_type: SelfTradePreventionType::TakerAtCross,
        reduce_only: Some(true),
        market_version: Some(7),
        ..Default::default()
    };
    let value = serde_json::to_value(request).unwrap();
    assert_eq!(value["price"], "65432.1234");
    assert_eq!(value["count"], "2.50");
    assert_eq!(value["side"], "ask");
    assert_eq!(value["reduce_only"], true);
    assert_eq!(value["market_version"], 7);
    assert!(value.get("market_id").is_none());
    assert!(value.get("yes_price").is_none());
    assert!(value.get("no_price").is_none());

    for (field, invalid) in [("price", json!(65432.1234)), ("count", json!(2.5))] {
        let mut malformed = value.clone();
        malformed[field] = invalid;
        assert!(serde_json::from_value::<CreateMarginOrderRequest>(malformed).is_err());
    }
    assert_eq!(
        "-2.50".parse::<FixedPointCount>().unwrap().to_string(),
        "-2.50"
    );
    assert_eq!(
        "65432.123456"
            .parse::<FixedPointDollars>()
            .unwrap()
            .to_string(),
        "65432.123456"
    );
}

#[test]
fn continuously_trading_markets_accept_a_null_referenced_schedule() {
    let mut value = json!({
        "ticker": "BTC-PERP",
        "title": "Bitcoin perpetual future",
        "status": "active",
        "contract_size": "0.000010",
        "underlying_multiplier": "1",
        "tick_size": "0.0100",
        "fractional_trading_enabled": true,
        "minimum_quantity": "0.01",
        "schedule": null,
        "market_version": 7,
        "funding_deadband": 0.000001,
        "funding_cap": 0,
        "funding_limit_unit": "percentage",
        "funding_invalid_premium_handling": "exclude",
        "halt_sources": []
    });
    value["funding_cap"] = serde_json::from_str("0.01234567890123456789").unwrap();
    let market: MarginMarket = serde_json::from_value(value.clone()).unwrap();
    assert!(market.schedule.is_none());
    assert_eq!(market.tick_size.to_string(), "0.0100");
    assert!(market.exchange_index.is_none());
    assert_eq!(market.funding_cap.to_string(), "0.01234567890123456789");

    let mut scheduled = value.clone();
    scheduled["schedule"] =
        json!({"is_open": true, "next_open_ts": null, "next_close_ts": 1800000000});
    let market: MarginMarket = serde_json::from_value(scheduled).unwrap();
    let schedule = market.schedule.unwrap();
    assert!(schedule.is_open);
    assert_eq!(schedule.next_open_ts, None);
    assert_eq!(schedule.next_close_ts, Some(1800000000));

    let mut missing = value;
    missing.as_object_mut().unwrap().remove("minimum_quantity");
    assert!(serde_json::from_value::<MarginMarket>(missing).is_err());
}

#[test]
fn margin_cursors_and_future_enum_values_remain_usable() {
    for cursor in [None, Some("")] {
        let mut value = json!({"fills": []});
        if let Some(cursor) = cursor {
            value["cursor"] = json!(cursor);
        }
        let response: GetMarginFillsResponse = serde_json::from_value(value).unwrap();
        assert!(response.next_cursor().is_none());
    }
    let response: GetMarginFillsResponse = serde_json::from_value(json!({
        "fills": [], "cursor": "next+/=?"
    }))
    .unwrap();
    assert_eq!(response.next_cursor(), Some("next+/=?"));
    let source: OrderSource = serde_json::from_value(json!("future-source")).unwrap();
    assert_eq!(source, OrderSource::Unknown("future-source".into()));
    assert_eq!(
        serde_json::to_value(source).unwrap(),
        json!("future-source")
    );
    let reason: OrderReason = serde_json::from_value(json!("future-reason")).unwrap();
    assert_eq!(reason, OrderReason::Unknown("future-reason".into()));
    assert_eq!(
        serde_json::to_value(reason).unwrap(),
        json!("future-reason")
    );
}

#[test]
fn margin_books_and_system_orders_keep_their_own_wire_semantics() {
    let book: MarginOrderbookResponse = serde_json::from_value(json!({"orderbook": {
        "bids": [["65432.1234", "2.50"]], "asks": [["65433.0000", "1.00"]]
    }}))
    .unwrap();
    assert_eq!(book.orderbook.bids[0].0.to_string(), "65432.1234");
    assert_eq!(book.orderbook.bids[0].1.to_string(), "2.50");
    assert_eq!(book.orderbook.asks[0].0.to_string(), "65433.0000");
    assert!(
        serde_json::from_value::<MarginOrderbookResponse>(json!({"orderbook": {
            "bids": [["65432.1234"]], "asks": []
        }}))
        .is_err()
    );

    let order: MarginOrder = serde_json::from_value(json!({
        "order_id": "system-order", "user_id": "user", "client_order_id": "system-client",
        "ticker": "BTC-PERP", "side": "ask", "price": "65000.0000",
        "fill_count": "1.00", "remaining_count": "0.00", "last_update_reason": "Liquidation",
        "order_source": "system", "order_reason": "liquidation", "expiration_time": null,
        "created_time": "2026-11-16T12:00:00Z"
    }))
    .unwrap();
    assert_eq!(order.order_source, Some(OrderSource::System));
    assert_eq!(order.order_reason, Some(OrderReason::Liquidation));
    assert_eq!(order.created_time.as_deref(), Some("2026-11-16T12:00:00Z"));
    assert!(order.expiration_time.is_none());
}
