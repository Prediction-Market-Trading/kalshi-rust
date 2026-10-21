mod common;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let balance = common::authenticated_demo()?
        .get_balance(&Default::default())
        .await?;
    println!(
        "Available: ${}; portfolio value: {} cents",
        balance.balance_dollars, balance.portfolio_value
    );
    Ok(())
}
