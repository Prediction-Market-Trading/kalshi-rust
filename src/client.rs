use crate::{Credentials, Error, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::{sync::Arc, time::Duration};
use url::Url;

/// API environment for Predictions and Perps. Credentials are specific to each environment.
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
    /// Recommended Perps WebSocket URL, including the margin handshake path.
    pub fn margin_websocket_url(self) -> &'static str {
        match self {
            Self::Production => "wss://external-api-margin-ws.kalshi.com/trade-api/ws/v2/margin",
            Self::Demo => "wss://external-api-margin-ws.demo.kalshi.co/trade-api/ws/v2/margin",
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
    pub(crate) margin_websocket_url: Url,
    pub(crate) timeout: Duration,
}

/// Client configuration. Redirects are disabled to avoid forwarding signed headers.
#[derive(Debug)]
pub struct ClientBuilder {
    environment: Environment,
    credentials: Option<Credentials>,
    base_url: Option<String>,
    websocket_url: Option<String>,
    margin_websocket_url: Option<String>,
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
    /// Override the complete Predictions WebSocket URL for a proxy or mock server.
    pub fn websocket_url(mut self, url: impl Into<String>) -> Self {
        self.websocket_url = Some(url.into());
        self
    }
    /// Override the complete Perps WebSocket URL independently of Predictions.
    /// May also select Kalshi's low-latency margin user-stream host for `fill` and `user_orders`.
    /// The caller controls the destination receiving signed authentication headers.
    pub fn margin_websocket_url(mut self, url: impl Into<String>) -> Self {
        self.margin_websocket_url = Some(url.into());
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
        let margin_websocket_url = parse(
            self.margin_websocket_url
                .as_deref()
                .unwrap_or(self.environment.margin_websocket_url()),
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
            margin_websocket_url,
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
            margin_websocket_url: None,
            timeout: Duration::from_secs(30),
        }
    }
    /// Access the Perps trading API, sharing this client's credentials and HTTP pool.
    pub fn margin(&self) -> crate::margin::Margin<'_> {
        crate::margin::Margin { client: self }
    }

    // OpenAPI form arrays with explode=false are one comma-separated query value.
    // Adapt only the named fields; the shared request transport retains Predictions' repeated keys.
    pub(crate) async fn request_with_csv_query<
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
        csv_fields: &[&str],
    ) -> Result<T> {
        let mut query = query.map(serde_json::to_value).transpose()?;
        if let Some(query) = query.as_mut() {
            let object = query
                .as_object_mut()
                .ok_or_else(|| Error::Config("query must be an object".into()))?;
            for field in csv_fields {
                if let Some(serde_json::Value::Array(values)) = object.get_mut(*field) {
                    let joined = values
                        .iter()
                        .map(|value| match value {
                            serde_json::Value::String(value) => value.clone(),
                            _ => value.to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join(",");
                    object.insert((*field).into(), serde_json::Value::String(joined));
                }
            }
        }
        self.request(method, path, query.as_ref(), body, auth_required)
            .await
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

// Preserve RFC 3986 unreserved characters; all identifiers are raw input.
const PATH_SEGMENT_ENCODE_SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub(crate) fn segment(value: &str) -> Result<String> {
    if value.is_empty() || value == "." || value == ".." {
        return Err(Error::Config(
            "path identifiers must be nonempty and must not be dot segments".into(),
        ));
    }
    Ok(percent_encoding::utf8_percent_encode(value, PATH_SEGMENT_ENCODE_SET).to_string())
}

#[cfg(test)]
mod tests {
    use super::segment;
    use crate::Error;

    #[test]
    fn margin_and_predictions_websocket_destinations_are_independent() {
        use super::{Client, Environment};
        for (environment, margin_url) in [
            (
                Environment::Demo,
                "wss://external-api-margin-ws.demo.kalshi.co/trade-api/ws/v2/margin",
            ),
            (
                Environment::Production,
                "wss://external-api-margin-ws.kalshi.com/trade-api/ws/v2/margin",
            ),
        ] {
            let client = Client::new(environment).unwrap();
            assert_eq!(client.margin_websocket_url.as_str(), margin_url);
            assert_eq!(client.websocket_url.as_str(), environment.websocket_url());
            let client = Client::builder(environment)
                .websocket_url("ws://localhost:1234/predictions")
                .build()
                .unwrap();
            assert_eq!(client.margin_websocket_url.as_str(), margin_url);
            let client = Client::builder(environment)
                .margin_websocket_url("ws://localhost:5678/margin")
                .build()
                .unwrap();
            assert_eq!(
                client.margin_websocket_url.as_str(),
                "ws://localhost:5678/margin"
            );
            assert_eq!(client.websocket_url.as_str(), environment.websocket_url());
        }
    }

    #[test]
    fn margin_websocket_override_requires_a_supported_url_without_embedded_credentials() {
        use super::{Client, Environment};
        for url in [
            "not a URL",
            "https://example.com/margin",
            "ws:///",
            "wss://user:password@example.com/margin",
            "wss://example.com/margin?key=value",
            "wss://example.com/margin#fragment",
        ] {
            assert!(
                matches!(
                    Client::builder(Environment::Demo)
                        .margin_websocket_url(url)
                        .build(),
                    Err(Error::Config(_))
                ),
                "{url}"
            );
        }
    }

    #[test]
    fn segments_preserve_unreserved_characters() {
        for value in [
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~",
            "123e4567-e89b-12d3-a456-426614174000",
            ".hidden",
            "..prefix",
            "...",
        ] {
            assert_eq!(segment(value).unwrap(), value);
        }
    }

    #[test]
    fn segments_encode_reserved_characters_and_raw_utf8() {
        for (raw, expected) in [
            (
                ":/?#[]@!$&'()*+,;= %\\",
                "%3A%2F%3F%23%5B%5D%40%21%24%26%27%28%29%2A%2B%2C%3B%3D%20%25%5C",
            ),
            ("café/東京🦀", "caf%C3%A9%2F%E6%9D%B1%E4%BA%AC%F0%9F%A6%80"),
            ("%2F%2D%2E", "%252F%252D%252E"),
        ] {
            assert_eq!(segment(raw).unwrap(), expected, "raw identifier: {raw:?}");
        }
    }

    #[test]
    fn segments_reject_empty_and_complete_dot_segments() {
        for value in ["", ".", ".."] {
            assert!(matches!(segment(value), Err(Error::Config(_))));
        }
    }
}
