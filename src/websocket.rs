//! Predictions WebSocket commands and typed incoming messages.
//!
//! Subscription IDs come from server acknowledgments; command IDs identify your
//! requests. No reconnection or orderbook reconstruction is performed. Reconnect,
//! resubscribe and obtain a fresh snapshot after a disconnect or sequence gap.

use crate::{Client, Error, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
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

/// A connected Predictions WebSocket. Drive [`WebSocket::next_message`] to process
/// incoming frames, including ping/pong and close handshakes.
pub struct WebSocket {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    closed: bool,
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
            closed: false,
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
    /// Receive a typed message. Returns `None` on clean disconnect, and transport or
    /// decode errors separately. Ping frames receive matching pongs automatically.
    /// Protocol `error` replies are returned as [`Message::Error`], with command IDs.
    pub async fn next_message(&mut self) -> Result<Option<Message>> {
        if self.closed {
            return Ok(None);
        }
        while let Some(frame) = self.socket.next().await {
            match frame? {
                Frame::Text(text) => return Ok(Some(Message::from_json(&text)?)),
                Frame::Binary(bytes) => return Ok(Some(Message::from_slice(&bytes)?)),
                Frame::Ping(_) => self.socket.flush().await?,
                Frame::Close(_) => {
                    self.closed = true;
                    match self.socket.flush().await {
                        Ok(()) | Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed) => {}
                        Err(e) => return Err(e.into()),
                    }
                    return Ok(None);
                }
                Frame::Pong(_) | Frame::Frame(_) => {}
            }
        }
        self.closed = true;
        Ok(None)
    }
    /// Send a WebSocket ping with caller-supplied payload (maximum 125 bytes).
    pub async fn ping(&mut self, payload: Vec<u8>) -> Result<()> {
        self.socket.send(Frame::Ping(payload.into())).await?;
        Ok(())
    }
    /// Initiate a clean close. Call `next_message` until `None` to finish the handshake.
    pub async fn close(&mut self) -> Result<()> {
        self.socket.close(None).await?;
        Ok(())
    }
}

#[path = "websocket_messages.rs"]
mod messages;
pub use messages::Message;
