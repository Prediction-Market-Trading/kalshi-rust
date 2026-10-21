use serde::{Deserialize, Serialize};

/// SDK result, including transport, decoding and exchange errors.
pub type Result<T> = std::result::Result<T, Error>;

/// Error details returned by Kalshi. Extra fields remain available in [`Error::Api`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiError {
    /// Exchange error code, such as `invalid_parameters`.
    pub code: Option<String>,
    /// Human-readable exchange message.
    pub message: Option<String>,
    /// Additional structured error details.
    pub details: Option<serde_json::Value>,
    /// Originating exchange service, when supplied.
    pub service: Option<String>,
}

/// Errors preserve HTTP status and distinguish API failures from invalid success bodies.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// HTTP transport or request construction failure.
    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),
    /// A non-success HTTP response. Bodies are retained for diagnosis, never logged by the SDK.
    #[error("Kalshi returned HTTP {status}: {error:?}")]
    Api {
        /// HTTP response status.
        status: u16,
        /// Parsed structured error, if available.
        error: Box<ApiError>,
        /// Original error body (may contain account information).
        body: String,
        /// Retry-After response header, in its original seconds/date representation.
        retry_after: Option<String>,
    },
    /// A successful response or WebSocket message could not be decoded.
    #[error("JSON decoding error: {0}")]
    Json(#[from] serde_json::Error),
    /// Invalid or missing configuration. Key contents are never included.
    #[error("configuration error: {0}")]
    Config(String),
    /// Key parsing or signing failed. Key contents are never included.
    #[error("authentication error: {0}")]
    Auth(&'static str),
    /// File loading failure.
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    /// WebSocket protocol or transport error.
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] Box<tokio_tungstenite::tungstenite::Error>),
    /// Invalid decimal syntax, scale or representable range.
    #[error("invalid fixed-point value: {0}")]
    FixedPoint(&'static str),
}

impl From<tokio_tungstenite::tungstenite::Error> for Error {
    fn from(value: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::WebSocket(Box::new(value))
    }
}
