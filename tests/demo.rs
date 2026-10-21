//! Optional integration checks. Every client is hardcoded to Demo; no production writes.
use kalshi_sdk_rs::{models::*, Client, Credentials, Environment};

#[tokio::test]
#[ignore = "requires demo credentials and network; run explicitly"]
async fn demo_authenticated_read_only() {
    let client = Client::builder(Environment::Demo)
        .credentials(Credentials::from_env().unwrap())
        .build()
        .unwrap();
    client.get_balance(&Default::default()).await.unwrap();
    client.get_orders(&Default::default()).await.unwrap();
    client.get_positions(&Default::default()).await.unwrap();
    client.get_fills(&Default::default()).await.unwrap();
    let mut socket = client.connect_websocket().await.unwrap();
    socket
        .subscribe(
            1,
            kalshi_sdk_rs::websocket::SubscribeParams {
                channels: vec![kalshi_sdk_rs::websocket::Channel::UserOrders],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let message = tokio::time::timeout(std::time::Duration::from_secs(15), socket.next_message())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        message,
        Some(kalshi_sdk_rs::websocket::Message::Subscribed(_))
    ));
    socket.close().await.unwrap();
}

#[tokio::test]
#[ignore = "submits a demo order; requires explicit KALSHI_DEMO_ORDER_OPT_IN=YES"]
async fn demo_create_and_cancel_order() {
    assert_eq!(
        std::env::var("KALSHI_DEMO_ORDER_OPT_IN").as_deref(),
        Ok("YES"),
        "explicit demo trading opt-in is required"
    );
    let ticker = std::env::var("KALSHI_MARKET_TICKER").unwrap();
    let price = std::env::var("KALSHI_DEMO_ORDER_PRICE")
        .unwrap_or_else(|_| "0.0100".into())
        .parse()
        .unwrap();
    let expiry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 60;
    let client = Client::builder(Environment::Demo)
        .credentials(Credentials::from_env().unwrap())
        .build()
        .unwrap();
    let order = client
        .create_order_v2(&CreateOrderV2Request {
            ticker: ticker.clone(),
            side: BookSide::Bid,
            count: "1.00".parse().unwrap(),
            price,
            time_in_force: CreateOrderV2RequestTimeInForce::GoodTillCanceled,
            self_trade_prevention_type: SelfTradePreventionType::TakerAtCross,
            client_order_id: Some(uuid::Uuid::new_v4().to_string()),
            post_only: Some(true),
            expiration_time: Some(i64::try_from(expiry).unwrap()),
            ..Default::default()
        })
        .await
        .unwrap();
    client
        .cancel_order_v2(
            &order.order_id,
            &CancelOrderV2Query {
                market_ticker: Some(ticker),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}
