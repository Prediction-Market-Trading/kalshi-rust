use super::{load_state, transfer_one_dollar};
use kalshi_sdk_rs::{Client, Credentials, Environment};
use ring::{rand::SystemRandom, signature::Ed25519KeyPair};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

struct StateFile {
    directory: PathBuf,
    path: PathBuf,
}

impl StateFile {
    fn new() -> Self {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("margin-transfer-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("state.json");
        Self { directory, path }
    }
}

impl Drop for StateFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Accepted,
    Duplicate,
    MalformedSuccess,
    Forbidden,
    Existing,
}

fn transfer(client_id: &str, status: &str) -> Value {
    json!({
        "transfer_id": "transfer-1", "client_transfer_id": client_id,
        "source": "event_contract", "destination": "margined",
        "source_exchange_shard": 0, "destination_exchange_shard": 0,
        "amount": "1.0000", "status": status, "created_ts": 1794866400
    })
}

struct Fixture {
    client: Client,
    server: MockServer,
    source_available: Arc<Mutex<String>>,
    complete: Arc<AtomicBool>,
}

impl Fixture {
    async fn new(client_id: &str, available: &str, mode: Mode) -> Self {
        let server = MockServer::start().await;
        let source_available = Arc::new(Mutex::new(available.to_owned()));
        let balance = source_available.clone();
        Mock::given(method("GET"))
            .and(path("/trade-api/v2/portfolio/balance"))
            .respond_with(move |_request: &wiremock::Request| {
                let dollars = balance.lock().unwrap().clone();
                let cents = (Decimal::from_str(&dollars).unwrap() * Decimal::from(100))
                    .trunc()
                    .to_string()
                    .parse::<i64>()
                    .unwrap();
                ResponseTemplate::new(200).set_body_json(json!({
                    "balance": cents, "balance_dollars": dollars,
                    "portfolio_value": 0, "updated_ts": 1794866400
                }))
            })
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/trade-api/v2/margin/balance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "settled_funds":"0.0000", "subaccount_balances":[{
                    "subaccount":0,"subaccount_type":"cross","position_value":"0.0000",
                    "account_equity":"0.0000","maintenance_margin":"0.0000",
                    "initial_margin":"0.0000","resting_orders_margin":"0.0000",
                    "available_balance":"0.0000"
                }]
            })))
            .mount(&server)
            .await;

        let visible = Arc::new(AtomicBool::new(matches!(mode, Mode::Existing)));
        let accepted = visible.clone();
        Mock::given(method("POST"))
            .and(path("/trade-api/v2/portfolio/intra_exchange_instance_transfer"))
            .respond_with(move |_request: &wiremock::Request| {
                if matches!(mode, Mode::Forbidden) {
                    return ResponseTemplate::new(403).set_body_json(json!({
                        "code":"permission_denied", "message":"cross-instance transfers unavailable"
                    }));
                }
                accepted.store(true, Ordering::SeqCst);
                match mode {
                    Mode::Duplicate => ResponseTemplate::new(409).set_body_json(json!({
                        "code":"transfer_already_exists", "message":"client transfer ID already accepted"
                    })),
                    Mode::MalformedSuccess => ResponseTemplate::new(200).set_body_string("{"),
                    _ => ResponseTemplate::new(200)
                        .set_body_json(json!({"transfer_id":"transfer-1"})),
                }
            })
            .mount(&server)
            .await;

        let history_id = client_id.to_owned();
        Mock::given(method("GET"))
            .and(path(
                "/trade-api/v2/portfolio/intra_exchange_instance_transfers",
            ))
            .respond_with(move |_request: &wiremock::Request| {
                let transfers = if visible.load(Ordering::SeqCst) {
                    vec![transfer(&history_id, "pending")]
                } else {
                    Vec::new()
                };
                ResponseTemplate::new(200).set_body_json(json!({"transfers":transfers}))
            })
            .mount(&server)
            .await;

        let complete = Arc::new(AtomicBool::new(true));
        let status_complete = complete.clone();
        let reads = Arc::new(AtomicUsize::new(0));
        let transfer_id = client_id.to_owned();
        Mock::given(method("GET"))
            .and(path(
                "/trade-api/v2/portfolio/intra_exchange_instance_transfers/transfer-1",
            ))
            .respond_with(move |_request: &wiremock::Request| {
                let count = reads.fetch_add(1, Ordering::SeqCst);
                let status = if count > 0 && status_complete.load(Ordering::SeqCst) {
                    "complete"
                } else {
                    "pending"
                };
                ResponseTemplate::new(200)
                    .set_body_json(json!({"transfer":transfer(&transfer_id,status)}))
            })
            .mount(&server)
            .await;

        let der = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let pem = pem::encode(&pem::Pem::new("PRIVATE KEY", der.as_ref()));
        let client = Client::builder(Environment::Demo)
            .credentials(Credentials::from_pem("example-test", &pem).unwrap())
            .base_url(format!("{}/trade-api/v2", server.uri()))
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        Self {
            client,
            server,
            source_available,
            complete,
        }
    }

    async fn writes(&self) -> Vec<wiremock::Request> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|request| request.method.as_str() == "POST")
            .collect()
    }
}

#[test]
fn state_retains_its_client_id_and_rejects_an_environment_change() {
    let file = StateFile::new();
    let initial = load_state(&file.path, "demo").unwrap();
    uuid::Uuid::parse_str(&initial.client_transfer_id).unwrap();
    assert!(initial.transfer_id.is_none());
    let resumed = load_state(&file.path, "demo").unwrap();
    assert_eq!(initial.client_transfer_id, resumed.client_transfer_id);
    assert!(load_state(&file.path, "production").is_err());
    let stored: Value = serde_json::from_slice(&fs::read(&file.path).unwrap()).unwrap();
    assert_eq!(stored["client_transfer_id"], initial.client_transfer_id);
}

#[tokio::test]
async fn transfers_exactly_one_dollar_and_waits_for_completion() {
    let file = StateFile::new();
    let mut state = load_state(&file.path, "demo").unwrap();
    let fixture = Fixture::new(&state.client_transfer_id, "2.0000", Mode::Accepted).await;
    transfer_one_dollar(
        &fixture.client,
        &mut state,
        &file.path,
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    let writes = fixture.writes().await;
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0].url.path(),
        "/trade-api/v2/portfolio/intra_exchange_instance_transfer"
    );
    assert!(writes[0].headers.contains_key("kalshi-access-signature"));
    let body: Value = serde_json::from_slice(&writes[0].body).unwrap();
    assert_eq!(body["amount"], 10_000);
    assert_eq!(body["source"], "event_contract");
    assert_eq!(body["destination"], "margined");
    assert_eq!(body["client_transfer_id"], state.client_transfer_id);
    assert_eq!(body["source_exchange_shard"], 0);
    assert_eq!(body["destination_exchange_shard"], 0);
    assert!(body.get("source_subaccount").is_none());
    assert!(body.get("destination_subaccount").is_none());
    let stored = load_state(&file.path, "demo").unwrap();
    assert_eq!(stored.transfer_id.as_deref(), Some("transfer-1"));
    let requests = fixture.server.received_requests().await.unwrap();
    let balance = requests
        .iter()
        .find(|request| request.url.path() == "/trade-api/v2/portfolio/balance")
        .unwrap();
    let query: std::collections::BTreeMap<_, _> = balance.url.query_pairs().collect();
    assert_eq!(
        query.get("exchange_index").map(|value| value.as_ref()),
        Some("0")
    );
    assert_eq!(
        query.get("subaccount").map(|value| value.as_ref()),
        Some("0")
    );
}

#[tokio::test]
async fn insufficient_predictions_funds_prevent_a_transfer() {
    let file = StateFile::new();
    let mut state = load_state(&file.path, "demo").unwrap();
    let fixture = Fixture::new(&state.client_transfer_id, "0.9999", Mode::Accepted).await;
    let result = transfer_one_dollar(
        &fixture.client,
        &mut state,
        &file.path,
        Duration::from_secs(1),
    )
    .await;
    assert!(result.is_err());
    assert!(fixture.writes().await.is_empty());
    assert!(state.transfer_id.is_none());
}

#[tokio::test]
async fn pending_transfer_resumes_without_a_second_post_or_another_dollar_balance() {
    let file = StateFile::new();
    let mut state = load_state(&file.path, "demo").unwrap();
    let fixture = Fixture::new(&state.client_transfer_id, "1.0000", Mode::Accepted).await;
    fixture.complete.store(false, Ordering::SeqCst);
    let result = transfer_one_dollar(
        &fixture.client,
        &mut state,
        &file.path,
        Duration::from_millis(100),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(state.transfer_id.as_deref(), Some("transfer-1"));
    let mut resumed = load_state(&file.path, "demo").unwrap();
    *fixture.source_available.lock().unwrap() = "0.0000".into();
    fixture.complete.store(true, Ordering::SeqCst);
    transfer_one_dollar(
        &fixture.client,
        &mut resumed,
        &file.path,
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    assert_eq!(fixture.writes().await.len(), 1);
}

#[tokio::test]
async fn a_duplicate_client_id_is_reconciled_without_replaying_the_post() {
    let file = StateFile::new();
    let mut state = load_state(&file.path, "demo").unwrap();
    let fixture = Fixture::new(&state.client_transfer_id, "2.0000", Mode::Duplicate).await;
    transfer_one_dollar(
        &fixture.client,
        &mut state,
        &file.path,
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    assert_eq!(fixture.writes().await.len(), 1);
    assert_eq!(state.transfer_id.as_deref(), Some("transfer-1"));
}

#[tokio::test]
async fn an_accepted_transfer_with_a_malformed_response_is_reconciled() {
    let file = StateFile::new();
    let mut state = load_state(&file.path, "demo").unwrap();
    let fixture = Fixture::new(&state.client_transfer_id, "2.0000", Mode::MalformedSuccess).await;
    transfer_one_dollar(
        &fixture.client,
        &mut state,
        &file.path,
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    assert_eq!(fixture.writes().await.len(), 1);
    assert_eq!(
        load_state(&file.path, "demo")
            .unwrap()
            .transfer_id
            .as_deref(),
        Some("transfer-1")
    );
}

#[tokio::test]
async fn a_previously_accepted_client_id_is_not_submitted_again() {
    let file = StateFile::new();
    let mut state = load_state(&file.path, "demo").unwrap();
    let fixture = Fixture::new(&state.client_transfer_id, "0.0000", Mode::Existing).await;
    transfer_one_dollar(
        &fixture.client,
        &mut state,
        &file.path,
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    assert!(fixture.writes().await.is_empty());
    assert_eq!(state.transfer_id.as_deref(), Some("transfer-1"));
}

#[tokio::test]
async fn api_unavailability_is_reported_without_an_automatic_retry() {
    let file = StateFile::new();
    let mut state = load_state(&file.path, "demo").unwrap();
    let original_id = state.client_transfer_id.clone();
    let fixture = Fixture::new(&original_id, "2.0000", Mode::Forbidden).await;
    let error = transfer_one_dollar(
        &fixture.client,
        &mut state,
        &file.path,
        Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("permission_denied"), "{error}");
    assert_eq!(fixture.writes().await.len(), 1);
    assert_eq!(
        load_state(&file.path, "demo").unwrap().client_transfer_id,
        original_id
    );
    assert!(state.transfer_id.is_none());
}
