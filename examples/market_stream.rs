use kalshi_sdk_rs::{
    websocket::{Channel, Message, SubscribeParams},
    Client, Credentials, Environment,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let environment = match std::env::var("KALSHI_ENV").as_deref() {
        Ok("demo") | Err(std::env::VarError::NotPresent) => Environment::Demo,
        Ok("production") => Environment::Production,
        _ => return Err("KALSHI_ENV must be demo or production".into()),
    };
    let ticker = std::env::var("KALSHI_MARKET_TICKER")?;
    let client = Client::builder(environment)
        .credentials(Credentials::from_env()?)
        .build()?;
    let mut socket = client.connect_websocket().await?;
    socket
        .subscribe(
            1,
            SubscribeParams {
                channels: vec![Channel::Ticker, Channel::OrderbookDelta, Channel::Trade],
                market_tickers: Some(vec![ticker]),
                send_initial_snapshot: Some(true),
                use_yes_price: Some(true),
                ..Default::default()
            },
        )
        .await?;
    loop {
        tokio::select! {
            result = socket.next_message() => match result? {
                Some(Message::Ticker(tick)) => println!("{}: {}", tick.msg.market_ticker, tick.msg.price_dollars),
                Some(Message::OrderbookSnapshot(snapshot)) => println!("{}", serde_json::to_string(&snapshot)?),
                Some(Message::OrderbookDelta(delta)) => println!("{}", serde_json::to_string(&delta)?),
                Some(message) => println!("{message:?}"),
                None => return Ok(()),
            },
            signal = tokio::signal::ctrl_c() => { signal?; break; }
        }
    }
    socket.close().await?;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while socket.next_message().await?.is_some() {}
        Ok::<_, kalshi_sdk_rs::Error>(())
    })
    .await?;
    Ok(())
}
