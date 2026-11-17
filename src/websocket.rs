//! Predictions WebSocket commands and typed incoming messages.
//!
//! Subscription IDs come from server acknowledgments; command IDs identify your
//! requests. No reconnection or orderbook reconstruction is performed. Reconnect,
//! resubscribe and obtain a fresh snapshot after a disconnect or sequence gap.

use crate::websocket_transport::{Connection, RawEvent};
use crate::{Client, Error, Result};
use serde::Serialize;

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
    connection: Connection,
}

impl Client {
    /// Connect with signed `GET /trade-api/ws/v2` headers. All Predictions channels
    /// require an authenticated handshake, including public market data channels.
    pub async fn connect_websocket(&self) -> Result<WebSocket> {
        Ok(WebSocket {
            connection: Connection::connect(self, &self.websocket_url).await?,
        })
    }
}

impl WebSocket {
    /// Send a typed command from [`models`]. Prefer the helpers for ordinary subscriptions.
    pub async fn send_command<T: Serialize + ?Sized>(&mut self, command: &T) -> Result<()> {
        self.connection.send_command(command).await
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
        Ok(match self.connection.next_event().await? {
            Some(RawEvent::Message(frame)) => {
                Some(Event::Message(Message::from_slice(&frame.into_data())?))
            }
            Some(RawEvent::Ping(payload)) => Some(Event::Ping(payload)),
            Some(RawEvent::Pong(payload)) => Some(Event::Pong(payload)),
            None => None,
        })
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
        self.connection.ping(payload).await
    }
    /// Initiate a clean close. Call [`Self::next_event`] or [`Self::next_message`]
    /// until `None` to finish the handshake.
    pub async fn close(&mut self) -> Result<()> {
        self.connection.close().await
    }
}

#[path = "websocket_messages.rs"]
mod messages;
pub use messages::Message;
