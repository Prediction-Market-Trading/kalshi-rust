use super::run_order;
use futures_util::{SinkExt, StreamExt};
use kalshi_sdk_rs::{Client, Credentials, Environment};
use ring::{rand::SystemRandom, signature::Ed25519KeyPair};
use serde_json::{json, Value};
use std::{
    io::{self, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message as Frame};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[derive(Clone, Copy)]
enum FeedMode {
    Healthy,
    Malformed,
    LostCreateResponse,
    RecoveryReadFailure,
    Rejected,
}

#[derive(Default)]
struct State {
    client_order_id: String,
    created_at: Option<std::time::Instant>,
    canceled_at: Option<std::time::Instant>,
    subscribed_before_create: bool,
}

struct Fixture {
    client: Client,
    http: Arc<MockServer>,
    websocket: JoinHandle<()>,
    state: Arc<Mutex<State>>,
}

fn market(tick: &str) -> Value {
    json!({"market": {
        "ticker": "KXETHPERP", "title": "Ethereum perpetual", "status": "active",
        "contract_size": "0.001000", "underlying_multiplier": "1", "tick_size": tick,
        "fractional_trading_enabled": true, "minimum_quantity": "1.00", "schedule": null,
        "market_version": 7, "funding_deadband": 0, "funding_cap": 0.02,
        "funding_limit_unit": "percentage", "funding_invalid_premium_handling": "exclude",
        "halt_sources": [], "bid": "1.8000", "ask": "1.8200"
    }})
}

fn order(client_order_id: &str, remaining: &str) -> Value {
    json!({"order_id":"order-1", "user_id":"user-1", "client_order_id":client_order_id,
        "ticker":"KXETHPERP", "side":"bid", "last_update_reason":"",
        "price":"0.0100", "fill_count":"0.00", "remaining_count":remaining,
        "order_source":"user"})
}

fn update(client_order_id: &str, order_id: &str, remaining: &str) -> Value {
    json!({"type":"user_order", "sid":7, "msg": {
        "order_id":order_id, "user_id":"user-1", "client_order_id":client_order_id,
        "ticker":"KXETHPERP", "side":"bid", "price":"0.0100",
        "fill_count":"0.00", "remaining_count":remaining,
        "created_ts_ms":1794866400000i64, "last_updated_ts_ms":1794866400001i64,
        "subaccount_number":2, "subaccount_type":"cross", "order_source":"user"
    }})
}

impl Fixture {
    async fn new(available: &str, tick: &str, mode: FeedMode) -> Self {
        let http = Arc::new(MockServer::start().await);
        Mock::given(method("GET"))
            .and(path("/trade-api/v2/margin/enabled"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"enabled":true})))
            .mount(&http)
            .await;
        Mock::given(method("GET"))
            .and(path("/trade-api/v2/margin/balance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "settled_funds":"10.0000", "subaccount_balances":[{
                    "subaccount":2, "subaccount_type":"cross", "position_value":"0.0000",
                    "account_equity":"10.0000", "maintenance_margin":"0.0000",
                    "initial_margin":"0.0000", "resting_orders_margin":"0.0000",
                    "available_balance":available
                }]
            })))
            .mount(&http)
            .await;
        Mock::given(method("GET"))
            .and(path("/trade-api/v2/margin/markets/KXETHPERP"))
            .respond_with(ResponseTemplate::new(200).set_body_json(market(tick)))
            .mount(&http)
            .await;

        let state = Arc::new(Mutex::new(State::default()));
        let ack = Arc::new(AtomicBool::new(false));
        let (created_tx, mut created_rx) = mpsc::unbounded_channel::<Value>();
        let (canceled_tx, mut canceled_rx) = mpsc::unbounded_channel::<()>();
        let (recovery_tx, mut recovery_rx) = mpsc::unbounded_channel::<()>();
        let post_state = state.clone();
        let post_ack = ack.clone();
        Mock::given(method("POST"))
            .and(path("/trade-api/v2/margin/orders"))
            .respond_with(move |request: &wiremock::Request| {
                let request: Value = serde_json::from_slice(&request.body).unwrap();
                let client_id = request["client_order_id"].as_str().unwrap().to_owned();
                let mut state = post_state.lock().unwrap();
                state.client_order_id = client_id.clone();
                if !matches!(mode, FeedMode::Rejected) {
                    state.created_at = Some(std::time::Instant::now());
                }
                state.subscribed_before_create = post_ack.load(Ordering::SeqCst);
                created_tx.send(request).unwrap();
                if matches!(mode, FeedMode::Rejected) {
                    ResponseTemplate::new(400).set_body_json(json!({"error": {
                        "code":"invalid_price", "message":"price outside the permitted band"
                    }}))
                } else if matches!(
                    mode,
                    FeedMode::LostCreateResponse | FeedMode::RecoveryReadFailure
                ) {
                    // The engine accepted the order, but its success body cannot be
                    // decoded. The client must reconcile this write rather than replay it.
                    ResponseTemplate::new(201).set_body_string("{")
                } else {
                    ResponseTemplate::new(201)
                        .set_delay(Duration::from_millis(500))
                        .set_body_json(json!({
                            "order_id":"order-1", "client_order_id":client_id,
                            "fill_count":"0.00", "remaining_count":"1.00"
                        }))
                }
            })
            .mount(&http)
            .await;
        let delete_state = state.clone();
        Mock::given(method("DELETE"))
            .and(path("/trade-api/v2/margin/orders/order-1"))
            .respond_with(move |_request: &wiremock::Request| {
                delete_state.lock().unwrap().canceled_at = Some(std::time::Instant::now());
                let _ = canceled_tx.send(());
                ResponseTemplate::new(200)
                    .set_body_json(json!({"order_id":"order-1", "reduced_by":"1.00"}))
            })
            .mount(&http)
            .await;
        let get_state = state.clone();
        Mock::given(method("GET"))
            .and(path("/trade-api/v2/margin/orders/order-1"))
            .respond_with(move |_request: &wiremock::Request| {
                let state = get_state.lock().unwrap();
                ResponseTemplate::new(200).set_body_json(json!({"order":order(
                    &state.client_order_id, if state.canceled_at.is_some() { "0.00" } else { "1.00" }
                )}))
            })
            .mount(&http).await;
        let list_state = state.clone();
        Mock::given(method("GET"))
            .and(path("/trade-api/v2/margin/orders"))
            .respond_with(move |_request: &wiremock::Request| {
                if matches!(mode, FeedMode::RecoveryReadFailure) {
                    let _ = recovery_tx.send(());
                    return ResponseTemplate::new(503)
                        .set_delay(Duration::from_millis(100))
                        .set_body_json(json!({"error":{"code":"unavailable"}}));
                }
                let state = list_state.lock().unwrap();
                let orders = if state.created_at.is_none() {
                    vec![]
                } else {
                    vec![order(
                        &state.client_order_id,
                        if state.canceled_at.is_some() {
                            "0.00"
                        } else {
                            "1.00"
                        },
                    )]
                };
                ResponseTemplate::new(200).set_body_json(json!({"orders":orders}))
            })
            .mount(&http)
            .await;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let ws_http = http.clone();
        let websocket = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let command = socket.next().await.unwrap().unwrap().into_text().unwrap();
            let command: Value = serde_json::from_str(&command).unwrap();
            assert_eq!(command["cmd"], "subscribe");
            assert_eq!(command["params"]["channels"], json!(["user_orders"]));
            // Delay the acknowledgment to prove the example does not send a write
            // after merely transmitting its subscribe command.
            tokio::time::sleep(Duration::from_millis(30)).await;
            let requests = ws_http.received_requests().await.unwrap();
            assert!(requests
                .iter()
                .all(|request| request.method.as_str() != "POST"));
            ack.store(true, Ordering::SeqCst);
            socket.send(Frame::Text(json!({
                "type":"subscribed", "id":command["id"], "msg":{"channel":"user_orders", "sid":7}
            }).to_string().into())).await.unwrap();
            let created = created_rx.recv().await.unwrap();
            let client_id = created["client_order_id"].as_str().unwrap();
            if matches!(mode, FeedMode::Rejected) {
                while let Some(Ok(frame)) = socket.next().await {
                    if matches!(frame, Frame::Close(_)) {
                        let _ = socket.flush().await;
                        break;
                    }
                }
                return;
            }
            if matches!(mode, FeedMode::RecoveryReadFailure) {
                // Reveal the order ID only while its reconciliation read is in
                // flight. That read then fails; the known order still needs DELETE.
                recovery_rx.recv().await.unwrap();
            }
            socket
                .send(Frame::Text(
                    update("unrelated-client", "other-order", "1.00")
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            socket
                .send(Frame::Text(
                    update(client_id, "order-1", "1.00").to_string().into(),
                ))
                .await
                .unwrap();
            if matches!(mode, FeedMode::Healthy) {
                socket
                    .send(Frame::Ping(b"pending-create".to_vec().into()))
                    .await
                    .unwrap();
                let pong = tokio::time::timeout(Duration::from_millis(300), socket.next())
                    .await
                    .expect("the order feed stopped being serviced during create")
                    .unwrap()
                    .unwrap();
                assert_eq!(pong, Frame::Pong(b"pending-create".to_vec().into()));
            }
            if matches!(mode, FeedMode::Malformed) {
                socket
                    .send(Frame::Text(
                        r#"{"type":"user_order","sid":7,"msg":{}}"#.into(),
                    ))
                    .await
                    .unwrap();
            }
            canceled_rx.recv().await.unwrap();
            let _ = socket
                .send(Frame::Text(
                    update(client_id, "order-1", "0.00").to_string().into(),
                ))
                .await;
            while let Some(Ok(frame)) = socket.next().await {
                if matches!(frame, Frame::Close(_)) {
                    let _ = socket.flush().await;
                    break;
                }
            }
        });
        let der = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let pem = pem::encode(&pem::Pem::new("PRIVATE KEY", der.as_ref()));
        let client = Client::builder(Environment::Demo)
            .credentials(Credentials::from_pem("lifecycle-test", &pem).unwrap())
            .base_url(format!("{}/trade-api/v2", http.uri()))
            .margin_websocket_url(format!("ws://{address}/trade-api/ws/v2/margin"))
            .build()
            .unwrap();
        Self {
            client,
            http,
            websocket,
            state,
        }
    }
}

#[tokio::test]
async fn order_updates_are_logged_and_order_is_canceled_after_the_hold() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new("10.0000", "0.0001", FeedMode::Healthy).await;
        let mut output = Vec::new();
        let hold = Duration::from_millis(100);
        run_order(&fixture.client, 2, hold, &mut output)
            .await
            .unwrap();
        fixture.websocket.await.unwrap();
        let requests = fixture.http.received_requests().await.unwrap();
        let balance = requests
            .iter()
            .find(|r| r.url.path().ends_with("/margin/balance"))
            .unwrap();
        assert!(balance
            .url
            .query_pairs()
            .any(|(k, v)| k == "compute_available_balance" && v == "true"));
        let create = requests
            .iter()
            .find(|r| r.method.as_str() == "POST")
            .unwrap();
        let body: Value = serde_json::from_slice(&create.body).unwrap();
        assert_eq!(body["ticker"], "KXETHPERP");
        assert_eq!(body["side"], "bid");
        assert_eq!(body["price"], "0.0100");
        assert_eq!(body["count"], "1.00");
        assert_eq!(body["post_only"], true);
        assert_eq!(body["time_in_force"], "good_till_canceled");
        assert_eq!(body["subaccount"], 2);
        assert_eq!(body["market_version"], 7);
        assert!(uuid::Uuid::parse_str(body["client_order_id"].as_str().unwrap()).is_ok());
        let expiry = body["expiration_time"].as_u64().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!((now + 20..=now + 90).contains(&expiry));
        let cancel = requests
            .iter()
            .find(|r| r.method.as_str() == "DELETE")
            .unwrap();
        assert_eq!(cancel.url.path(), "/trade-api/v2/margin/orders/order-1");
        assert!(cancel
            .url
            .query_pairs()
            .any(|(k, v)| k == "subaccount" && v == "2"));
        let state = fixture.state.lock().unwrap();
        assert!(state.subscribed_before_create);
        assert!(
            state
                .canceled_at
                .unwrap()
                .duration_since(state.created_at.unwrap())
                >= hold
        );
        let lines: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert!(lines
            .iter()
            .all(|line| line["type"] == "user_order" && line["msg"]["order_id"] == "order-1"));
        assert_eq!(lines[0]["msg"]["remaining_count"], "1.00");
        assert_eq!(lines[1]["msg"]["remaining_count"], "0.00");
    })
    .await
    .expect("local order lifecycle did not finish");
}

#[tokio::test]
async fn no_available_funds_prevents_order_submission() {
    let fixture = Fixture::new("0.0000", "0.0001", FeedMode::Healthy).await;
    let mut output = Vec::new();
    let error = run_order(&fixture.client, 2, Duration::from_millis(10), &mut output)
        .await
        .unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("available"),
        "{error}"
    );
    let requests = fixture.http.received_requests().await.unwrap();
    assert!(requests.iter().all(|r| r.method.as_str() == "GET"));
    assert!(output.is_empty());
    fixture.websocket.abort();
}

#[tokio::test]
async fn an_invalid_one_cent_tick_prevents_order_submission() {
    let fixture = Fixture::new("10.0000", "0.0300", FeedMode::Healthy).await;
    let mut output = Vec::new();
    let error = run_order(&fixture.client, 2, Duration::from_millis(10), &mut output)
        .await
        .unwrap_err();
    assert!(error.to_string().to_lowercase().contains("tick"), "{error}");
    let requests = fixture.http.received_requests().await.unwrap();
    assert!(requests.iter().all(|r| r.method.as_str() == "GET"));
    fixture.websocket.abort();
}

#[tokio::test]
async fn a_stream_decode_failure_still_cancels_the_order() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new("10.0000", "0.0001", FeedMode::Malformed).await;
        let mut output = Vec::new();
        let result = run_order(&fixture.client, 2, Duration::from_secs(1), &mut output).await;
        assert!(result.is_err());
        fixture.websocket.await.unwrap();
        assert!(fixture.state.lock().unwrap().canceled_at.is_some());
    })
    .await
    .expect("failed stream prevented cancellation");
}

struct FailedLog;
impl Write for FailedLog {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("test log failure"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_log_failure_still_cancels_the_order() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new("10.0000", "0.0001", FeedMode::Healthy).await;
        let result = run_order(&fixture.client, 2, Duration::from_secs(1), &mut FailedLog).await;
        assert!(result.is_err());
        fixture.websocket.await.unwrap();
        assert!(fixture.state.lock().unwrap().canceled_at.is_some());
    })
    .await
    .expect("failed logger prevented cancellation");
}

#[tokio::test]
async fn an_uncertain_create_is_reconciled_and_canceled_without_replaying_it() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new("10.0000", "0.0001", FeedMode::LostCreateResponse).await;
        let mut output = Vec::new();
        let result = run_order(&fixture.client, 2, Duration::from_millis(100), &mut output).await;
        assert!(result.is_err());
        fixture.websocket.await.unwrap();
        let requests = fixture.http.received_requests().await.unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.method.as_str() == "POST")
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.method.as_str() == "DELETE")
                .count(),
            1
        );
        assert!(fixture.state.lock().unwrap().canceled_at.is_some());
    })
    .await
    .expect("uncertain create was not canceled");
}

#[tokio::test]
async fn a_price_band_rejection_does_not_change_or_replay_the_order() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new("10.0000", "0.0001", FeedMode::Rejected).await;
        let mut output = Vec::new();
        let error = run_order(&fixture.client, 2, Duration::from_millis(100), &mut output)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("invalid_price"), "{error}");
        fixture.websocket.await.unwrap();
        let requests = fixture.http.received_requests().await.unwrap();
        let creates: Vec<_> = requests
            .iter()
            .filter(|r| r.method.as_str() == "POST")
            .collect();
        assert_eq!(creates.len(), 1);
        let submitted: Value = serde_json::from_slice(&creates[0].body).unwrap();
        assert_eq!(submitted["price"], "0.0100");
        assert_eq!(submitted["count"], "1.00");
        assert!(requests.iter().all(|r| r.method.as_str() != "DELETE"));
        assert!(output.is_empty());
        assert!(fixture.state.lock().unwrap().created_at.is_none());
    })
    .await
    .expect("rejected order did not finish");
}

#[tokio::test]
async fn an_order_discovered_on_the_stream_is_canceled_even_if_reconciliation_read_fails() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new("10.0000", "0.0001", FeedMode::RecoveryReadFailure).await;
        let mut output = Vec::new();
        let result = run_order(&fixture.client, 2, Duration::from_millis(100), &mut output).await;
        assert!(result.is_err());
        fixture.websocket.await.unwrap();
        let requests = fixture.http.received_requests().await.unwrap();
        assert!(requests.iter().any(|r| {
            r.method.as_str() == "GET" && r.url.path() == "/trade-api/v2/margin/orders"
        }));
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.method.as_str() == "POST")
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.method.as_str() == "DELETE")
                .count(),
            1
        );
        assert!(fixture.state.lock().unwrap().canceled_at.is_some());
    })
    .await
    .expect("failed reconciliation read prevented cancellation of a known order");
}
