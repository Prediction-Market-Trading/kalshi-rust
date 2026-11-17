//! Optional perps integration checks. All clients use Demo; ordinary tests stay offline.
use kalshi_sdk_rs::{
    margin::{
        models::*,
        websocket::{Channel, Message, SubscribeParams},
    },
    Client, Credentials, Environment,
};

#[tokio::test]
#[ignore = "requires margin-enabled demo credentials and network; run explicitly"]
async fn margin_demo_authenticated_read_only() {
    let client = Client::builder(Environment::Demo)
        .credentials(Credentials::from_env().unwrap())
        .build()
        .unwrap();
    let margin = client.margin();
    assert!(
        margin.get_enabled().await.unwrap().enabled,
        "margin must be enabled for this demo account"
    );
    margin.get_balance(&Default::default()).await.unwrap();
    margin.get_orders(&Default::default()).await.unwrap();
    margin.get_positions(&Default::default()).await.unwrap();
    margin.get_fills(&Default::default()).await.unwrap();
    margin.get_risk().await.unwrap();
    let mut socket = margin.connect_websocket().await.unwrap();
    socket
        .subscribe(
            1,
            SubscribeParams {
                channels: vec![Channel::UserOrders],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let message = tokio::time::timeout(std::time::Duration::from_secs(15), socket.next_message())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(message, Some(Message::Subscribed(_))));
    socket.close().await.unwrap();
    while tokio::time::timeout(std::time::Duration::from_secs(5), socket.next_message())
        .await
        .unwrap()
        .unwrap()
        .is_some()
    {}
}

#[tokio::test]
#[ignore = "submits a demo perps order; requires KALSHI_MARGIN_DEMO_ORDER_OPT_IN=YES"]
async fn margin_demo_create_and_cancel_order() {
    assert_eq!(
        std::env::var("KALSHI_MARGIN_DEMO_ORDER_OPT_IN").as_deref(),
        Ok("YES"),
        "explicit margin demo trading opt-in is required"
    );
    let ticker =
        std::env::var("KALSHI_MARGIN_MARKET_TICKER").expect("choose an open demo perps market");
    let price = std::env::var("KALSHI_MARGIN_DEMO_ORDER_PRICE")
        .expect("choose a valid, non-crossing demo perps price")
        .parse()
        .unwrap();
    let count = std::env::var("KALSHI_MARGIN_DEMO_ORDER_COUNT")
        .unwrap_or_else(|_| "1.00".into())
        .parse()
        .unwrap();
    let client = Client::builder(Environment::Demo)
        .credentials(Credentials::from_env().unwrap())
        .build()
        .unwrap();
    let margin = client.margin();
    assert!(
        margin.get_enabled().await.unwrap().enabled,
        "margin must be enabled for this demo account"
    );
    let market = margin.get_market(&ticker).await.unwrap();
    let expiry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 60;
    let order = margin
        .create_order(&CreateMarginOrderRequest {
            ticker: Some(ticker),
            client_order_id: uuid::Uuid::new_v4().to_string(),
            side: BookSide::Bid,
            price,
            count,
            time_in_force: CreateMarginOrderRequestTimeInForce::GoodTillCanceled,
            self_trade_prevention_type: SelfTradePreventionType::TakerAtCross,
            expiration_time: Some(i64::try_from(expiry).unwrap()),
            post_only: Some(true),
            market_version: Some(market.market.market_version),
            ..Default::default()
        })
        .await
        .unwrap();
    margin
        .cancel_order(&order.order_id, &Default::default())
        .await
        .unwrap();
}
