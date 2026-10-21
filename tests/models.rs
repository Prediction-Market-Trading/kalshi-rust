use kalshi_sdk_rs::{models::*, FixedPointCount, FixedPointDollars};
use serde_json::json;

#[test]
fn fixed_point_preserves_units_scale_and_exact_arithmetic() {
    let a: FixedPointDollars = "0.100001".parse().unwrap();
    let b: FixedPointDollars = "0.200002".parse().unwrap();
    assert_eq!(
        FixedPointDollars::new(a.decimal().checked_add(b.decimal()).unwrap())
            .unwrap()
            .to_string(),
        "0.300003"
    );
    let count: FixedPointCount = "-54.00".parse().unwrap();
    assert_eq!(serde_json::to_value(count).unwrap(), json!("-54.00"));
    assert_eq!(
        serde_json::to_value("0.5600".parse::<FixedPointDollars>().unwrap()).unwrap(),
        json!("0.5600")
    );
    for text in [
        "",
        "NaN",
        "1e-3",
        "+1",
        ".5",
        "1.",
        " 1",
        "--1",
        "1.0000001",
        "99999999999999999999999999999999999",
    ] {
        assert!(text.parse::<FixedPointDollars>().is_err(), "{text}");
    }
    assert!("1.001".parse::<FixedPointCount>().is_err());
    assert!(serde_json::from_value::<FixedPointDollars>(json!(0.5)).is_err());
    assert!(serde_json::from_value::<FixedPointCount>(json!(2)).is_err());
}

#[test]
fn market_status_query_uses_singular_status_and_omits_unset_fields() {
    let query = GetMarketsQuery {
        status: Some(GetMarketsQueryStatus::Open),
        limit: Some(10),
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_value(query).unwrap(),
        json!({"status":"open", "limit":10})
    );
}

#[test]
fn v2_order_has_book_side_and_string_price_and_count() {
    let request = CreateOrderV2Request {
        ticker: "EXAMPLE".into(),
        side: BookSide::Bid,
        price: "0.1234".parse().unwrap(),
        count: "2.50".parse().unwrap(),
        time_in_force: CreateOrderV2RequestTimeInForce::GoodTillCanceled,
        self_trade_prevention_type: SelfTradePreventionType::TakerAtCross,
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({"ticker":"EXAMPLE","side":"bid","price":"0.1234","count":"2.50","time_in_force":"good_till_canceled","self_trade_prevention_type":"taker_at_cross"})
    );
}

#[test]
fn pagination_handles_missing_and_empty_cursor() {
    let empty: GetMarketsResponse =
        serde_json::from_value(json!({"markets":[],"cursor":""})).unwrap();
    assert!(empty.next_cursor().is_none());
    let missing: GetMarketsResponse = serde_json::from_value(json!({"markets":[]})).unwrap();
    assert!(missing.next_cursor().is_none());
    let next: GetMarketsResponse =
        serde_json::from_value(json!({"markets":[],"cursor":"a+/=?"})).unwrap();
    assert_eq!(next.next_cursor(), Some("a+/=?"));
}

#[test]
fn unknown_enum_values_roundtrip() {
    let side: BookSide = serde_json::from_value(json!("future-side")).unwrap();
    assert_eq!(side, BookSide::Unknown("future-side".into()));
    assert_eq!(serde_json::to_value(side).unwrap(), json!("future-side"));
}

#[test]
fn captured_public_api_responses_deserialize() {
    let _: ExchangeStatus =
        serde_json::from_str(include_str!("fixtures/public-status.json")).unwrap();
    let markets: GetMarketsResponse =
        serde_json::from_str(include_str!("fixtures/public-markets.json")).unwrap();
    assert!(!markets.markets.is_empty());
    let _: GetEventsResponse =
        serde_json::from_str(include_str!("fixtures/public-events.json")).unwrap();
    let _: GetTradesResponse =
        serde_json::from_str(include_str!("fixtures/public-trades.json")).unwrap();
    let _: GetMarketOrderbookResponse =
        serde_json::from_str(include_str!("fixtures/public-orderbook.json")).unwrap();
    let _: GetHistoricalCutoffResponse =
        serde_json::from_str(include_str!("fixtures/public-historical-cutoff.json")).unwrap();
}

#[test]
fn orderbook_levels_are_price_quantity_tuples() {
    let response: GetMarketOrderbookResponse = serde_json::from_value(
        json!({"orderbook_fp":{"yes_dollars":[["0.1234","2.50"]],"no_dollars":[]}}),
    )
    .unwrap();
    let (price, count) = response.orderbook_fp.yes_dollars[0];
    assert_eq!(price.to_string(), "0.1234");
    assert_eq!(count.to_string(), "2.50");
    assert!(serde_json::from_value::<GetMarketOrderbookResponse>(
        json!({"orderbook_fp":{"yes_dollars":[["0.1234"]],"no_dollars":[]}})
    )
    .is_err());
}
