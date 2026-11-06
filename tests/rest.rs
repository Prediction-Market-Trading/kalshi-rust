mod common;
use base64::{engine::general_purpose::STANDARD, Engine};
use kalshi_sdk_rs::{models::*, Client, Environment, Error};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde_json::json;
use wiremock::{
    matchers::{method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

async fn client(server: &MockServer) -> Client {
    Client::builder(Environment::Demo)
        .base_url(format!("{}/trade-api/v2", server.uri()))
        .build()
        .unwrap()
}

fn assert_signed_request(
    request: &wiremock::Request,
    public: &[u8],
    expected_method: &str,
    canonical_path: &str,
) {
    assert_eq!(request.method.as_str(), expected_method);
    assert_eq!(request.url.path(), canonical_path);
    assert_eq!(request.headers["kalshi-access-key"], "test-key-id");
    let timestamp = request.headers["kalshi-access-timestamp"].to_str().unwrap();
    assert!(timestamp.parse::<u64>().unwrap() > 0);
    // Expected paths are specified by the caller, independently of the SDK encoder and URL.
    let message = format!("{timestamp}{expected_method}{canonical_path}");
    let signature = STANDARD
        .decode(request.headers["kalshi-access-signature"].to_str().unwrap())
        .unwrap();
    let verifier = UnparsedPublicKey::new(&ED25519, public);
    verifier.verify(message.as_bytes(), &signature).unwrap();
    if let Some(query) = request.url.query() {
        assert!(verifier
            .verify(format!("{message}?{query}").as_bytes(), &signature)
            .is_err());
    }
}

#[tokio::test]
async fn public_request_pagination_and_reserved_query_characters() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/trade-api/v2/markets"))
        .and(query_param("status", "open"))
        .and(query_param("cursor", "a+/=?"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"markets":[],"cursor":""})))
        .expect(1)
        .mount(&server)
        .await;
    let response = client(&server)
        .await
        .get_markets(&GetMarketsQuery {
            status: Some(GetMarketsQueryStatus::Open),
            cursor: Some("a+/=?".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(response.next_cursor().is_none());
    let requests = server.received_requests().await.unwrap();
    assert!(requests[0].headers.get("kalshi-access-key").is_none());
}

#[tokio::test]
async fn arrays_use_repeated_query_keys() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":{"code":"test"}})))
        .mount(&server)
        .await;
    let _ = client(&server)
        .await
        .get_market_orderbooks(&GetMarketOrderbooksQuery {
            tickers: vec!["A/B".into(), "C,D".into()],
        })
        .await;
    let requests = server.received_requests().await.unwrap();
    let values: Vec<_> = requests[0]
        .url
        .query_pairs()
        .filter(|(k, _)| k == "tickers")
        .map(|(_, v)| v.into_owned())
        .collect();
    assert_eq!(values, vec!["A/B", "C,D"]);
}

#[tokio::test]
async fn configured_credentials_sign_public_requests_with_canonical_encoded_segments() {
    let server = MockServer::start().await;
    let (auth, public) = common::credentials();
    let client = Client::builder(Environment::Demo)
        .base_url(format!("{}/trade-api/v2", server.uri()))
        .credentials(auth)
        .build()
        .unwrap();
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"error":{"code":"not_found"}})),
        )
        .mount(&server)
        .await;
    let cases = [
        (
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~",
            "/trade-api/v2/markets/ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~",
        ),
        (
            ":/?#[]@!$&'()*+,;= %\\",
            "/trade-api/v2/markets/%3A%2F%3F%23%5B%5D%40%21%24%26%27%28%29%2A%2B%2C%3B%3D%20%25%5C",
        ),
        ("café/東京🦀", "/trade-api/v2/markets/caf%C3%A9%2F%E6%9D%B1%E4%BA%AC%F0%9F%A6%80"),
        ("%2F%2D%2E", "/trade-api/v2/markets/%252F%252D%252E"),
        ("...", "/trade-api/v2/markets/..."),
    ];
    for (identifier, _) in cases {
        assert!(matches!(
            client.get_market(identifier).await,
            Err(Error::Api { status: 404, .. })
        ));
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), cases.len());
    for (request, (_, canonical_path)) in requests.iter().zip(cases) {
        assert!(request.url.query().is_none());
        assert_signed_request(request, &public, "GET", canonical_path);
    }
    for identifier in ["", ".", ".."] {
        assert!(matches!(
            client.get_market(identifier).await,
            Err(Error::Config(_))
        ));
    }
    assert_eq!(server.received_requests().await.unwrap().len(), cases.len());
}

#[tokio::test]
async fn single_transfer_get_and_single_order_delete_sign_literal_uuid_hyphens() {
    const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";
    const TRANSFER_PATH: &str = "/trade-api/v2/portfolio/intra_exchange_instance_transfers/123e4567-e89b-12d3-a456-426614174000";
    const ORDER_PATH: &str =
        "/trade-api/v2/portfolio/events/orders/123e4567-e89b-12d3-a456-426614174000";
    let server = MockServer::start().await;
    let (auth, public) = common::credentials();
    let client = Client::builder(Environment::Demo)
        .base_url(format!("{}/trade-api/v2/", server.uri()))
        .credentials(auth)
        .build()
        .unwrap();
    Mock::given(method("GET"))
        .and(path(TRANSFER_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transfer": {
                "transfer_id": UUID,
                "source": "event_contract",
                "destination": "event_contract",
                "source_exchange_shard": 0,
                "destination_exchange_shard": 1,
                "amount": "1.00",
                "status": "complete",
                "created_ts": 1
            }
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(ORDER_PATH))
        .and(query_param("market_ticker", "TICKER/A ?#%"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "order_id": UUID, "reduced_by": "2.50", "ts_ms": 2
        })))
        .expect(1)
        .mount(&server)
        .await;

    let transfer = client
        .get_intra_exchange_instance_transfer(UUID)
        .await
        .unwrap();
    assert_eq!(transfer.transfer.transfer_id, UUID);
    let order = client
        .cancel_order_v2(
            UUID,
            &CancelOrderV2Query {
                market_ticker: Some("TICKER/A ?#%".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(order.order_id, UUID);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].url.query().is_none());
    assert_signed_request(&requests[0], &public, "GET", TRANSFER_PATH);
    assert!(requests[1].url.query().is_some());
    assert_signed_request(&requests[1], &public, "DELETE", ORDER_PATH);
    for identifier in ["", ".", ".."] {
        assert!(matches!(
            client
                .get_intra_exchange_instance_transfer(identifier)
                .await,
            Err(Error::Config(_))
        ));
        assert!(matches!(
            client
                .cancel_order_v2(identifier, &Default::default())
                .await,
            Err(Error::Config(_))
        ));
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn missing_auth_fails_before_network() {
    let server = MockServer::start().await;
    assert!(matches!(
        client(&server).await.get_balance(&Default::default()).await,
        Err(Error::Auth(_))
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn api_errors_preserve_status_details_retry_after_and_non_json_body() {
    let server = MockServer::start().await;
    Mock::given(path("/trade-api/v2/exchange/status")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After","3").set_body_json(json!({"error":{"code":"too_many_requests","message":"slow down","details":{"budget":0}}}))).expect(1).mount(&server).await;
    match client(&server)
        .await
        .get_exchange_status()
        .await
        .unwrap_err()
    {
        Error::Api {
            status,
            error,
            retry_after,
            ..
        } => {
            assert_eq!(status, 429);
            assert_eq!(error.code.as_deref(), Some("too_many_requests"));
            assert_eq!(error.details, Some(json!({"budget":0})));
            assert_eq!(retry_after.as_deref(), Some("3"));
        }
        e => panic!("unexpected error {e:?}"),
    }
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(502).set_body_string("upstream unavailable"))
        .mount(&server)
        .await;
    assert!(
        matches!(client(&server).await.get_exchange_status().await,Err(Error::Api {status:502,body,..}) if body=="upstream unavailable")
    );
}

#[tokio::test]
async fn invalid_success_body_is_a_decode_error_and_redirects_are_not_followed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"invalid":true})))
        .mount(&server)
        .await;
    assert!(matches!(
        client(&server).await.get_exchange_status().await,
        Err(Error::Json(_))
    ));
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/elsewhere"))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        client(&server).await.get_exchange_status().await,
        Err(Error::Api { status: 302, .. })
    ));
}

#[tokio::test]
async fn trading_uses_v2_paths_bodies_and_empty_success_responses() {
    let server = MockServer::start().await;
    let (auth, _) = common::credentials();
    let client = Client::builder(Environment::Demo)
        .base_url(format!("{}/trade-api/v2", server.uri()))
        .credentials(auth)
        .build()
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/trade-api/v2/portfolio/events/orders"))
        .respond_with(ResponseTemplate::new(201).set_body_json(
            json!({"order_id":"id-1","fill_count":"0.00","remaining_count":"2.50","ts_ms":1}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let order = client
        .create_order_v2(&CreateOrderV2Request {
            ticker: "TICKER".into(),
            count: "2.50".parse().unwrap(),
            price: "0.1234".parse().unwrap(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(order.order_id, "id-1");
    Mock::given(method("DELETE"))
        .and(path("/trade-api/v2/portfolio/events/orders/id-1"))
        .and(query_param("market_ticker", "TICKER"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"order_id":"id-1","reduced_by":"2.50","ts_ms":2})),
        )
        .expect(1)
        .mount(&server)
        .await;
    client
        .cancel_order_v2(
            &order.order_id,
            &CancelOrderV2Query {
                market_ticker: Some("TICKER".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    Mock::given(method("DELETE"))
        .and(path("/trade-api/v2/portfolio/events/orders"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    client.cancel_all_orders(&Default::default()).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["count"], "2.50");
    assert_eq!(body["price"], "0.1234");
}

#[tokio::test]
async fn amend_and_decrease_keep_v2_total_and_reduction_semantics() {
    let server = MockServer::start().await;
    let (auth, _) = common::credentials();
    let client = Client::builder(Environment::Demo)
        .base_url(format!("{}/trade-api/v2", server.uri()))
        .credentials(auth)
        .build()
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/trade-api/v2/portfolio/events/orders/O/amend"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "order_id":"O", "remaining_count": null, "fill_count": null, "ts_ms":5
        })))
        .expect(1)
        .mount(&server)
        .await;
    let amend = client
        .amend_order_v2(
            "O",
            &Default::default(),
            &AmendOrderV2Request {
                ticker: "T".into(),
                side: BookSide::Ask,
                price: "0.5600".parse().unwrap(),
                count: "8.25".parse().unwrap(),
                expiration_time: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(amend.remaining_count.is_none());
    Mock::given(method("POST"))
        .and(path("/trade-api/v2/portfolio/events/orders/O/decrease"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "order_id":"O", "remaining_count":"6.00", "ts_ms":6
        })))
        .expect(1)
        .mount(&server)
        .await;
    let decrease = client
        .decrease_order_v2(
            "O",
            &Default::default(),
            &DecreaseOrderV2Request {
                reduce_by: Some("2.25".parse().unwrap()),
                market_ticker: Some("T".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(decrease.remaining_count.to_string(), "6.00");
    let requests = server.received_requests().await.unwrap();
    let amendment: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(amendment["side"], "ask");
    assert_eq!(amendment["count"], "8.25");
    assert_eq!(amendment["expiration_time"], 0);
    let reduction: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(reduction["reduce_by"], "2.25");
    assert!(reduction.get("reduce_to").is_none());
}

#[tokio::test]
async fn request_timeout_surfaces_without_retry() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(1)))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::builder(Environment::Demo)
        .base_url(server.uri())
        .timeout(std::time::Duration::from_millis(50))
        .build()
        .unwrap();
    assert!(matches!(client.get_exchange_status().await, Err(Error::Http(e)) if e.is_timeout()));
}
