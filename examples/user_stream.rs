mod common;
use kalshi_sdk_rs::websocket::{Channel, Message, SubscribeParams};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut socket = common::authenticated_demo()?.connect_websocket().await?;
    socket
        .subscribe(
            1,
            SubscribeParams {
                channels: vec![Channel::Fill, Channel::UserOrders],
                ..Default::default()
            },
        )
        .await?;
    loop {
        tokio::select! {
            result = socket.next_message() => match result? {
                Some(Message::Fill(fill)) => println!("fill {}: {} contracts", fill.msg.order_id, fill.msg.count_fp),
                Some(Message::UserOrder(order)) => println!("order {}: {:?}", order.msg.order_id, order.msg.status),
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
