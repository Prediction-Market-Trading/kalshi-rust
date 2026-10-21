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
async fn configured_credentials_sign_public_requests_and_actual_encoded_path() {
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
    let _ = client.get_market("A/B ?#%").await;
    let requests = server.received_requests().await.unwrap();
    let request = &requests[0];
    assert_eq!(request.headers["kalshi-access-key"], "test-key-id");
    assert!(request.url.query().is_none());
    assert_eq!(
        request.url.path(),
        "/trade-api/v2/markets/A%2FB%20%3F%23%25"
    );
    let message = format!(
        "{}GET{}",
        request.headers["kalshi-access-timestamp"].to_str().unwrap(),
        request.url.path()
    );
    let signature = STANDARD
        .decode(request.headers["kalshi-access-signature"].to_str().unwrap())
        .unwrap();
    UnparsedPublicKey::new(&ED25519, public)
        .verify(message.as_bytes(), &signature)
        .unwrap();
    assert!(client.get_market("..").await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
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
        .and(path("/trade-api/v2/portfolio/events/orders/id%2D1"))
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
