mod common;
use base64::{engine::general_purpose::STANDARD, Engine};
use kalshi_sdk_rs::{Credentials, Error};
use ring::signature::{UnparsedPublicKey, ED25519};
use rsa::{
    pkcs1::EncodeRsaPrivateKey,
    pkcs8::EncodePrivateKey,
    pss::{Signature, VerifyingKey},
    signature::Verifier,
    RsaPrivateKey,
};

#[test]
fn ed25519_signs_canonical_raw_message_without_query() {
    let (auth, public) = common::credentials();
    let signature = auth
        .sign(
            1_700_000_000_000,
            "get",
            "/trade-api/v2/portfolio/orders?limit=5",
        )
        .unwrap();
    let bytes = STANDARD.decode(&signature).unwrap();
    assert_eq!(bytes.len(), 64);
    UnparsedPublicKey::new(&ED25519, &public)
        .verify(b"1700000000000GET/trade-api/v2/portfolio/orders", &bytes)
        .unwrap();
    assert_eq!(
        signature,
        auth.sign(1_700_000_000_000, "GET", "/trade-api/v2/portfolio/orders")
            .unwrap()
    );
    assert!(UnparsedPublicKey::new(&ED25519, &public)
        .verify(
            b"1700000000000GET/trade-api/v2/portfolio/orders?limit=5",
            &bytes
        )
        .is_err());
    assert!(matches!(
        auth.sign(1, "GET", "https://host/path"),
        Err(Error::Auth(_))
    ));
}

#[test]
fn rsa_pkcs1_and_pkcs8_use_sha256_and_digest_length_pss_salt() {
    // Independent verifier (rsa) checks signatures made by ring. No private key fixtures.
    let key = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
    let verifier = VerifyingKey::<rsa::sha2::Sha256>::new_with_salt_len(key.to_public_key(), 32);
    let messages = [
        key.to_pkcs1_pem(Default::default()).unwrap(),
        key.to_pkcs8_pem(Default::default()).unwrap(),
    ];
    for pem in messages {
        let auth = Credentials::from_pem("rsa-test", pem.as_str()).unwrap();
        let a = STANDARD
            .decode(
                auth.sign(42, "POST", "/trade-api/v2/portfolio/events/orders?a=b")
                    .unwrap(),
            )
            .unwrap();
        let b = STANDARD
            .decode(
                auth.sign(42, "POST", "/trade-api/v2/portfolio/events/orders")
                    .unwrap(),
            )
            .unwrap();
        verifier
            .verify(
                b"42POST/trade-api/v2/portfolio/events/orders",
                &Signature::try_from(a.as_slice()).unwrap(),
            )
            .unwrap();
        assert_ne!(a, b, "RSA-PSS must use fresh random salt");
        assert!(verifier
            .verify(
                b"42GET/trade-api/v2/portfolio/events/orders",
                &Signature::try_from(a.as_slice()).unwrap()
            )
            .is_err());
    }
}

#[test]
fn credentials_debug_and_errors_redact_keys() {
    let (auth, _) = common::credentials();
    let debug = format!("{auth:?}");
    assert!(debug.contains("REDACTED"));
    assert!(!debug.contains("test-key-id"));
    let error = Credentials::from_pem("id", "secret invalid key contents")
        .unwrap_err()
        .to_string();
    assert!(!error.contains("secret"));
    assert!(Credentials::from_pem("", "bad PEM").is_err());
}
