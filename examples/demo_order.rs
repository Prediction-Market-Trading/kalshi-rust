mod common;
use kalshi_sdk_rs::models::{
    BookSide, CancelOrderV2Query, CreateOrderV2Request, CreateOrderV2RequestTimeInForce,
    SelfTradePreventionType,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("KALSHI_DEMO_ORDER_OPT_IN").as_deref() != Ok("YES") {
        return Err("Set KALSHI_DEMO_ORDER_OPT_IN=YES to submit and cancel a demo order".into());
    }
    let ticker = std::env::var("KALSHI_MARKET_TICKER")?;
    let price = std::env::var("KALSHI_DEMO_ORDER_PRICE")
        .unwrap_or_else(|_| "0.0100".into())
        .parse()?;
    let expiry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        + 60;
    // This client is always Demo. No environment variable can switch it to production.
    let client = common::authenticated_demo()?;
    let request = CreateOrderV2Request {
        ticker: ticker.clone(),
        side: BookSide::Bid,
        count: "1.00".parse()?,
        price,
        time_in_force: CreateOrderV2RequestTimeInForce::GoodTillCanceled,
        self_trade_prevention_type: SelfTradePreventionType::TakerAtCross,
        client_order_id: Some(uuid::Uuid::new_v4().to_string()),
        post_only: Some(true),
        expiration_time: Some(i64::try_from(expiry)?),
        ..Default::default()
    };
    let order = client.create_order_v2(&request).await?;
    println!("Created demo order {}", order.order_id);
    // Include the ticker to route to the correct exchange shard.
    let canceled = client
        .cancel_order_v2(
            &order.order_id,
            &CancelOrderV2Query {
                market_ticker: Some(ticker),
                ..Default::default()
            },
        )
        .await?;
    println!(
        "Canceled {} (reduced by {})",
        canceled.order_id, canceled.reduced_by
    );
    Ok(())
}
