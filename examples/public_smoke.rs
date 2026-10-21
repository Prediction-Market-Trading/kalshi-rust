//! Read-only live compatibility probe; never loads credentials or sends order writes.
use kalshi_sdk_rs::{models::GetMarketsQuery, Client, Environment};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Environment::Production)?;
    println!("Status: {:?}", client.get_exchange_status().await?);
    let page = client
        .get_markets(&GetMarketsQuery {
            limit: Some(2),
            ..Default::default()
        })
        .await?;
    println!("Markets: {}", page.markets.len());
    if let Some(market) = page.markets.first() {
        client.get_market(&market.ticker).await?;
        client
            .get_market_orderbook(&market.ticker, &Default::default())
            .await?;
        println!("Market and orderbook decoded");
    }
    let events = client.get_events(&Default::default()).await?;
    println!("Events: {}", events.events.len());
    client.get_trades(&Default::default()).await?;
    client.get_historical_cutoff().await?;
    println!("Trades and historical cutoff decoded");
    Ok(())
}
