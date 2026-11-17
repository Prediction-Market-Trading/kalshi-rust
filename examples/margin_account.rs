use kalshi_sdk_rs::{Client, Credentials, Environment};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let environment = match std::env::var("KALSHI_ENV").as_deref() {
        Ok("demo") | Err(std::env::VarError::NotPresent) => Environment::Demo,
        Ok("production") => Environment::Production,
        _ => return Err("KALSHI_ENV must be demo or production".into()),
    };
    let client = Client::builder(environment)
        .credentials(Credentials::from_env()?)
        .build()?;
    let margin = client.margin();
    let enabled = margin.get_enabled().await?;
    println!("Margin enabled: {}", enabled.enabled);
    if !enabled.enabled {
        return Err(
            "margin trading is not enabled for this account in the selected environment".into(),
        );
    }
    let markets = margin.get_markets(&Default::default()).await?;
    for market in markets.markets {
        println!("{}: {}", market.ticker, market.title);
    }
    // Leave the more costly available-balance computation disabled.
    let balance = margin.get_balance(&Default::default()).await?;
    println!("{}", serde_json::to_string_pretty(&balance)?);
    Ok(())
}
