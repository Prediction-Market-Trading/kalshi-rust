use kalshi_sdk_rs::Credentials;
use ring::{
    rand::SystemRandom,
    signature::{Ed25519KeyPair, KeyPair},
};

pub fn credentials() -> (Credentials, Vec<u8>) {
    // Ephemeral test key: generated in memory, never written to disk.
    let der = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    let key = Ed25519KeyPair::from_pkcs8(der.as_ref()).unwrap();
    let pem = pem::encode(&pem::Pem::new("PRIVATE KEY", der.as_ref()));
    (
        Credentials::from_pem("test-key-id", &pem).unwrap(),
        key.public_key().as_ref().to_vec(),
    )
}
