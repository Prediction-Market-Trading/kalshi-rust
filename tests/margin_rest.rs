mod common;
use base64::{engine::general_purpose::STANDARD, Engine};
use kalshi_sdk_rs::{margin::models::*, Client, Environment, Error};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde_json::{json, Value};
use wiremock::{
    matchers::{method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

async fn authenticated(server: &MockServer) -> (Client, Vec<u8>) {
    let (credentials, public) = common::credentials();
    let client = Client::builder(Environment::Demo)
        .base_url(format!("{}/trade-api/v2/", server.uri()))
        .credentials(credentials)
        .build()
        .unwrap();
    (client, public)
}

fn assert_signed(request: &wiremock::Request, public: &[u8], verb: &str, expected_path: &str) {
    assert_eq!(request.method.as_str(), verb);
    assert_eq!(request.url.path(), expected_path);
    assert_eq!(request.headers["kalshi-access-key"], "test-key-id");
    let timestamp = request.headers["kalshi-access-timestamp"].to_str().unwrap();
    assert!(timestamp.parse::<u64>().unwrap() > 0);
    let signature = STANDARD
        .decode(request.headers["kalshi-access-signature"].to_str().unwrap())
        .unwrap();
    let verifier = UnparsedPublicKey::new(&ED25519, public);
    let signed = format!("{timestamp}{verb}{expected_path}");
    verifier.verify(signed.as_bytes(), &signature).unwrap();
    if let Some(query) = request.url.query() {
        assert!(verifier
            .verify(format!("{signed}?{query}").as_bytes(), &signature)
            .is_err());
    }
}

fn body(request: &wiremock::Request) -> Value {
    serde_json::from_slice(&request.body).unwrap()
}

#[tokio::test]
async fn public_margin_filters_and_authentication_are_independent_of_predictions() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/trade-api/v2/margin/markets"))
        .and(query_param("status", "active"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"markets": []})))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::builder(Environment::Demo)
        .base_url(format!("{}/trade-api/v2", server.uri()))
        .build()
        .unwrap();
    let page = client
        .margin()
        .get_markets(&GetMarginMarketsQuery {
            status: Some(MarginMarketStatus::Active),
        })
        .await
        .unwrap();
    assert!(page.markets.is_empty());
    assert!(matches!(
        client.margin().get_enabled().await,
        Err(Error::Auth(_))
    ));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].headers.get("kalshi-access-key").is_none());
}

#[tokio::test]
async fn margin_ticker_arrays_use_one_comma_separated_query_value() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {"code": "test"}})))
        .mount(&server)
        .await;
    let (client, public) = authenticated(&server).await;
    let tickers = vec!["BTC/PERP".into(), "ETH +%".into()];
    assert!(matches!(
        client
            .margin()
            .get_market_schedules(&GetMarginMarketSchedulesQuery {
                tickers: tickers.clone()
            })
            .await,
        Err(Error::Api { status: 400, .. })
    ));
    assert!(matches!(
        client
            .margin()
            .get_market_orderbooks(&GetMarginMarketOrderbooksQuery { tickers })
            .await,
        Err(Error::Api { status: 400, .. })
    ));
    let requests = server.received_requests().await.unwrap();
    for (request, expected_path) in requests.iter().zip([
        "/trade-api/v2/margin/markets/schedules",
        "/trade-api/v2/margin/markets/orderbooks",
    ]) {
        let values: Vec<_> = request
            .url
            .query_pairs()
            .filter(|(name, _)| name == "tickers")
            .map(|(_, value)| value.into_owned())
            .collect();
        assert_eq!(values, vec!["BTC/PERP,ETH +%"]);
        assert_signed(request, &public, "GET", expected_path);
    }
}

#[tokio::test]
async fn margin_public_requests_sign_the_exact_encoded_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"error": {"code": "not_found"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (client, public) = authenticated(&server).await;
    assert!(matches!(
        client.margin().get_market("BTC/A ?#%").await,
        Err(Error::Api { status: 404, .. })
    ));
    let requests = server.received_requests().await.unwrap();
    assert_signed(
        &requests[0],
        &public,
        "GET",
        "/trade-api/v2/margin/markets/BTC%2FA%20%3F%23%25",
    );
    for ticker in ["", ".", ".."] {
        assert!(matches!(
            client.margin().get_market(ticker).await,
            Err(Error::Config(_))
        ));
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn margin_fills_preserve_opaque_pagination_and_subaccount_filters() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/trade-api/v2/margin/fills"))
        .and(query_param("cursor", "next+/=?"))
        .and(query_param("subaccount", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"fills": [], "cursor": "following+/=?"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (client, public) = authenticated(&server).await;
    let page = client
        .margin()
        .get_fills(&GetMarginFillsQuery {
            cursor: Some("next+/=?".into()),
            subaccount: Some(2),
            limit: Some(10),
            min_ts: Some(1800000000),
            max_ts: Some(1800000060),
        })
        .await
        .unwrap();
    assert_eq!(page.next_cursor(), Some("following+/=?"));
    let requests = server.received_requests().await.unwrap();
    assert_signed(&requests[0], &public, "GET", "/trade-api/v2/margin/fills");
    assert_eq!(requests[0].url.query_pairs().count(), 5);
}

#[tokio::test]
async fn perps_limits_and_transfers_use_their_documented_non_margin_prefixes() {
    let server = MockServer::start().await;
    let (client, public) = authenticated(&server).await;
    Mock::given(method("GET"))
        .and(path("/trade-api/v2/account/limits/perps"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "usage_tier": "basic", "read": {"refill_rate": 20, "bucket_capacity": 20},
            "write": {"refill_rate": 10, "bucket_capacity": 10}, "grants": []
        })))
        .expect(1)
        .mount(&server)
        .await;
    let limits = client.margin().get_account_api_limits().await.unwrap();
    assert_eq!(limits.read.refill_rate, 20);
    Mock::given(method("POST"))
        .and(path("/trade-api/v2/portfolio/margin/subaccounts"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"subaccount_number": 2})))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        client
            .margin()
            .create_subaccount()
            .await
            .unwrap()
            .subaccount_number,
        2
    );
    Mock::given(method("POST"))
        .and(path("/trade-api/v2/portfolio/margin/subaccounts/transfer"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    client
        .margin()
        .apply_subaccount_transfer(&ApplySubaccountTransferRequest {
            client_transfer_id: "transfer-client".into(),
            from_subaccount: 0,
            to_subaccount: 2,
            amount_cents: 12345,
            ..Default::default()
        })
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path(
            "/trade-api/v2/portfolio/intra_exchange_instance_transfer",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"transfer_id": "transfer-1"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let transfer = client
        .margin()
        .intra_exchange_instance_transfer(&IntraExchangeInstanceTransferRequest {
            source: ExchangeInstance::EventContract,
            destination: ExchangeInstance::Margined,
            amount: 123456,
            client_transfer_id: Some("transfer-client-2".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(transfer.transfer_id, "transfer-1");
    let requests = server.received_requests().await.unwrap();
    for (request, (verb, expected_path)) in requests.iter().zip([
        ("GET", "/trade-api/v2/account/limits/perps"),
        ("POST", "/trade-api/v2/portfolio/margin/subaccounts"),
        (
            "POST",
            "/trade-api/v2/portfolio/margin/subaccounts/transfer",
        ),
        (
            "POST",
            "/trade-api/v2/portfolio/intra_exchange_instance_transfer",
        ),
    ]) {
        assert_signed(request, &public, verb, expected_path);
    }
    assert!(requests[1].body.is_empty());
    assert_eq!(
        body(&requests[2]),
        json!({
            "client_transfer_id": "transfer-client", "from_subaccount": 0,
            "to_subaccount": 2, "amount_cents": 12345
        })
    );
    assert_eq!(body(&requests[3])["amount"], 123456);
    assert_eq!(body(&requests[3])["destination"], "margined");
}

#[tokio::test]
async fn margin_order_lifecycle_preserves_total_counts_reductions_and_empty_success() {
    let server = MockServer::start().await;
    let (client, public) = authenticated(&server).await;
    Mock::given(method("POST"))
        .and(path("/trade-api/v2/margin/orders"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "order_id": "order-1", "fill_count": "1.25", "remaining_count": "7.00",
            "average_fill_price": "65432.123456", "average_fee_paid": "0.001234"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let order = client
        .margin()
        .create_order(&CreateMarginOrderRequest {
            ticker: Some("BTC-PERP".into()),
            client_order_id: "client-order-1".into(),
            side: BookSide::Bid,
            count: "8.25".parse().unwrap(),
            price: "65432.1234".parse().unwrap(),
            time_in_force: CreateMarginOrderRequestTimeInForce::GoodTillCanceled,
            self_trade_prevention_type: SelfTradePreventionType::TakerAtCross,
            post_only: Some(true),
            subaccount: Some(2),
            market_version: Some(7),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(order.fill_count.to_string(), "1.25");
    assert_eq!(
        order.average_fill_price.unwrap().to_string(),
        "65432.123456"
    );
    Mock::given(method("POST"))
        .and(path("/trade-api/v2/margin/orders/order-1/amend"))
        .and(query_param("subaccount", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "order_id": "order-1", "remaining_count": null, "fill_count": null,
            "average_fill_price": null, "average_fee_paid": null
        })))
        .expect(1)
        .mount(&server)
        .await;
    let amendment = client
        .margin()
        .amend_order(
            "order-1",
            &AmendMarginOrderQuery {
                subaccount: Some(2),
            },
            &AmendMarginOrderRequest {
                ticker: Some("BTC-PERP".into()),
                side: BookSide::Bid,
                price: "65433.0000".parse().unwrap(),
                count: "8.25".parse().unwrap(),
                expiration_time: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(amendment.remaining_count.is_none());
    Mock::given(method("POST"))
        .and(path("/trade-api/v2/margin/orders/order-1/decrease"))
        .and(query_param("subaccount", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"order_id": "order-1", "remaining_count": "5.00"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let decrease = client
        .margin()
        .decrease_order(
            "order-1",
            &DecreaseMarginOrderQuery {
                subaccount: Some(2),
            },
            &DecreaseMarginOrderRequest {
                reduce_by: Some("2.00".parse().unwrap()),
                reduce_to: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(decrease.remaining_count.to_string(), "5.00");
    Mock::given(method("DELETE"))
        .and(path("/trade-api/v2/margin/orders/order-1"))
        .and(query_param("subaccount", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"order_id": "order-1", "reduced_by": "5.00"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let canceled = client
        .margin()
        .cancel_order(
            "order-1",
            &CancelMarginOrderQuery {
                subaccount: Some(2),
            },
        )
        .await
        .unwrap();
    assert_eq!(canceled.reduced_by.to_string(), "5.00");
    Mock::given(method("DELETE"))
        .and(path("/trade-api/v2/margin/orders"))
        .and(query_param("subaccount", "2"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    client
        .margin()
        .cancel_all_orders(&CancelAllMarginOrdersQuery {
            subaccount: Some(2),
        })
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    for (request, (verb, expected_path)) in requests.iter().zip([
        ("POST", "/trade-api/v2/margin/orders"),
        ("POST", "/trade-api/v2/margin/orders/order-1/amend"),
        ("POST", "/trade-api/v2/margin/orders/order-1/decrease"),
        ("DELETE", "/trade-api/v2/margin/orders/order-1"),
        ("DELETE", "/trade-api/v2/margin/orders"),
    ]) {
        assert_signed(request, &public, verb, expected_path);
    }
    assert_eq!(body(&requests[0])["price"], "65432.1234");
    assert_eq!(body(&requests[0])["count"], "8.25");
    assert_eq!(body(&requests[0])["market_version"], 7);
    assert_eq!(body(&requests[1])["count"], "8.25");
    assert_eq!(body(&requests[1])["expiration_time"], 0);
    assert_eq!(body(&requests[2]), json!({"reduce_by": "2.00"}));
}

fn trigger_response(kind: &str) -> Value {
    json!({"id": "trigger-1", "ticker": "BTC-PERP", "kind": kind, "status": "active",
        "count": "2.50", "filled_count": "0.00", "created_time": "2026-11-16T12:00:00Z",
        "updated_time": "2026-11-16T12:00:00Z"})
}

#[tokio::test]
async fn cross_and_isolated_exit_triggers_use_distinct_routes_and_shapes() {
    let server = MockServer::start().await;
    let (client, public) = authenticated(&server).await;
    Mock::given(method("PUT"))
        .and(path(
            "/trade-api/v2/margin/isolated/positions/BTC-PERP/exit_trigger",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(trigger_response("trailing")))
        .expect(1)
        .mount(&server)
        .await;
    let isolated = client
        .margin()
        .set_isolated_exit_trigger(
            "BTC-PERP",
            &SetIsolatedExitTriggerRequest {
                kind: Some(ExitTriggerKind::Trailing),
                trail_amount: Some("100.0000".parse().unwrap()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(isolated.kind, ExitTriggerKind::Trailing);
    Mock::given(method("PUT"))
        .and(path(
            "/trade-api/v2/margin/cross/positions/BTC-PERP/exit_trigger",
        ))
        .and(query_param("subaccount", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(trigger_response("bracket")))
        .expect(1)
        .mount(&server)
        .await;
    let cross = client
        .margin()
        .set_cross_exit_trigger(
            "BTC-PERP",
            &SetCrossMarginExitTriggerQuery {
                subaccount: Some(2),
            },
            &SetCrossExitTriggerRequest {
                count: Some("2.50".parse().unwrap()),
                anchor_order_id: Some("entry-order".into()),
                client_trigger_id: Some("client-trigger-1".into()),
                stop_loss_price: Some("64000.0000".parse().unwrap()),
                take_profit_price: Some("67000.0000".parse().unwrap()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(cross.status, ExitTriggerStatus::Active);
    Mock::given(method("PUT"))
        .and(path(
            "/trade-api/v2/margin/cross/positions/BTC-PERP/exit_trigger/trigger-1",
        ))
        .and(query_param("subaccount", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(trigger_response("bracket")))
        .expect(1)
        .mount(&server)
        .await;
    client
        .margin()
        .update_cross_exit_trigger(
            "BTC-PERP",
            "trigger-1",
            &UpdateCrossMarginExitTriggerQuery {
                subaccount: Some(2),
            },
            &UpdateExitTriggerRequest {
                stop_loss_price: Some("64500.0000".parse().unwrap()),
                take_profit_price: None,
            },
        )
        .await
        .unwrap();
    Mock::given(method("DELETE"))
        .and(path(
            "/trade-api/v2/margin/cross/positions/BTC-PERP/exit_trigger/trigger-1",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    client
        .margin()
        .delete_cross_exit_trigger_by_id("BTC-PERP", "trigger-1", &Default::default())
        .await
        .unwrap();
    Mock::given(method("DELETE"))
        .and(path(
            "/trade-api/v2/margin/isolated/positions/BTC-PERP/exit_trigger",
        ))
        .and(query_param("kind", "trailing"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    client
        .margin()
        .delete_isolated_exit_trigger(
            "BTC-PERP",
            &DeleteIsolatedMarginExitTriggerQuery {
                kind: Some(ExitTriggerKind::Trailing),
            },
        )
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        body(&requests[0]),
        json!({"kind": "trailing", "trail_amount": "100.0000"})
    );
    assert_eq!(body(&requests[1])["anchor_order_id"], "entry-order");
    assert_eq!(body(&requests[1])["count"], "2.50");
    assert_eq!(body(&requests[2]), json!({"stop_loss_price": "64500.0000"}));
    for (request, (verb, expected_path)) in requests.iter().zip([
        (
            "PUT",
            "/trade-api/v2/margin/isolated/positions/BTC-PERP/exit_trigger",
        ),
        (
            "PUT",
            "/trade-api/v2/margin/cross/positions/BTC-PERP/exit_trigger",
        ),
        (
            "PUT",
            "/trade-api/v2/margin/cross/positions/BTC-PERP/exit_trigger/trigger-1",
        ),
        (
            "DELETE",
            "/trade-api/v2/margin/cross/positions/BTC-PERP/exit_trigger/trigger-1",
        ),
        (
            "DELETE",
            "/trade-api/v2/margin/isolated/positions/BTC-PERP/exit_trigger",
        ),
    ]) {
        assert_signed(request, &public, verb, expected_path);
    }
}

#[tokio::test]
async fn margin_funding_risk_and_fcm_responses_keep_units_and_nullable_fields() {
    let server = MockServer::start().await;
    let (client, public) = authenticated(&server).await;
    Mock::given(method("GET")).and(path("/trade-api/v2/margin/funding_history"))
        .and(query_param("ticker", "BTC-PERP")).and(query_param("start_date", "2026-11-15"))
        .and(query_param("end_date", "2026-11-16")).and(query_param("subaccount", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"funding_history": [{
            "market_ticker": "BTC-PERP", "funding_time": "2026-11-16T00:00:00Z", "funding_rate": -0.0005,
            "mark_price": "65432.123456", "funding_amount": "-0.123456", "quantity": "-2.50",
            "subaccount_number": 2, "subaccount_type": "cross"
        }]}))).expect(1).mount(&server).await;
    let funding = client
        .margin()
        .get_funding_history(&GetMarginFundingHistoryQuery {
            ticker: Some("BTC-PERP".into()),
            start_date: "2026-11-15".into(),
            end_date: "2026-11-16".into(),
            subaccount: Some(2),
        })
        .await
        .unwrap();
    assert_eq!(funding.funding_history[0].quantity.to_string(), "-2.50");
    assert_eq!(
        funding.funding_history[0].funding_amount.to_string(),
        "-0.123456"
    );
    assert_eq!(
        funding.funding_history[0].funding_rate.to_string(),
        "-0.0005"
    );
    Mock::given(method("GET")).and(path("/trade-api/v2/margin/risk"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "account_leverage": null, "total_position_notional": "163580.308640",
            "total_maintenance_margin": "1000.000000", "positions": [{
                "subaccount": 2, "subaccount_type": "cross", "market_ticker": "BTC-PERP", "position": "-2.50",
                "mark_price": "65432.123456", "position_notional": "163580.308640", "is_portfolio": true,
                "maintenance_margin_required": null, "position_leverage": null, "estimated_liquidation_price": null
            }]
        }))).expect(1).mount(&server).await;
    let risk = client.margin().get_risk().await.unwrap();
    assert!(risk.account_leverage.is_none());
    assert_eq!(risk.positions[0].position.to_string(), "-2.50");
    assert!(risk.positions[0].estimated_liquidation_price.is_none());
    Mock::given(method("GET")).and(path("/trade-api/v2/margin/fcm/subtraders/risk_controls"))
        .and(query_param("subtrader_id", "subtrader-1")).and(query_param("market_ticker", "BTC-PERP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "risk_controls": [{"subtrader_id": "subtrader-1", "market_ticker": "BTC-PERP", "im_cap": "1000.0000", "current_im": "250.0000"}],
            "notional_limits": [{"subtrader_id": "subtrader-1", "market_ticker": "BTC-PERP", "notional_value_risk_limit": "5000.0000", "current_notional": "1250.0000"}]
        }))).expect(1).mount(&server).await;
    let controls = client
        .margin()
        .get_fcm_subtrader_risk_controls(&GetFCMSubtraderRiskControlsQuery {
            subtrader_id: Some("subtrader-1".into()),
            market_ticker: Some("BTC-PERP".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(controls.risk_controls[0].current_im.to_string(), "250.0000");
    assert_eq!(
        controls.notional_limits[0].current_notional.to_string(),
        "1250.0000"
    );
    let requests = server.received_requests().await.unwrap();
    for (request, expected_path) in requests.iter().zip([
        "/trade-api/v2/margin/funding_history",
        "/trade-api/v2/margin/risk",
        "/trade-api/v2/margin/fcm/subtraders/risk_controls",
    ]) {
        assert_signed(request, &public, "GET", expected_path);
    }
}

#[tokio::test]
async fn margin_rejections_and_transfer_restrictions_preserve_details_without_retrying() {
    let server = MockServer::start().await;
    let (client, _) = authenticated(&server).await;
    Mock::given(method("POST")).and(path("/trade-api/v2/margin/orders"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {
            "code": "NOTIONAL_RISK_LIMIT_EXCEEDED", "message": "notional limit exceeded", "details": {"limit": "100.0000"}
        }}))).expect(1).mount(&server).await;
    let error = client
        .margin()
        .create_order(&CreateMarginOrderRequest::default())
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Api { status: 400, error, .. } if error.code.as_deref() == Some("NOTIONAL_RISK_LIMIT_EXCEEDED") && error.details == Some(json!({"limit": "100.0000"})))
    );
    const DETAILS: &str =
        "margin withdrawals are temporarily disabled during settlement and funding";
    Mock::given(method("POST")).and(path("/trade-api/v2/portfolio/margin/subaccounts/transfer"))
        .respond_with(ResponseTemplate::new(403).insert_header("Retry-After", "5").set_body_json(json!({"error": {
            "code": "permission_denied", "message": "transfer unavailable", "details": DETAILS
        }}))).expect(1).mount(&server).await;
    let error = client
        .margin()
        .apply_subaccount_transfer(&Default::default())
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Api { status: 403, error, body, retry_after } if error.code.as_deref() == Some("permission_denied") && error.details == Some(json!(DETAILS)) && body.contains(DETAILS) && retry_after.as_deref() == Some("5"))
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}
