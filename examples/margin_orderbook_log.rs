//! Capture a short Perps orderbook feed as JSON Lines. Never submits orders.
use kalshi_sdk_rs::{
    margin::{
        models::{GetMarginMarketsQuery, MarginMarketStatus},
        websocket::{Channel, Message, SubscribeParams, WebSocket},
    },
    Client, Credentials, Environment,
};
use std::{
    error::Error,
    fs::{self, OpenOptions},
    io::{self, BufWriter, Write},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Options {
    ticker: Option<String>,
    seconds: u64,
    output: PathBuf,
}

impl Options {
    fn from_args() -> Result<Option<Self>> {
        let mut ticker = std::env::var("KALSHI_MARGIN_MARKET_TICKER").ok();
        let mut seconds = 5;
        let mut output = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--help" || arg == "-h" {
                println!(
                    "Usage: cargo run --example margin_orderbook_log -- [--ticker TICKER] [--seconds N] [--output PATH]\n\
                     Defaults: most active open perp market, 5 seconds, target/margin-orderbook-<unix-ms>.jsonl\n\
                     KALSHI_ENV=demo (default) or production; existing KALSHI_API_KEY_ID and private-key environment variables are required.\n\
                     KALSHI_MARGIN_MARKET_TICKER supplies the ticker unless --ticker overrides it. Existing output files are never overwritten."
                );
                return Ok(None);
            }
            if !matches!(arg.as_str(), "--ticker" | "--seconds" | "--output") {
                return Err(format!("unknown argument {arg}; use --help").into());
            }
            let value = args.next().ok_or_else(|| format!("{arg} needs a value"))?;
            match arg.as_str() {
                "--ticker" => ticker = Some(value),
                "--seconds" => seconds = value.parse()?,
                "--output" => output = Some(PathBuf::from(value)),
                _ => unreachable!(),
            }
        }
        if seconds == 0
            || ticker
                .as_ref()
                .is_some_and(|ticker| ticker.trim().is_empty())
        {
            return Err("--seconds must be positive and the ticker must not be empty".into());
        }
        let output = output.unwrap_or_else(|| {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            PathBuf::from(format!("target/margin-orderbook-{stamp}.jsonl"))
        });
        Ok(Some(Self {
            ticker,
            seconds,
            output,
        }))
    }
}

#[derive(Default)]
struct Counts {
    snapshots: u64,
    deltas: u64,
}

async fn choose_ticker(client: &Client) -> Result<String> {
    let markets = client
        .margin()
        .get_markets(&GetMarginMarketsQuery {
            status: Some(MarginMarketStatus::Active),
        })
        .await?;
    markets.markets.into_iter()
        .filter(|market| market.status == MarginMarketStatus::Active && market.schedule.as_ref().is_none_or(|schedule| schedule.is_open))
        .max_by_key(|market| market.volume_24h.unwrap_or_default())
        .map(|market| market.ticker)
        .ok_or_else(|| "no active, currently open perp market found; select another environment or specify --ticker".into())
}

async fn capture(
    socket: &mut WebSocket,
    log: &mut impl Write,
    duration: Duration,
) -> Result<Counts> {
    let deadline = tokio::time::Instant::now()
        .checked_add(duration)
        .ok_or("--seconds is too large")?;
    let mut counts = Counts::default();
    loop {
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => return Ok(counts),
            signal = tokio::signal::ctrl_c() => { signal?; return Ok(counts); }
            result = socket.next_message() => match result? {
                Some(Message::OrderbookSnapshot(snapshot)) => {
                    serde_json::to_writer(&mut *log, &snapshot)?;
                    writeln!(log)?;
                    log.flush()?;
                    counts.snapshots += 1;
                }
                Some(Message::OrderbookDelta(delta)) => {
                    serde_json::to_writer(&mut *log, &delta)?;
                    writeln!(log)?;
                    log.flush()?;
                    counts.deltas += 1;
                }
                Some(Message::Error(error)) => return Err(format!("subscription error: {}", serde_json::to_string(&error)?).into()),
                Some(_) => {}
                None => return Err("WebSocket closed before the capture duration elapsed".into()),
            }
        }
    }
}

async fn close(socket: &mut WebSocket) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        socket.close().await?;
        while socket.next_message().await?.is_some() {}
        Ok::<_, kalshi_sdk_rs::Error>(())
    })
    .await??;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let Some(options) = Options::from_args()? else {
        return Ok(());
    };
    let environment = match std::env::var("KALSHI_ENV").as_deref() {
        Ok("demo") | Err(std::env::VarError::NotPresent) => Environment::Demo,
        Ok("production") => Environment::Production,
        _ => return Err("KALSHI_ENV must be demo or production".into()),
    };
    let client = Client::builder(environment)
        .credentials(Credentials::from_env()?)
        .build()?;
    let ticker = match options.ticker {
        Some(ticker) => ticker,
        None => choose_ticker(&client).await?,
    };
    if let Some(parent) = options
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&options.output)?;
    let mut log = BufWriter::new(file);
    println!(
        "Environment: {environment:?}; ticker: {ticker}; duration: {}s; output: {}",
        options.seconds,
        options.output.display()
    );
    let mut socket = client.margin().connect_websocket().await?;
    socket
        .subscribe(
            1,
            SubscribeParams {
                channels: vec![Channel::OrderbookDelta],
                market_ticker: Some(ticker),
                ..Default::default()
            },
        )
        .await?;
    let result = capture(&mut socket, &mut log, Duration::from_secs(options.seconds)).await;
    let flushed = log.flush();
    let closed = close(&mut socket).await;
    flushed?;
    let counts = result?;
    println!(
        "Saved {} snapshots and {} deltas to {}",
        counts.snapshots,
        counts.deltas,
        options.output.display()
    );
    closed?;
    if counts.snapshots == 0 {
        return Err(io::Error::other(
            "no orderbook snapshot received; try a longer capture or another ticker",
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use ring::{rand::SystemRandom, signature::Ed25519KeyPair};
    use serde_json::{json, Value};
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, tungstenite::Message as Frame};

    #[tokio::test]
    async fn capture_stops_on_time_and_writes_snapshot_and_delta_json_lines() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let snapshot = json!({"type":"orderbook_snapshot","sid":7,"seq":1,"msg":{
                "market_ticker":"BTC-PERP","bid":[["12345.0000","2.50"]],"ask":[]}});
            let delta = json!({"type":"orderbook_delta","sid":7,"seq":2,"msg":{
                "market_ticker":"BTC-PERP","price":"12345.0000","delta":"-0.50","side":"bid","ts_ms":1794866400000i64}});
            let expected = vec![snapshot.clone(), delta.clone()];
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = accept_async(stream).await.unwrap();
                let command = socket.next().await.unwrap().unwrap().into_text().unwrap();
                let command: Value = serde_json::from_str(&command).unwrap();
                assert_eq!(command["params"]["channels"], json!(["orderbook_delta"]));
                socket.send(Frame::Text(snapshot.to_string().into())).await.unwrap();
                socket.send(Frame::Text(delta.to_string().into())).await.unwrap();
                while let Some(frame) = socket.next().await {
                    if matches!(frame.unwrap(), Frame::Close(_)) { socket.flush().await.unwrap(); break; }
                }
            });
            let der = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
            let pem = pem::encode(&pem::Pem::new("PRIVATE KEY", der.as_ref()));
            let client = Client::builder(Environment::Demo)
                .credentials(Credentials::from_pem("example-test", &pem).unwrap())
                .margin_websocket_url(format!("ws://{address}/trade-api/ws/v2/margin"))
                .build().unwrap();
            let mut socket = client.margin().connect_websocket().await.unwrap();
            socket.subscribe(1, SubscribeParams { channels: vec![Channel::OrderbookDelta], market_ticker: Some("BTC-PERP".into()), ..Default::default() }).await.unwrap();
            let mut output = Vec::new();
            let start = tokio::time::Instant::now();
            let counts = capture(&mut socket, &mut output, Duration::from_millis(100)).await.unwrap();
            assert!(start.elapsed() >= Duration::from_millis(100));
            assert_eq!((counts.snapshots, counts.deltas), (1, 1));
            let actual: Vec<Value> = String::from_utf8(output).unwrap().lines()
                .map(|line| serde_json::from_str(line).unwrap()).collect();
            assert_eq!(actual, expected);
            close(&mut socket).await.unwrap();
            server.await.unwrap();
        }).await.expect("capture did not stop and close on time");
    }
}
