# Verification results

Completed October 20, 2026 (America/New_York), using `rustc 1.95.0 (59807616e 2026-04-14)` and Cargo 1.95.0 on `aarch64-apple-darwin`. The checked-in Cargo.lock records the dependency resolution. CI is configured for Rust 1.95.0 and stable on Linux; remote CI has not been run from this workspace.

| Required check | Result |
| --- | --- |
| `cargo fmt --check` | Passed |
| `cargo check --all-targets` | Passed; library, tests and all seven examples compile |
| `cargo test` | Passed: 24 automated tests and 5 compiling README/Rustdoc examples; 2 explicitly ignored demo integration tests |
| `cargo clippy --all-targets -- -D warnings` | Passed with no warnings |
| `cargo doc --no-deps` | Passed; also checked with `RUSTDOCFLAGS='-D warnings'` |

After dependency download, the final checks were run with Cargo's `--offline` option where applicable. Local HTTP/WebSocket tests require loopback socket permissions; the workspace sandbox initially denied port binding, so those tests were rerun with that permission and passed. No automated test contacts Kalshi unless explicitly selected with `--ignored` as documented.

## What the automated tests establish

- Ed25519 signs the expected canonical bytes, with uppercase method and excluded query; signature verification fails for the wrong message. RSA signatures from ring are independently checked using the rsa crate with SHA-256 and a 32-byte PSS salt. Both RSA PEM formats work. Tests generate private keys in memory and never commit key fixtures.
- Credentials and invalid-key errors do not expose private key text or API key IDs. Generated private-key responses have redacted Debug implementations.
- Decimal strings preserve exact values, fractional contracts, signed deltas and scale. Excess precision, invalid syntax, overflow and JSON numeric values are rejected. Orderbook tuples enforce separate price/count units.
- Request filters use the correct singular `status`; URL encoding preserves reserved characters, opaque cursors and repeated array parameters. Configured credentials sign public endpoints and the actual encoded request path. Missing required authentication fails before sending.
- Create/cancel/amend/decrease calls use the V2 paths and payloads. Tests cover nullable amendment responses, total-count versus reduction fields, routing ticker, and no-body 204 success. HTTP errors preserve status, details, body and Retry-After; bad success bodies are decoding errors. Redirects are not followed and timed-out requests are not retried.
- Captured unauthenticated production JSON decodes for status, markets, events, trades, orderbook and historical cutoff.
- All **38 official server-message examples** in the pinned AsyncAPI document decode into known typed variants. Deprecated fields can disappear without breaking the relevant payloads. Empty books, shared `ok`/lifecycle envelopes, future message types and malformed known messages are distinguished.
- A local WebSocket server verifies handshake signing, subscribe/list/update/unsubscribe commands, matching pong payloads, and both server- and client-initiated clean closes.

## Live public verification

`cargo run --example public_smoke` completed successfully against the production public Predictions API without loading credentials. It verified successful HTTP retrieval and typed decoding of:

1. `GET /exchange/status`
2. `GET /markets` (two markets)
3. `GET /markets/{ticker}`
4. `GET /markets/{ticker}/orderbook`
5. `GET /events` (200 events)
6. `GET /markets/trades`
7. `GET /historical/cutoff`

This was read-only. No order was submitted or canceled, and no account endpoint or authenticated WebSocket was exercised against Kalshi.

## Additional checks

Regenerating all generated source and the REST coverage table, then applying rustfmt, reproduced the existing artifacts byte-for-byte. `cargo package --offline --allow-dirty` built and verified the unpublished crate archive. Cargo emitted only the expected package-metadata notice that no hosted documentation/homepage/repository URL has been configured; no repository URL was invented and nothing was published. Building from the archive does not require Python, the generator or the specification snapshots.

## What remains unverified

No `KALSHI_API_KEY_ID`, `KALSHI_PRIVATE_KEY_PEM` or `KALSHI_PRIVATE_KEY_PATH` was configured. Consequently, **live signing acceptance, balances, positions, fills, order submission/cancellation/amendment, institutional endpoints, rate-limit entitlements and live account/market WebSocket streams remain unverified**. The two demo integration tests were intentionally left ignored. Their clients hardcode Demo, and the order test also requires `KALSHI_DEMO_ORDER_OPT_IN=YES` plus a valid demo market.

The SDK exposes 118 spec-defined operations, but only the public calls listed above have end-to-end live evidence. Broad generated coverage is supported by the pinned schemas, reusable tested transport, compilation and selected payload fixtures; it is not a substitute for testing each privileged operation with the right account permissions. There is no dependency security audit, measured test-coverage percentage, Linux run or older-Rust compatibility claim in these results.
