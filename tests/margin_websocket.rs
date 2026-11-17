mod common;
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{SinkExt, StreamExt};
use kalshi_sdk_rs::{
    margin::websocket::{
        Channel, Event, Message, SubscribeParams, UpdateAction, UpdateParams, WebSocket,
    },
    Client, Environment, Error,
};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde_json::{json, Value};
use std::{collections::BTreeSet, time::Duration};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        handshake::server::{Request, Response},
        Message as Frame,
    },
    WebSocketStream,
};

fn delta() -> Value {
    json!({"type":"orderbook_delta","sid":9,"seq":2,"sending_ts_ms":9007199254740993i64,
        "msg":{"market_ticker":"BTC-PERP","side":"ask","price":"12345.1234","delta":"-2.50","ts_ms":1794866400001i64}})
}

#[test]
fn official_server_examples_decode_into_known_margin_variants() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/margin-ws-official.json")).unwrap();
    assert_eq!(cases.len(), 3);
    let mut types = BTreeSet::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let payload = case["payload"].to_string();
        for message in [
            Message::from_json(&payload),
            Message::from_slice(payload.as_bytes()),
        ] {
            let message = message.unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!matches!(message, Message::Unknown(_)), "{name}");
        }
        types.insert(case["payload"]["type"].as_str().unwrap());
    }
    assert_eq!(
        types,
        BTreeSet::from(["market_lifecycle_v2", "order_group_updates"])
    );
}

#[test]
fn margin_books_preserve_units_and_milliseconds_without_predictions_clocks() {
    let payload = json!({"type":"orderbook_snapshot","sid":9,"seq":1,
        "msg":{"market_ticker":"BTC-PERP","bid":[["12345.0000","2.50"]],"ask":[]}});
    let Message::OrderbookSnapshot(snapshot) = Message::from_json(&payload.to_string()).unwrap()
    else {
        panic!("snapshot")
    };
    let level = snapshot.msg.bid.as_ref().unwrap()[0];
    assert_eq!(level.0.to_string(), "12345.0000");
    assert_eq!(level.1.to_string(), "2.50");
    assert!(snapshot.msg.ask.as_ref().unwrap().is_empty());
    assert_eq!(serde_json::to_value(snapshot).unwrap(), payload);
    let Message::OrderbookDelta(change) = Message::from_json(&delta().to_string()).unwrap() else {
        panic!("delta")
    };
    assert_eq!(change.msg.price.to_string(), "12345.1234");
    assert_eq!(change.msg.delta.to_string(), "-2.50");
    assert_eq!(change.msg.side.as_str(), "ask");
    assert_eq!(change.msg.ts_ms, Some(1794866400001));
    assert_eq!(change.sending_ts_ms, Some(9007199254740993));
    assert_eq!(serde_json::to_value(change).unwrap(), delta());
    let Message::OrderbookSnapshot(empty) = Message::from_json(
        r#"{"type":"orderbook_snapshot","sid":9,"seq":1,"msg":{"market_ticker":"BTC-PERP"}}"#,
    )
    .unwrap() else {
        panic!("empty snapshot")
    };
    assert!(empty.msg.bid.is_none());
    assert!(empty.msg.ask.is_none());
}

#[test]
fn ticker_trade_fill_and_user_order_preserve_margin_payloads() {
    // These are handwritten contract fixtures: upstream provides examples only for
    // lifecycle and order-group messages in the pinned perps snapshot.
    let ticker = json!({"type":"ticker","sid":1,"sending_ts_ms":1794866400001i64,"msg":{
        "market_ticker":"BTC-PERP","price":"12345.1234","bid":"12345.0000","ask":"12346.0000",
        "bid_size_fp":"1.25","ask_size_fp":"2.50","last_trade_size_fp":"0.25","volume":"10.00",
        "volume_notional_value_dollars":"123451.234567","volume_24h":"9.00","volume_24h_notional_value_dollars":"111111.111111",
        "open_interest":"3.00","open_interest_notional_value_dollars":"37035.370370","ts_ms":1794866400000i64,
        "reference_price":{"price":"12345.0000","ts_ms":1794866399000i64},
        "settlement_mark_price":{"price":"12345.0001","ts_ms":1794866399001i64},
        "liquidation_mark_price":{"price":"12345.0002","ts_ms":1794866399002i64},
        "funding_rate":{"rate":serde_json::from_str::<Value>("0.0001234567890123456789").unwrap(),"next_funding_time_ms":1794866500000i64,"ts_ms":1794866400000i64}}});
    let Message::Ticker(message) = Message::from_json(&ticker.to_string()).unwrap() else {
        panic!("ticker")
    };
    assert_eq!(message.msg.volume.to_string(), "10.00");
    assert_eq!(message.msg.price.to_string(), "12345.1234");
    assert_eq!(
        message.msg.funding_rate.as_ref().unwrap().rate.to_string(),
        "0.0001234567890123456789"
    );
    assert_eq!(
        message.msg.reference_price.as_ref().unwrap().ts_ms,
        1794866399000
    );
    assert_eq!(serde_json::to_value(message).unwrap(), ticker);
    let trade = json!({"type":"trade","sid":2,"seq":1,"msg":{"trade_id":"trade-1","market_ticker":"BTC-PERP",
        "price":"12345.0000","count":"2.50","taker_side":"bid","is_block_trade":true,"ts_ms":1794866400000i64}});
    let Message::Trade(message) = Message::from_json(&trade.to_string()).unwrap() else {
        panic!("trade")
    };
    assert_eq!(message.msg.count.to_string(), "2.50");
    assert_eq!(message.msg.ts_ms, 1794866400000);
    assert!(message.msg.is_block_trade);
    assert_eq!(serde_json::to_value(message).unwrap(), trade);
    let fill = json!({"type":"fill","sid":3,"msg":{"trade_id":"trade-1","order_id":"order-1","market_ticker":"BTC-PERP",
        "is_taker":true,"side":"ask","ts_ms":1794866400000i64,"price":"12345.0000","count":"2.50",
        "fee_cost":"0.123456","post_position":"-2.50","order_source":"system","subaccount":1,"subaccount_type":"cross"}});
    let Message::Fill(message) = Message::from_json(&fill.to_string()).unwrap() else {
        panic!("fill")
    };
    assert_eq!(message.msg.post_position.to_string(), "-2.50");
    assert_eq!(message.msg.fee_cost.to_string(), "0.123456");
    assert_eq!(message.msg.order_source.as_str(), "system");
    assert_eq!(serde_json::to_value(message).unwrap(), fill);
    let user_order = json!({"type":"user_order","sid":4,"msg":{"order_id":"order-1","user_id":"user-1","client_order_id":"client-1",
        "ticker":"BTC-PERP","side":"ask","price":"12345.0000","fill_count":"2.50","remaining_count":"0.00",
        "created_ts_ms":null,"last_updated_ts_ms":1794866400000i64,"expiration_ts_ms":1794866500000i64,"order_source":"system"}});
    let Message::UserOrder(message) = Message::from_json(&user_order.to_string()).unwrap() else {
        panic!("user order")
    };
    assert_eq!(message.msg.fill_count.to_string(), "2.50");
    assert!(message.msg.created_ts_ms.is_none());
    assert_eq!(message.msg.expiration_ts_ms, Some(1794866500000));
    assert_eq!(message.msg.order_source.as_str(), "system");
}

#[test]
fn corporate_action_messages_decode_each_terms_object() {
    for (kind, field, terms) in [
        (
            "stock_split",
            "split",
            json!({"ratio_old":"1","ratio_new":"4","pre_split_mark_price":"400.0000","post_split_mark_price":"100.0000"}),
        ),
        (
            "final_settlement",
            "final_settlement",
            json!({"settlement_price_per_contract":"101.1234"}),
        ),
        (
            "ticker_change",
            "ticker_change",
            json!({"old_ticker":"OLD-PERP","new_ticker":"NEW-PERP"}),
        ),
    ] {
        let mut action = json!({"id":"action-1","market_id":"market-1","market_ticker":"NEW-PERP","type":kind,"ts_ms":1794866400000i64});
        action[field] = terms;
        let payload = json!({"type":"corporate_actions_delta","sid":1,"seq":1,"msg":{"corporate_action":action}});
        let Message::CorporateActionsDelta(message) =
            Message::from_json(&payload.to_string()).unwrap()
        else {
            panic!("corporate actions")
        };
        assert_eq!(message.msg.corporate_action.r#type.as_str(), kind);
        assert_eq!(message.msg.corporate_action.ts_ms, 1794866400000);
        assert_eq!(serde_json::to_value(message).unwrap(), payload);
    }
}

#[test]
fn unknown_types_enums_and_shared_acknowledgments_remain_distinct() {
    let unknown = json!({"type":"future_margin_message","msg":{"value":"1.234567"}});
    assert!(
        matches!(Message::from_json(&unknown.to_string()).unwrap(), Message::Unknown(v) if v == unknown)
    );
    let mut payload = delta();
    payload["msg"]["side"] = json!("future_side");
    let Message::OrderbookDelta(change) = Message::from_json(&payload.to_string()).unwrap() else {
        panic!("delta")
    };
    assert_eq!(change.msg.side.as_str(), "future_side");
    assert_eq!(serde_json::to_value(change).unwrap(), payload);
    assert!(matches!(
        Message::from_json(r#"{"type":"ok","id":2,"msg":[]}"#).unwrap(),
        Message::Subscriptions(_)
    ));
    assert!(matches!(
        Message::from_json(r#"{"type":"ok","id":2}"#).unwrap(),
        Message::Ok(_)
    ));
    for payload in [
        r#"{"type":"ticker","msg":{}}"#,
        r#"{"type":"corporate_actions_delta","sid":1,"seq":1,"msg":{}}"#,
        r#"{"type":"orderbook_delta","sid":9,"seq":2,"msg":{"market_ticker":"BTC-PERP","price":"1.00","delta":2,"side":"bid"}}"#,
        r#"{"type":"orderbook_delta","sid":9,"seq":2,"msg":{"market_ticker":"BTC-PERP","price":"1.00","delta":"2.001","side":"bid"}}"#,
        r#"{"type":123}"#,
        "[]",
        "invalid",
    ] {
        assert!(Message::from_json(payload).is_err(), "{payload}");
    }
}

#[tokio::test]
#[allow(clippy::result_large_err)] // Tungstenite's handshake callback uses an HTTP rejection response.
async fn margin_signed_handshake_commands_control_frames_and_decode_recovery() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (auth, public) = common::credentials();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_hdr_async(stream, move |req: &Request, response: Response| {
                assert_eq!(req.uri().path(), "/trade-api/ws/v2/margin");
                assert_eq!(req.headers()["kalshi-access-key"], "test-key-id");
                let message = format!("{}GET/trade-api/ws/v2/margin", req.headers()["kalshi-access-timestamp"].to_str().unwrap());
                let signature = STANDARD.decode(req.headers()["kalshi-access-signature"].to_str().unwrap()).unwrap();
                UnparsedPublicKey::new(&ED25519, &public).verify(message.as_bytes(), &signature).unwrap();
                Ok(response)
            }).await.unwrap();
            for (id, command) in [(1,"subscribe"), (2,"list_subscriptions"), (3,"update_subscription"), (4,"unsubscribe")] {
                let frame = ws.next().await.unwrap().unwrap();
                let value: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
                assert_eq!(value["id"], id);
                assert_eq!(value["cmd"], command);
                match command {
                    "subscribe" => {
                        assert_eq!(value["params"]["channels"], json!(["orderbook_delta","ticker","corporate_actions","market_lifecycle_v2"]));
                        assert_eq!(value["params"]["market_tickers"], json!(["BTC-PERP"]));
                        assert_eq!(value["params"]["update_interval_ms"], 100);
                        assert!(value["params"].get("use_yes_price").is_none());
                    },
                    "update_subscription" => {
                        assert_eq!(value["params"]["sid"], 9);
                        assert_eq!(value["params"]["action"], "get_snapshot");
                    },
                    "unsubscribe" => assert_eq!(value["params"]["sids"], json!([9])),
                    _ => {},
                }
            }
            ws.send(Frame::Text(json!({"type":"ok","id":2,"msg":[]}).to_string().into())).await.unwrap();
            ws.send(Frame::Text(json!({"type":"trade","sid":9,"msg":{}}).to_string().into())).await.unwrap();
            ws.send(Frame::Ping(vec![0,255,1].into())).await.unwrap();
            assert_eq!(ws.next().await.unwrap().unwrap(), Frame::Pong(vec![0,255,1].into()));
            ws.send(Frame::Binary(delta().to_string().into_bytes().into())).await.unwrap();
            ws.send(Frame::Pong(vec![7,8].into())).await.unwrap();
            ws.close(None).await.unwrap();
            let _ = ws.next().await;
        });
        let client = Client::builder(Environment::Demo).credentials(auth)
            .margin_websocket_url(format!("ws://{address}/trade-api/ws/v2/margin")).build().unwrap();
        let mut ws = client.margin().connect_websocket().await.unwrap();
        assert!(matches!(ws.subscribe(0, SubscribeParams::default()).await, Err(Error::Config(_))));
        ws.subscribe(1, SubscribeParams { channels: vec![Channel::OrderbookDelta, Channel::Ticker, Channel::CorporateActions, Channel::MarketLifecycleV2],
            market_tickers: Some(vec!["BTC-PERP".into()]), update_interval_ms: Some(100), ..Default::default() }).await.unwrap();
        ws.list_subscriptions(2).await.unwrap();
        ws.update_subscription(3, UpdateParams { sid: Some(9), action: UpdateAction::GetSnapshot, market_tickers: Some(vec!["BTC-PERP".into()]), ..Default::default() }).await.unwrap();
        ws.unsubscribe(4, vec![9]).await.unwrap();
        assert!(matches!(ws.next_message().await.unwrap(), Some(Message::Subscriptions(_))));
        assert!(matches!(ws.next_message().await, Err(Error::Json(_))));
        assert!(matches!(ws.next_event().await.unwrap(), Some(Event::Ping(v)) if v == [0,255,1]));
        assert!(matches!(ws.next_event().await.unwrap(), Some(Event::Message(Message::OrderbookDelta(v))) if v.seq == 2));
        assert!(matches!(ws.next_event().await.unwrap(), Some(Event::Pong(v)) if v == [7,8]));
        assert!(ws.next_message().await.unwrap().is_none());
        assert!(ws.next_event().await.unwrap().is_none());
        server.await.unwrap();
    }).await.expect("margin socket lifecycle timed out");
}

async fn local_socket_pair() -> (WebSocket, WebSocketStream<TcpStream>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (auth, _) = common::credentials();
    let client = Client::builder(Environment::Demo)
        .credentials(auth)
        .margin_websocket_url(format!("ws://{address}/trade-api/ws/v2/margin"))
        .build()
        .unwrap();
    let margin = client.margin();
    let (connected, accepted) = tokio::join!(margin.connect_websocket(), async {
        let (stream, _) = listener.accept().await.unwrap();
        tokio_tungstenite::accept_async(stream).await.unwrap()
    });
    (connected.unwrap(), accepted)
}

#[tokio::test]
async fn both_margin_receive_apis_preserve_messages_and_handle_control_frames() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for events in [false, true] {
            let (mut client, mut server) = local_socket_pair().await;
            let task = tokio::spawn(async move {
                server.send(Frame::Ping(b"heartbeat".to_vec().into())).await.unwrap();
                server.send(Frame::Pong(Vec::new().into())).await.unwrap();
                server.send(Frame::Text(delta().to_string().into())).await.unwrap();
                assert_eq!(server.next().await.unwrap().unwrap(), Frame::Pong(b"heartbeat".to_vec().into()));
                server.close(None).await.unwrap();
                let _ = server.next().await;
            });
            if events {
                assert!(matches!(client.next_event().await.unwrap(), Some(Event::Ping(v)) if v == b"heartbeat"));
                assert!(matches!(client.next_event().await.unwrap(), Some(Event::Pong(v)) if v.is_empty()));
                assert!(matches!(client.next_event().await.unwrap(), Some(Event::Message(Message::OrderbookDelta(v))) if v.msg.delta.to_string() == "-2.50"));
            } else {
                assert!(matches!(client.next_message().await.unwrap(), Some(Message::OrderbookDelta(v)) if v.msg.delta.to_string() == "-2.50"));
            }
            assert!(client.next_message().await.unwrap().is_none());
            task.await.unwrap();
        }
    }).await.expect("margin control frames timed out");
}

#[tokio::test]
async fn margin_client_ping_clean_close_and_missing_credentials() {
    assert!(matches!(
        Client::new(Environment::Demo)
            .unwrap()
            .margin()
            .connect_websocket()
            .await,
        Err(Error::Auth(_))
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        let (mut client, mut server) = local_socket_pair().await;
        let task = tokio::spawn(async move {
            assert_eq!(
                server.next().await.unwrap().unwrap(),
                Frame::Ping(vec![5, 0, 255].into())
            );
            server.flush().await.unwrap();
            assert!(matches!(
                server.next().await.unwrap().unwrap(),
                Frame::Close(_)
            ));
            server.flush().await.unwrap();
        });
        client.ping(vec![5, 0, 255]).await.unwrap();
        assert!(
            matches!(client.next_event().await.unwrap(), Some(Event::Pong(v)) if v == [5,0,255])
        );
        client.close().await.unwrap();
        assert!(client.next_event().await.unwrap().is_none());
        task.await.unwrap();
    })
    .await
    .expect("margin client close timed out");
}

async fn receive(socket: &mut WebSocket, events: bool) -> kalshi_sdk_rs::Result<Option<Event>> {
    if events {
        socket.next_event().await
    } else {
        Ok(socket.next_message().await?.map(Event::Message))
    }
}

#[tokio::test]
async fn both_margin_receive_apis_preserve_decode_transport_and_protocol_errors() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for events in [false, true] {
            let (mut client, mut server) = local_socket_pair().await;
            for frame in [Frame::Text("not JSON".into()), Frame::Binary(br#"{"type":"ticker","sid":1,"msg":{}}"#.to_vec().into())] {
                server.send(frame).await.unwrap();
                assert!(matches!(receive(&mut client, events).await, Err(Error::Json(_))));
            }
            server.send(Frame::Text("[]".into())).await.unwrap();
            assert!(matches!(receive(&mut client, events).await, Err(Error::Config(_))));
            server.send(Frame::Text(delta().to_string().into())).await.unwrap();
            assert!(matches!(receive(&mut client, events).await.unwrap(), Some(Event::Message(Message::OrderbookDelta(_)))));
            server.send(Frame::Text(r#"{"type":"error","id":7,"msg":{"code":11,"msg":"Invalid update interval"}}"#.into())).await.unwrap();
            assert!(matches!(receive(&mut client, events).await.unwrap(), Some(Event::Message(Message::Error(v))) if v.id == Some(7) && v.msg.code == 11));
            drop(server);
            assert!(matches!(receive(&mut client, events).await, Err(Error::WebSocket(_))));
        }
    }).await.expect("margin error recovery timed out");
}
