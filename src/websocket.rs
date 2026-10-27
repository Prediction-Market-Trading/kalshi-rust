//! Predictions WebSocket commands and typed incoming messages.
//!
//! Subscription IDs come from server acknowledgments; command IDs identify your
//! requests. No reconnection or orderbook reconstruction is performed. Reconnect,
//! resubscribe and obtain a fresh snapshot after a disconnect or sequence gap.

use crate::{Client, Error, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Message as Frame},
    MaybeTlsStream, WebSocketStream,
};

/// Full wire models, including envelopes, channel payloads and command parameters.
#[path = "websocket_models.rs"]
pub mod models;
pub use models::{
    SubscribeCommandPayloadParams as SubscribeParams,
    SubscribeCommandPayloadParamsChannelsItem as Channel,
    UpdateSubscriptionCommandPayloadParams as UpdateParams,
    UpdateSubscriptionCommandPayloadParamsAction as UpdateAction,
};

/// An incoming WebSocket message or control frame.
#[derive(Clone, Debug)]
pub enum Event {
    /// A typed application message decoded from a text or binary frame.
    Message(Message),
    /// A server ping with its exact payload. Its automatic matching pong has been flushed.
    /// Kalshi's heartbeat pings are exposed here without special payload decoding.
    Ping(Vec<u8>),
    /// A received pong with its exact payload, including replies to [`WebSocket::ping`].
    Pong(Vec<u8>),
}

/// A connected Predictions WebSocket. Drive [`WebSocket::next_event`] or
/// [`WebSocket::next_message`] to process incoming frames and control handshakes.
/// There is no background reader.
pub struct WebSocket {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    receive: ReceiveState,
}

/// Control frames must survive cancellation while their automatic replies flush.
#[derive(Default)]
enum ReceiveState {
    #[default]
    Reading,
    Ping(Vec<u8>),
    Closing,
    Closed,
}

impl ReceiveState {
    async fn next_event<S>(&mut self, socket: &mut WebSocketStream<S>) -> Result<Option<Event>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        loop {
            match self {
                Self::Closed => return Ok(None),
                Self::Ping(payload) => {
                    socket.flush().await?;
                    let event = Event::Ping(std::mem::take(payload));
                    *self = Self::Reading;
                    return Ok(Some(event));
                }
                Self::Closing => {
                    let result = socket.flush().await;
                    *self = Self::Closed;
                    match result {
                        Ok(()) | Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed) => {}
                        Err(e) => return Err(e.into()),
                    }
                    return Ok(None);
                }
                Self::Reading => {}
            }
            match socket.next().await {
                Some(frame) => match frame? {
                    Frame::Text(text) => {
                        return Ok(Some(Event::Message(Message::from_json(&text)?)))
                    }
                    Frame::Binary(bytes) => {
                        return Ok(Some(Event::Message(Message::from_slice(&bytes)?)));
                    }
                    Frame::Ping(payload) => *self = Self::Ping(payload.to_vec()),
                    Frame::Pong(payload) => return Ok(Some(Event::Pong(payload.to_vec()))),
                    Frame::Close(_) => *self = Self::Closing,
                    Frame::Frame(_) => {}
                },
                None => {
                    *self = Self::Closed;
                    return Ok(None);
                }
            }
        }
    }
}

impl Client {
    /// Connect with signed `GET /trade-api/ws/v2` headers. All Predictions channels
    /// require an authenticated handshake, including public market data channels.
    pub async fn connect_websocket(&self) -> Result<WebSocket> {
        let credentials = self
            .credentials
            .as_ref()
            .ok_or(Error::Auth("WebSocket connection requires credentials"))?;
        let mut request = self.websocket_url.as_str().into_client_request()?;
        let headers = credentials.headers("GET", self.websocket_url.path())?;
        for (name, value) in &headers {
            request.headers_mut().insert(name.clone(), value.clone());
        }
        let (socket, _) =
            tokio::time::timeout(self.timeout, tokio_tungstenite::connect_async(request))
                .await
                .map_err(|_| Error::Config("WebSocket handshake timed out".into()))??;
        Ok(WebSocket {
            socket,
            receive: ReceiveState::default(),
        })
    }
}
impl WebSocket {
    /// Send a typed command from [`models`]. Prefer the helpers for ordinary subscriptions.
    pub async fn send_command<T: Serialize + ?Sized>(&mut self, command: &T) -> Result<()> {
        self.socket
            .send(Frame::Text(serde_json::to_string(command)?.into()))
            .await?;
        Ok(())
    }
    /// Subscribe; inspect the returned `subscribed` message for the subscription ID.
    /// Set `use_yes_price` explicitly for orderbooks to choose a price convention.
    pub async fn subscribe(&mut self, id: i64, params: SubscribeParams) -> Result<()> {
        if params.channels.is_empty() {
            return Err(Error::Config(
                "at least one WebSocket channel is required".into(),
            ));
        }
        self.send_command(&models::SubscribeCommandPayload {
            id,
            cmd: "subscribe".into(),
            params,
        })
        .await
    }
    /// Unsubscribe server-assigned subscription IDs.
    pub async fn unsubscribe(&mut self, id: i64, sids: Vec<i64>) -> Result<()> {
        self.send_command(&models::UnsubscribeCommandPayload {
            id,
            cmd: "unsubscribe".into(),
            params: models::UnsubscribeCommandPayloadParams { sids },
        })
        .await
    }
    /// Ask the server to list active subscriptions. The reply is [`Message::Subscriptions`].
    pub async fn list_subscriptions(&mut self, id: i64) -> Result<()> {
        self.send_command(&models::ListSubscriptionsCommandPayload {
            id,
            cmd: "list_subscriptions".into(),
        })
        .await
    }
    /// Add/remove markets or request a new snapshot for one subscription ID.
    pub async fn update_subscription(&mut self, id: i64, params: UpdateParams) -> Result<()> {
        self.send_command(&models::UpdateSubscriptionCommandPayload {
            id,
            cmd: "update_subscription".into(),
            params,
        })
        .await
    }
    /// Receive a typed message or an incoming ping/pong, preserving control payloads.
    /// Matching pongs are flushed automatically before returning [`Event::Ping`].
    /// Returns `None` on clean disconnect; transport and decode failures are errors.
    /// Protocol `error` replies remain [`Event::Message`] containing [`Message::Error`].
    ///
    /// Cancelling this read retains pending frame parsing and control replies for
    /// the next read. Both receive methods consume the same ordered stream.
    /// Keep driving reads to service heartbeats. A timeout around this method can
    /// detect inactivity even when no application messages are expected:
    ///
    /// ```rust,no_run
    /// use kalshi_sdk_rs::websocket::{Event, WebSocket};
    /// use std::time::Duration;
    /// # async fn receive(socket: &mut WebSocket) -> Result<(), Box<dyn std::error::Error>> {
    /// // The application chooses its inactivity threshold.
    /// while let Some(event) = tokio::time::timeout(
    ///     Duration::from_secs(30), socket.next_event(),
    /// ).await?? {
    ///     match event {
    ///         Event::Message(message) => println!("{message:?}"),
    ///         Event::Ping(_) | Event::Pong(_) => {} // Also resets the next read's timeout.
    ///     }
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn next_event(&mut self) -> Result<Option<Event>> {
        self.receive.next_event(&mut self.socket).await
    }

    /// Receive a typed message, filtering out incoming ping/pong events.
    /// Returns `None` on clean disconnect, and transport or decode errors separately.
    /// Ping frames receive matching pongs automatically. Protocol `error` replies
    /// are returned as [`Message::Error`], with command IDs.
    ///
    /// A timeout around this method measures application-message inactivity, even
    /// while heartbeats are arriving. Use [`Self::next_event`] to observe that activity.
    pub async fn next_message(&mut self) -> Result<Option<Message>> {
        while let Some(event) = self.next_event().await? {
            if let Event::Message(message) = event {
                return Ok(Some(message));
            }
        }
        Ok(None)
    }
    /// Send a WebSocket ping with caller-supplied payload (maximum 125 bytes).
    pub async fn ping(&mut self, payload: Vec<u8>) -> Result<()> {
        self.socket.send(Frame::Ping(payload.into())).await?;
        Ok(())
    }
    /// Initiate a clean close. Call [`Self::next_event`] or [`Self::next_message`]
    /// until `None` to finish the handshake.
    pub async fn close(&mut self) -> Result<()> {
        self.socket.close(None).await?;
        Ok(())
    }
}

#[path = "websocket_messages.rs"]
mod messages;
pub use messages::Message;

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;
    use tokio_tungstenite::tungstenite::protocol::Role;

    #[tokio::test]
    async fn cancelled_control_reply_flushes_resume_without_losing_events() {
        tokio::time::timeout(Duration::from_secs(5), async {
            for frame in [
                Frame::Ping(b"heartbeat".to_vec().into()),
                Frame::Close(None),
            ] {
                let (client, server) = tokio::io::duplex(64);
                let mut client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
                let mut server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
                let mut receive = ReceiveState::default();

                // A masked client frame uses 6 header bytes. Fill 60 of the 64 bytes
                // so either automatic reply can write only part of its frame.
                client
                    .send(Frame::Text("x".repeat(54).into()))
                    .await
                    .unwrap();
                server.send(frame.clone()).await.unwrap();
                assert!(receive.next_event(&mut client).now_or_never().is_none());

                // Drain the unrelated data directly (the server may already be closing).
                // Any partially written control reply remains for server.next() to parse.
                let mut unrelated = [0; 60];
                server.get_mut().read_exact(&mut unrelated).await.unwrap();
                let event = receive.next_event(&mut client).await.unwrap();
                match frame {
                    Frame::Ping(payload) => {
                        assert!(matches!(event, Some(Event::Ping(bytes)) if bytes == payload));
                        assert_eq!(server.next().await.unwrap().unwrap(), Frame::Pong(payload));
                        // The retained Ping was delivered exactly once.
                        assert!(receive.next_event(&mut client).now_or_never().is_none());
                    }
                    Frame::Close(_) => {
                        assert!(event.is_none());
                        assert!(matches!(
                            server.next().await.unwrap().unwrap(),
                            Frame::Close(_)
                        ));
                        assert!(receive.next_event(&mut client).await.unwrap().is_none());
                    }
                    _ => unreachable!(),
                }
            }
        })
        .await
        .expect("cancelled control reply did not resume");
    }
}
