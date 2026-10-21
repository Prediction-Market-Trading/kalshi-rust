use crate::{Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::{
    rand::SystemRandom,
    signature::{Ed25519KeyPair, RsaKeyPair, RSA_PSS_SHA256},
};
use std::{
    fmt,
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

enum Key {
    Ed25519(Ed25519KeyPair),
    Rsa(RsaKeyPair),
}

/// Parsed API credentials. Clones share the key; Debug always redacts it.
#[derive(Clone)]
pub struct Credentials {
    key_id: String,
    key: Arc<Key>,
}
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}
impl Credentials {
    /// Parse Ed25519 PKCS#8, RSA PKCS#8 or RSA PKCS#1 PEM.
    /// Encrypted PEMs are not supported. Algorithm detection uses the parsed key.
    pub fn from_pem(key_id: impl Into<String>, private_key: &str) -> Result<Self> {
        let key_id = key_id.into();
        if key_id.is_empty() {
            return Err(Error::Auth("API key ID must not be empty"));
        }
        let parsed = pem::parse(private_key).map_err(|_| Error::Auth("invalid PEM"))?;
        let tag = parsed.tag().to_owned();
        let der = Zeroizing::new(parsed.into_contents());
        let key = if tag == "RSA PRIVATE KEY" {
            Key::Rsa(
                RsaKeyPair::from_der(&der)
                    .map_err(|_| Error::Auth("invalid RSA PKCS#1 key (minimum 2048 bits)"))?,
            )
        } else if tag == "PRIVATE KEY" {
            if let Ok(key) = Ed25519KeyPair::from_pkcs8_maybe_unchecked(&der) {
                Key::Ed25519(key)
            } else {
                Key::Rsa(
                    RsaKeyPair::from_pkcs8(&der)
                        .map_err(|_| Error::Auth("unsupported or invalid PKCS#8 key"))?,
                )
            }
        } else {
            return Err(Error::Auth("expected unencrypted PKCS#8 or RSA PKCS#1 PEM"));
        };
        Ok(Self {
            key_id,
            key: Arc::new(key),
        })
    }
    /// Load a PEM file without logging its contents or path.
    pub fn from_file(key_id: impl Into<String>, path: impl AsRef<Path>) -> Result<Self> {
        let pem = Zeroizing::new(std::fs::read_to_string(path)?);
        Self::from_pem(key_id, &pem)
    }
    /// Load `KALSHI_API_KEY_ID` and either `KALSHI_PRIVATE_KEY_PEM` or `KALSHI_PRIVATE_KEY_PATH`.
    /// The PEM variable takes precedence. Newlines must be actual newlines.
    pub fn from_env() -> Result<Self> {
        let id = std::env::var("KALSHI_API_KEY_ID")
            .map_err(|_| Error::Config("KALSHI_API_KEY_ID is required".into()))?;
        if let Ok(pem) = std::env::var("KALSHI_PRIVATE_KEY_PEM") {
            Self::from_pem(id, &Zeroizing::new(pem))
        } else {
            let path = std::env::var("KALSHI_PRIVATE_KEY_PATH").map_err(|_| {
                Error::Config(
                    "KALSHI_PRIVATE_KEY_PEM or KALSHI_PRIVATE_KEY_PATH is required".into(),
                )
            })?;
            Self::from_file(id, path)
        }
    }
    /// Sign `timestamp_ms + UPPERCASE_METHOD + path`, excluding query parameters.
    /// RSA uses SHA-256, MGF1-SHA256 and a 32-byte PSS salt. Ed25519 signs the raw text.
    pub fn sign(&self, timestamp_ms: u64, method: &str, path: &str) -> Result<String> {
        if !path.starts_with('/') {
            return Err(Error::Auth("signing requires an absolute URL path"));
        }
        let path = path.split(['?', '#']).next().unwrap_or(path);
        let message = format!("{timestamp_ms}{}{path}", method.to_ascii_uppercase());
        let signature = match self.key.as_ref() {
            Key::Ed25519(key) => key.sign(message.as_bytes()).as_ref().to_vec(),
            Key::Rsa(key) => {
                let mut signature = vec![0; key.public().modulus_len()];
                key.sign(
                    &RSA_PSS_SHA256,
                    &SystemRandom::new(),
                    message.as_bytes(),
                    &mut signature,
                )
                .map_err(|_| Error::Auth("RSA-PSS signing failed"))?;
                signature
            }
        };
        Ok(STANDARD.encode(signature))
    }
    pub(crate) fn headers(&self, method: &str, path: &str) -> Result<reqwest::header::HeaderMap> {
        use reqwest::header::{HeaderMap, HeaderValue};
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Auth("system clock is before Unix epoch"))?
            .as_millis();
        let timestamp = u64::try_from(timestamp).map_err(|_| Error::Auth("timestamp overflow"))?;
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("kalshi-access-key", self.key_id.clone()),
            ("kalshi-access-timestamp", timestamp.to_string()),
            (
                "kalshi-access-signature",
                self.sign(timestamp, method, path)?,
            ),
        ] {
            let mut value = HeaderValue::from_str(&value)
                .map_err(|_| Error::Auth("invalid authentication header"))?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
        Ok(headers)
    }
}
