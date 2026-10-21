use kalshi_sdk_rs::{
    models::{GetMarketsQuery, GetMarketsQueryStatus},
    Client, Environment,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Environment::Production)?;
    let mut query = GetMarketsQuery {
        status: Some(GetMarketsQueryStatus::Open),
        limit: Some(20),
        ..Default::default()
    };
    // Explicit pagination keeps memory and request volume under the caller's control.
    for _ in 0..2 {
        let page = client.get_markets(&query).await?;
        for market in &page.markets {
            println!(
                "{}: {}",
                market.ticker,
                market.title.as_deref().unwrap_or("")
            );
        }
        match page.next_cursor() {
            Some(cursor) => query.cursor = Some(cursor.to_owned()),
            None => break,
        }
    }
    Ok(())
}
