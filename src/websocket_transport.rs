//! Shared WebSocket framing, signed handshakes and cancellation-safe control replies.
use crate::{Client, Error, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Message as Frame},
    MaybeTlsStream, WebSocketStream,
};
use url::Url;

pub(crate) enum RawEvent {
    Message(Frame),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
}

pub(crate) struct Connection {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    receive: ReceiveState,
}

impl Connection {
    pub(crate) async fn connect(client: &Client, url: &Url) -> Result<Self> {
        let credentials = client
            .credentials
            .as_ref()
            .ok_or(Error::Auth("WebSocket connection requires credentials"))?;
        let mut request = url.as_str().into_client_request()?;
        let headers = credentials.headers("GET", url.path())?;
        for (name, value) in &headers {
            request.headers_mut().insert(name.clone(), value.clone());
        }
        let (socket, _) =
            tokio::time::timeout(client.timeout, tokio_tungstenite::connect_async(request))
                .await
                .map_err(|_| Error::Config("WebSocket handshake timed out".into()))??;
        Ok(Self {
            socket,
            receive: ReceiveState::default(),
        })
    }
    pub(crate) async fn send_command<T: Serialize + ?Sized>(&mut self, command: &T) -> Result<()> {
        self.socket
            .send(Frame::Text(serde_json::to_string(command)?.into()))
            .await?;
        Ok(())
    }
    pub(crate) async fn next_event(&mut self) -> Result<Option<RawEvent>> {
        self.receive.next_event(&mut self.socket).await
    }
    pub(crate) async fn ping(&mut self, payload: Vec<u8>) -> Result<()> {
        self.socket.send(Frame::Ping(payload.into())).await?;
        Ok(())
    }
    pub(crate) async fn close(&mut self) -> Result<()> {
        self.socket.close(None).await?;
        Ok(())
    }
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
    async fn next_event<S>(&mut self, socket: &mut WebSocketStream<S>) -> Result<Option<RawEvent>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        loop {
            match self {
                Self::Closed => return Ok(None),
                Self::Ping(payload) => {
                    socket.flush().await?;
                    let event = RawEvent::Ping(std::mem::take(payload));
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
                    Frame::Text(text) => return Ok(Some(RawEvent::Message(Frame::Text(text)))),
                    Frame::Binary(bytes) => {
                        return Ok(Some(RawEvent::Message(Frame::Binary(bytes))));
                    }
                    Frame::Ping(payload) => *self = Self::Ping(payload.to_vec()),
                    Frame::Pong(payload) => return Ok(Some(RawEvent::Pong(payload.to_vec()))),
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
                        assert!(matches!(event, Some(RawEvent::Ping(bytes)) if bytes == payload));
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
