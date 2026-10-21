use kalshi_sdk_rs::{models::GetMarketOrderbookQuery, Client, Environment};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ticker = std::env::var("KALSHI_MARKET_TICKER")?;
    let client = Client::new(Environment::Production)?;
    let response = client
        .get_market_orderbook(&ticker, &GetMarketOrderbookQuery { depth: Some(10) })
        .await?;
    for (price, count) in response.orderbook_fp.yes_dollars {
        println!("YES bid {price}: {count} contracts");
    }
    for (price, count) in response.orderbook_fp.no_dollars {
        println!("NO bid {price}: {count} contracts");
    }
    Ok(())
}
