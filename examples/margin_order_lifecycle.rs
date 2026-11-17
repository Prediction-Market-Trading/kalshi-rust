//! Log a Perps buy/cancel lifecycle. Requires explicit --submit; never adjusts the order price.
use kalshi_sdk_rs::{
    margin::{
        models::{
            BookSide, CancelMarginOrderQuery, CreateMarginOrderRequest,
            CreateMarginOrderRequestTimeInForce, GetMarginBalanceQuery, GetMarginOrdersQuery,
            MarginMarketStatus, SelfTradePreventionType, SubaccountType,
        },
        websocket::{Channel, Message, SubscribeParams, WebSocket},
    },
    Client, Credentials, Environment, Error as SdkError,
};
use rust_decimal::Decimal;
use std::{
    error::Error,
    fs::{self, OpenOptions},
    future::Future,
    io::{self, BufWriter, Write},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const TICKER: &str = "KXETHPERP";
const PRICE: &str = "0.0100";
const COUNT: &str = "1.00";

struct Options {
    seconds: u64,
    subaccount: i64,
    output: PathBuf,
}

impl Options {
    fn from_args() -> Result<Option<Self>> {
        let mut submit = false;
        let mut seconds = 5;
        let mut subaccount = 0;
        let mut output = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    println!(
                        "Usage: cargo run --example margin_order_lifecycle -- --submit [--seconds N] [--subaccount N] [--output PATH]\n\
                         Buys 1 KXETHPERP contract at a $0.0100 limit, post-only, then cancels.\n\
                         Defaults: demo, 5 seconds (1-30), primary subaccount 0, target/margin-order-updates-<unix-ms>.jsonl.\n\
                         KALSHI_ENV=demo or production selects the environment. Existing API key variables are required.\n\
                         Post-only orders can fill after resting; cancellation does not reverse fills."
                    );
                    return Ok(None);
                }
                "--submit" => submit = true,
                "--seconds" | "--subaccount" | "--output" => {
                    let value = args.next().ok_or_else(|| format!("{arg} needs a value"))?;
                    match arg.as_str() {
                        "--seconds" => seconds = value.parse()?,
                        "--subaccount" => subaccount = value.parse()?,
                        "--output" => output = Some(PathBuf::from(value)),
                        _ => unreachable!(),
                    }
                }
                _ => return Err(format!("unknown argument {arg}; use --help").into()),
            }
        }
        if !submit {
            return Err(
                "pass --submit to explicitly submit and cancel this order; use --help for details"
                    .into(),
            );
        }
        if !(1..=30).contains(&seconds) || !(0..=63).contains(&subaccount) {
            return Err("--seconds must be 1-30 and --subaccount must be 0-63".into());
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        Ok(Some(Self {
            seconds,
            subaccount,
            output: output.unwrap_or_else(|| {
                PathBuf::from(format!("target/margin-order-updates-{stamp}.jsonl"))
            }),
        }))
    }
}

struct OrderLog<'a, W> {
    writer: &'a mut W,
    client_order_id: String,
    order_id: Option<String>,
    terminal: bool,
    updates: usize,
    stream_open: bool,
    stop_requested: bool,
    logging: bool,
    error: Option<Box<dyn Error>>,
}

impl<W: Write> OrderLog<'_, W> {
    fn fail(&mut self, error: impl Into<Box<dyn Error>>) {
        if self.error.is_none() {
            self.error = Some(error.into());
        }
    }

    fn receive(&mut self, result: kalshi_sdk_rs::Result<Option<Message>>) {
        match result {
            Ok(Some(Message::UserOrder(update))) => {
                if update.msg.client_order_id != self.client_order_id
                    && self.order_id.as_deref() != Some(update.msg.order_id.as_str())
                {
                    return;
                }
                self.order_id = Some(update.msg.order_id.clone());
                self.terminal |= update.msg.remaining_count.decimal().is_zero();
                if self.logging {
                    let written = (|| -> Result<()> {
                        serde_json::to_writer(&mut *self.writer, &update)?;
                        writeln!(self.writer)?;
                        self.writer.flush()?;
                        Ok(())
                    })();
                    match written {
                        Ok(()) => self.updates += 1,
                        Err(error) => {
                            self.logging = false;
                            self.fail(error);
                        }
                    }
                }
            }
            Ok(Some(Message::Error(error))) => self.fail(format!(
                "order subscription error: {}",
                serde_json::to_string(&error).unwrap_or_default()
            )),
            Ok(Some(_)) => {}
            Ok(None) => {
                self.stream_open = false;
                self.fail("order stream closed unexpectedly");
            }
            Err(error) => {
                self.stream_open = false;
                self.fail(error);
            }
        }
    }
}

// Do not abandon a pending create when the stream/log fails: it may already have
// reached the exchange. Obtain the response ID so cleanup can cancel that order.
async fn drive_request<T, W: Write>(
    socket: &mut WebSocket,
    log: &mut OrderLog<'_, W>,
    request: impl Future<Output = kalshi_sdk_rs::Result<T>>,
) -> kalshi_sdk_rs::Result<T> {
    tokio::pin!(request);
    loop {
        tokio::select! {
            result = &mut request => return result,
            signal = tokio::signal::ctrl_c(), if !log.stop_requested => {
                log.stop_requested = true;
                match signal {
                    Ok(()) => log.fail("interrupted; finishing the pending request so its order can be canceled"),
                    Err(error) => log.fail(error),
                }
            }
            update = socket.next_message(), if log.stream_open => log.receive(update),
        }
    }
}

async fn await_subscription<W: Write>(
    socket: &mut WebSocket,
    log: &mut OrderLog<'_, W>,
) -> Result<()> {
    socket
        .subscribe(
            1,
            SubscribeParams {
                channels: vec![Channel::UserOrders],
                market_tickers: Some(vec![TICKER.into()]),
                ..Default::default()
            },
        )
        .await?;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let update = tokio::select! {
                signal = tokio::signal::ctrl_c() => { signal?; return Err("interrupted before order submission".into()); }
                update = socket.next_message() => update?,
            };
            match update {
                Some(Message::Subscribed(ack))
                    if ack.id.is_none_or(|id| id == 1) && ack.msg.channel == "user_orders" =>
                {
                    return Ok(())
                }
                update => {
                    log.receive(Ok(update));
                    if let Some(error) = log.error.take() {
                        return Err(error);
                    }
                }
            }
        }
    })
    .await?
}

async fn recover_order_id<W: Write>(
    client: &Client,
    subaccount: i64,
    socket: &mut WebSocket,
    log: &mut OrderLog<'_, W>,
) -> Result<()> {
    let mut cursor = None;
    for _ in 0..10 {
        let page = drive_request(
            socket,
            log,
            client.margin().get_orders(&GetMarginOrdersQuery {
                ticker: Some(TICKER.into()),
                subaccount: Some(subaccount),
                limit: Some(1000),
                cursor,
                ..Default::default()
            }),
        )
        .await;
        // The stream can provide the ID even if this REST read fails.
        if log.order_id.is_some() {
            return Ok(());
        }
        let page = page?;
        if let Some(order) = page
            .orders
            .iter()
            .find(|order| order.client_order_id == log.client_order_id)
        {
            log.order_id = Some(order.order_id.clone());
            return Ok(());
        }
        cursor = page.next_cursor().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    Err(format!("create result is uncertain; could not find client_order_id {}. No create retry was sent; the order's fallback expiry is 60 seconds", log.client_order_id).into())
}

async fn lifecycle<W: Write>(
    client: &Client,
    subaccount: i64,
    hold: Duration,
    market_version: i64,
    socket: &mut WebSocket,
    log: &mut OrderLog<'_, W>,
) -> Result<()> {
    await_subscription(socket, log).await?;
    let expiry = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 60;
    let request = CreateMarginOrderRequest {
        ticker: Some(TICKER.into()),
        client_order_id: log.client_order_id.clone(),
        side: BookSide::Bid,
        count: COUNT.parse()?,
        price: PRICE.parse()?,
        time_in_force: CreateMarginOrderRequestTimeInForce::GoodTillCanceled,
        self_trade_prevention_type: SelfTradePreventionType::TakerAtCross,
        post_only: Some(true),
        expiration_time: Some(i64::try_from(expiry)?),
        subaccount: Some(subaccount),
        market_version: Some(market_version),
        ..Default::default()
    };
    println!(
        "Submitting {TICKER}: buy {COUNT} at ${PRICE}; client_order_id={}",
        log.client_order_id
    );
    let create = drive_request(socket, log, client.margin().create_order(&request)).await;
    let mut failure: Option<Box<dyn Error>> = None;
    match create {
        Ok(order) => {
            println!("Created order {}", order.order_id);
            log.order_id = Some(order.order_id);
        }
        Err(error) => {
            let uncertain = matches!(&error, SdkError::Http(_) | SdkError::Json(_))
                || matches!(&error, SdkError::Api { status, .. } if *status >= 500);
            failure = Some(error.into());
            if uncertain && log.order_id.is_none() {
                if let Err(error) = recover_order_id(client, subaccount, socket, log).await {
                    return Err(format!("{}; {error}", failure.as_ref().unwrap()).into());
                }
            }
        }
    }
    if failure.is_none() && log.error.is_none() && !log.terminal {
        let deadline = tokio::time::Instant::now() + hold;
        loop {
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(deadline) => break,
                signal = tokio::signal::ctrl_c() => { log.stop_requested = true; if let Err(error) = signal { log.fail(error); } break; }
                update = socket.next_message() => log.receive(update),
            }
            if log.error.is_some() || log.terminal {
                break;
            }
        }
    }
    if let Some(order_id) = log.order_id.clone() {
        // No fallible operation after creation may return before this cancellation attempt.
        match drive_request(
            socket,
            log,
            client.margin().cancel_order(
                &order_id,
                &CancelMarginOrderQuery {
                    subaccount: Some(subaccount),
                },
            ),
        )
        .await
        {
            Ok(canceled) => println!(
                "Canceled order {}: {} contracts removed",
                canceled.order_id, canceled.reduced_by
            ),
            Err(error) => {
                // An order can fill/expire while the cancellation is in flight.
                let checked =
                    drive_request(socket, log, client.margin().get_order(&order_id)).await;
                match checked {
                    Ok(order) if order.order.remaining_count.decimal().is_zero() => println!("Order {order_id} is already terminal; filled={}", order.order.fill_count),
                    _ => return Err(format!("cancel failed for order {order_id}: {error}. Fallback expiry was set to {expiry}; inspect the order before further action").into()),
                }
            }
        }
        if log.error.is_none() && !log.terminal {
            let received = tokio::time::timeout(Duration::from_secs(5), async {
                while !log.terminal && log.error.is_none() {
                    log.receive(socket.next_message().await);
                }
            })
            .await;
            if received.is_err() {
                log.fail(
                    "order cancellation or terminal state was confirmed, but no terminal order update arrived within five seconds",
                );
            }
        }
    }
    println!("Logged {} matching order updates", log.updates);
    if let Some(error) = failure.or_else(|| log.error.take()) {
        return Err(error);
    }
    Ok(())
}

async fn run_order(
    client: &Client,
    subaccount: i64,
    hold: Duration,
    writer: &mut impl Write,
) -> Result<()> {
    let balance = client
        .margin()
        .get_balance(&GetMarginBalanceQuery {
            compute_available_balance: Some(true),
        })
        .await?;
    if let Some(settled) = balance.settled_funds {
        println!("Margin settled funds: ${settled}");
    }
    for b in &balance.subaccount_balances {
        println!("Subaccount {} ({}): equity=${}, available=${}, initial_margin=${}, maintenance_margin=${}, resting_orders_margin=${}",
            b.subaccount, b.subaccount_type.as_str(), b.account_equity, b.available_balance, b.initial_margin, b.maintenance_margin, b.resting_orders_margin);
    }
    let account = balance
        .subaccount_balances
        .iter()
        .find(|b| b.subaccount == subaccount && b.subaccount_type == SubaccountType::Cross)
        .ok_or_else(|| format!("no cross-margin balance returned for subaccount {subaccount}"))?;
    if account.available_balance.decimal() <= Decimal::ZERO {
        return Err(format!(
            "subaccount {subaccount} has no available margin funds; no order submitted"
        )
        .into());
    }
    let market = client.margin().get_market(TICKER).await?.market;
    let price: kalshi_sdk_rs::FixedPointDollars = PRICE.parse()?;
    if market.status != MarginMarketStatus::Active
        || market
            .schedule
            .as_ref()
            .is_some_and(|schedule| !schedule.is_open)
    {
        return Err(format!("{TICKER} is not currently open; no order submitted").into());
    }
    if market.minimum_quantity.decimal() > Decimal::ONE
        || market.tick_size.decimal() <= Decimal::ZERO
        || price
            .decimal()
            .checked_rem(market.tick_size.decimal())
            .is_none_or(|remainder| !remainder.is_zero())
    {
        return Err("the fixed one-contract/$0.01 order does not meet this market's size or tick rules; no order submitted".into());
    }
    let mut socket = client.margin().connect_websocket().await?;
    let mut log = OrderLog {
        writer,
        client_order_id: uuid::Uuid::new_v4().to_string(),
        order_id: None,
        terminal: false,
        updates: 0,
        stream_open: true,
        stop_requested: false,
        logging: true,
        error: None,
    };
    let result = lifecycle(
        client,
        subaccount,
        hold,
        market.market_version,
        &mut socket,
        &mut log,
    )
    .await;
    let flushed = log.writer.flush();
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        socket.close().await?;
        while socket.next_message().await?.is_some() {}
        Ok::<_, SdkError>(())
    })
    .await;
    result?;
    flushed?;
    closed??;
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
        .timeout(Duration::from_secs(15))
        .build()?;
    if let Some(parent) = options
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&options.output)?;
    let mut writer = BufWriter::new(file);
    println!(
        "Environment: {environment:?}; market: {TICKER}; output: {}",
        options.output.display()
    );
    if let Err(error) = run_order(
        &client,
        options.subaccount,
        Duration::from_secs(options.seconds),
        &mut writer,
    )
    .await
    {
        if let Err(flush_error) = writer.flush() {
            return Err(io::Error::other(format!(
                "{error}; log flush also failed: {flush_error}; order update log: {}",
                options.output.display()
            ))
            .into());
        }
        return Err(io::Error::other(format!(
            "{error}; order update log: {}",
            options.output.display()
        ))
        .into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "common/margin_order_lifecycle_tests.rs"]
mod tests;
