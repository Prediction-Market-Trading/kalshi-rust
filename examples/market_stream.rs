mod common;
use kalshi_sdk_rs::websocket::{Channel, Message, SubscribeParams};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ticker = std::env::var("KALSHI_MARKET_TICKER")?;
    let mut socket = common::authenticated_demo()?.connect_websocket().await?;
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
                Some(Message::OrderbookDelta(delta)) => println!("seq={} {} {}", delta.seq, delta.msg.price_dollars, delta.msg.delta_fp),
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
