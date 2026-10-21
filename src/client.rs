use crate::{Credentials, Error, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::{sync::Arc, time::Duration};
use url::Url;

/// Predictions API environment. Credentials are specific to each environment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Environment {
    /// Production exchange; requests can trade real funds.
    Production,
    /// Separate demo exchange, recommended for initial development.
    Demo,
}
impl Environment {
    /// Recommended REST base URL, including the API version prefix.
    pub fn rest_url(self) -> &'static str {
        match self {
            Self::Production => "https://external-api.kalshi.com/trade-api/v2",
            Self::Demo => "https://external-api.demo.kalshi.co/trade-api/v2",
        }
    }
    /// Recommended Predictions WebSocket URL.
    pub fn websocket_url(self) -> &'static str {
        match self {
            Self::Production => "wss://external-api-ws.kalshi.com/trade-api/ws/v2",
            Self::Demo => "wss://external-api-ws.demo.kalshi.co/trade-api/ws/v2",
        }
    }
}

/// Cheaply cloneable asynchronous client sharing a connection pool and parsed key.
/// Requests are sent once: there are no automatic retries, including order writes.
#[derive(Clone, Debug)]
pub struct Client {
    pub(crate) http: reqwest::Client,
    pub(crate) credentials: Option<Arc<Credentials>>,
    pub(crate) base_url: Url,
    pub(crate) websocket_url: Url,
    pub(crate) timeout: Duration,
}

/// Client configuration. Redirects are disabled to avoid forwarding signed headers.
#[derive(Debug)]
pub struct ClientBuilder {
    environment: Environment,
    credentials: Option<Credentials>,
    base_url: Option<String>,
    websocket_url: Option<String>,
    timeout: Duration,
}
impl ClientBuilder {
    /// Configure credentials. Public endpoints will also be signed.
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }
    /// Override the REST root (including `/trade-api/v2`) for a proxy or mock server.
    /// HTTP is accepted for local mocks. The caller controls the destination receiving credentials.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }
    /// Override the complete WebSocket URL for a proxy or mock server.
    pub fn websocket_url(mut self, url: impl Into<String>) -> Self {
        self.websocket_url = Some(url.into());
        self
    }
    /// Set REST request and WebSocket handshake timeouts (default 30 seconds).
    /// WebSocket reads have no timeout; use `tokio::time::timeout` if desired.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    /// Validate URLs and create the HTTP connection pool.
    pub fn build(self) -> Result<Client> {
        fn parse(value: &str, schemes: &[&str]) -> Result<Url> {
            let url = Url::parse(value).map_err(|_| Error::Config("invalid API URL".into()))?;
            if !schemes.contains(&url.scheme())
                || url.host_str().is_none()
                || url.query().is_some()
                || url.fragment().is_some()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(Error::Config("API URLs require a supported scheme and host, without credentials, query or fragment".into()));
            }
            Ok(url)
        }
        let base_url = parse(
            self.base_url
                .as_deref()
                .unwrap_or(self.environment.rest_url()),
            &["http", "https"],
        )?;
        let websocket_url = parse(
            self.websocket_url
                .as_deref()
                .unwrap_or(self.environment.websocket_url()),
            &["ws", "wss"],
        )?;
        let http = reqwest::Client::builder()
            .timeout(self.timeout)
            .connect_timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("kalshi-sdk-rs/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Client {
            http,
            credentials: self.credentials.map(Arc::new),
            base_url,
            websocket_url,
            timeout: self.timeout,
        })
    }
}
impl Client {
    /// Create a public client for an explicitly chosen environment.
    pub fn new(environment: Environment) -> Result<Self> {
        Self::builder(environment).build()
    }
    /// Configure authentication, timeouts and endpoint overrides.
    pub fn builder(environment: Environment) -> ClientBuilder {
        ClientBuilder {
            environment,
            credentials: None,
            base_url: None,
            websocket_url: None,
            timeout: Duration::from_secs(30),
        }
    }
    pub(crate) async fn request<
        T: DeserializeOwned,
        Q: Serialize + ?Sized,
        B: Serialize + ?Sized,
    >(
        &self,
        method: reqwest::Method,
        path: &str,
        query: Option<&Q>,
        body: Option<&B>,
        auth_required: bool,
    ) -> Result<T> {
        if auth_required && self.credentials.is_none() {
            return Err(Error::Auth("this endpoint requires credentials"));
        }
        let mut url = self.base_url.clone();
        url.set_path(&format!(
            "{}{}",
            self.base_url.path().trim_end_matches('/'),
            path
        ));
        if let Some(query) = query {
            let value = serde_json::to_value(query)?;
            let object = value
                .as_object()
                .ok_or_else(|| Error::Config("query must be an object".into()))?;
            let mut pairs = url.query_pairs_mut();
            for (key, value) in object {
                if value.is_null() {
                    continue;
                }
                let values = match value {
                    serde_json::Value::Array(v) => v.as_slice(),
                    _ => std::slice::from_ref(value),
                };
                for value in values {
                    let text = match value {
                        serde_json::Value::String(s) => s.clone(),
                        _ => value.to_string(),
                    };
                    pairs.append_pair(key, &text);
                }
            }
        }
        let mut request = self.http.request(method.clone(), url.clone());
        if let Some(credentials) = &self.credentials {
            request = request.headers(credentials.headers(method.as_str(), url.path())?);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await?;
        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = response.bytes().await?;
        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes).into_owned();
            let error = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| serde_json::from_value(v.get("error").unwrap_or(&v).clone()).ok())
                .unwrap_or_default();
            return Err(Error::Api {
                status: status.as_u16(),
                error,
                body,
                retry_after,
            });
        }
        if bytes.is_empty() {
            return Ok(serde_json::from_slice(b"null")?);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}

pub(crate) fn segment(value: &str) -> Result<String> {
    if value.is_empty() || value == "." || value == ".." {
        return Err(Error::Config(
            "path identifiers must be nonempty and must not be dot segments".into(),
        ));
    }
    Ok(
        percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)
            .to_string(),
    )
}
