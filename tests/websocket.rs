mod common;
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{SinkExt, StreamExt};
use kalshi_sdk_rs::{
    websocket::{Channel, Message, SubscribeParams},
    Client, Environment,
};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        handshake::server::{Request, Response},
        Message as Frame,
    },
};

#[test]
fn official_server_examples_decode_into_known_typed_variants() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/ws-official.json")).unwrap();
    assert_eq!(cases.len(), 38);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let message = Message::from_json(&case["payload"].to_string())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!matches!(message, Message::Unknown(_)), "{name}");
        // The documented legacy direction/time fields may disappear at any time.
        let mut payload = case["payload"].clone();
        if let Some(msg) = payload.get_mut("msg").and_then(|m| m.as_object_mut()) {
            for field in [
                "action",
                "side",
                "is_yes",
                "purchased_side",
                "taker_side",
                "ts",
                "time",
                "created_time",
                "last_update_time",
                "expiration_time",
            ] {
                msg.remove(field);
            }
        }
        // Orderbook delta's `side` and string `ts` are still canonical in that schema.
        if payload["type"] == "orderbook_delta" {
            continue;
        }
        Message::from_json(&payload.to_string())
            .unwrap_or_else(|e| panic!("without deprecated fields {name}: {e}"));
    }
}

#[test]
fn unknown_messages_are_retained_and_malformed_known_messages_are_errors() {
    assert!(
        matches!(Message::from_json(r#"{"type":"new_channel","msg":{"amount":"1.00"}}"#).unwrap(),Message::Unknown(v) if v["msg"]["amount"]=="1.00")
    );
    assert!(Message::from_json(r#"{"type":"ticker","msg":{}}"#).is_err());
    assert!(Message::from_json("[]").is_err());
    assert!(Message::from_json(r#"{"type":123}"#).is_err());
    assert!(Message::from_json("not JSON").is_err());
}

#[test]
fn empty_books_negative_deltas_and_shared_envelope_types() {
    let message=Message::from_json(r#"{"type":"orderbook_snapshot","sid":1,"seq":1,"msg":{"market_ticker":"T","market_id":"I"}}"#).unwrap();
    assert!(matches!(message,Message::OrderbookSnapshot(v) if v.msg.yes_dollars_fp.is_none()));
    assert!(matches!(
        Message::from_json(r#"{"type":"ok","id":3,"msg":[]}"#).unwrap(),
        Message::Subscriptions(_)
    ));
    assert!(matches!(
        Message::from_json(r#"{"type":"ok","id":3}"#).unwrap(),
        Message::Ok(_)
    ));
    assert!(matches!(
        Message::from_json(r#"{"type":"ok","sid":1,"seq":2,"msg":{"index_ids":["BRTI"]}}"#)
            .unwrap(),
        Message::SubscribedIndicesResponse(_)
    ));
    assert!(matches!(Message::from_json(r#"{"type":"market_lifecycle_v2","sid":1,"seq":2,"msg":{"event_type":"metadata_updated","market_ticker":"T"}}"#).unwrap(),Message::MarketMetadataUpdated(_)));
}

#[tokio::test]
// Tungstenite requires this callback to return its unboxed HTTP rejection response.
#[allow(clippy::result_large_err)]
async fn local_socket_auth_commands_ping_pong_and_clean_server_disconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (auth, public) = common::credentials();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_hdr_async(stream, move |req: &Request, response: Response| {
            assert_eq!(req.uri().path(), "/trade-api/ws/v2");
            assert_eq!(req.headers()["kalshi-access-key"], "test-key-id");
            let message = format!(
                "{}GET/trade-api/ws/v2",
                req.headers()["kalshi-access-timestamp"].to_str().unwrap()
            );
            let signature = STANDARD
                .decode(req.headers()["kalshi-access-signature"].to_str().unwrap())
                .unwrap();
            UnparsedPublicKey::new(&ED25519, &public)
                .verify(message.as_bytes(), &signature)
                .unwrap();
            Ok(response)
        })
        .await
        .unwrap();
        for expected in [
            "subscribe",
            "list_subscriptions",
            "update_subscription",
            "unsubscribe",
        ] {
            let frame = ws.next().await.unwrap().unwrap();
            let value: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            assert_eq!(value["cmd"], expected);
            if expected == "subscribe" {
                assert_eq!(value["params"]["use_yes_price"], true);
            }
        }
        ws.send(Frame::Text(
            json!({"type":"subscribed","id":1,"msg":{"channel":"ticker","sid":9}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        ws.send(Frame::Ping(vec![1, 2, 3].into())).await.unwrap();
        assert_eq!(
            ws.next().await.unwrap().unwrap(),
            Frame::Pong(vec![1, 2, 3].into())
        );
        ws.close(None).await.unwrap();
        let _ = ws.next().await;
    });
    let client = Client::builder(Environment::Demo)
        .credentials(auth)
        .websocket_url(format!("ws://{address}/trade-api/ws/v2"))
        .build()
        .unwrap();
    let mut ws = client.connect_websocket().await.unwrap();
    ws.subscribe(
        1,
        SubscribeParams {
            channels: vec![Channel::Ticker],
            use_yes_price: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    ws.list_subscriptions(2).await.unwrap();
    ws.update_subscription(
        3,
        kalshi_sdk_rs::websocket::UpdateParams {
            sid: Some(9),
            market_tickers: Some(vec!["T".into()]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    ws.unsubscribe(4, vec![9]).await.unwrap();
    assert!(matches!(
        ws.next_message().await.unwrap(),
        Some(Message::Subscribed(_))
    ));
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), ws.next_message())
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    assert!(ws.next_message().await.unwrap().is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn clean_client_disconnect_and_auth_failure() {
    let unauthenticated = Client::new(Environment::Demo).unwrap();
    assert!(matches!(
        unauthenticated.connect_websocket().await,
        Err(kalshi_sdk_rs::Error::Auth(_))
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert!(matches!(ws.next().await.unwrap().unwrap(), Frame::Close(_)));
        ws.flush().await.unwrap();
    });
    let (auth, _) = common::credentials();
    let client = Client::builder(Environment::Demo)
        .credentials(auth)
        .websocket_url(format!("ws://{address}/trade-api/ws/v2"))
        .build()
        .unwrap();
    let mut ws = client.connect_websocket().await.unwrap();
    ws.close().await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), ws.next_message())
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    server.await.unwrap();
}
