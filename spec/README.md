# Official specification snapshots

The JSON files began as structural YAML-to-JSON conversions of the published Predictions specifications downloaded during this task on **October 20, 2026 (America/New_York)**. `asyncapi.json` also includes the targeted orderbook clock update described below. They are upstream API descriptions; SDK implementation and generator code are independently authored.

- `openapi.json`: [official openapi.yaml](https://docs.kalshi.com/openapi.yaml), OpenAPI 3.0.0, API info version 3.34.0, 118 operations.
- `asyncapi.json`: [official asyncapi.yaml](https://docs.kalshi.com/asyncapi.yaml), AsyncAPI 3.0.0, API info version 2.0.0.
- [Official documentation index](https://docs.kalshi.com/llms.txt).
- [Official changelog](https://docs.kalshi.com/changelog).

The REST spec's HTTP Last-Modified header was October 19, 2026. The retrieved documentation also contains entries dated October 22; future-dated changelog entries are announcements, not evidence that every change was already deployed at the audit date.

Regenerate from the checked-in files without network access:

```sh
python3 tools/generate.py
cargo fmt
```

For an upstream update, download the authoritative YAML files, convert them to JSON without lossy floating-point conversion, review their changes, update the hashes and provenance, regenerate, and run the full test suite. Review parameter styles, authentication flags and schema constructs as well as endpoint counts. The generator rejects schema types it does not understand; extend it deliberately for new constructs. Generated models are checked in, so consumers need neither Python nor YAML tooling.

Public fixture files in `tests/fixtures/public-*.json` were captured from unauthenticated production reads on the audit date. `ws-official.json` contains the 38 server-message examples from the original AsyncAPI snapshot, with the two orderbook examples updated alongside the clock schemas. These are distinct from authenticated integration evidence.

## Orderbook clock update

On **October 27, 2026 (America/Chicago)**, the [official asyncapi.yaml](https://docs.kalshi.com/asyncapi.yaml) was retrieved again, still declaring AsyncAPI 3.0.0 and API info version 2.0.0. Only its orderbook channel description, `msg.clock` schema properties and required-field lists, and the two corresponding message examples were applied to the pinned JSON and websocket fixtures. Other schemas remain at the original snapshot; this is not a full refresh of the downloaded document.

The upstream schema requires an `int64` exchange clock in both orderbook message bodies. Snapshots permit zero for a market with no recorded change; deltas describe the action that made the change and have a minimum of one. These bounds remain schema metadata, consistent with the generator's existing handling of field ranges. The channel description records matching across connections, repeated clocks, gaps, and maintenance resets. See the [orderbook update documentation](https://docs.kalshi.com/websockets/orderbook-updates).

Before the release, an authenticated example run reported an orderbook message without `clock` after successful subscription acknowledgments, so a temporary generator workaround exposed these fields as `Option<i64>`. On **October 31, 2026 (America/Chicago)**, that workaround was removed following the October 29 release recorded in [Kalshi's changelog](https://docs.kalshi.com/changelog). The existing pinned schema now generates required `clock: i64` fields directly. Missing or null clocks produce decoding errors; valid clocks remain exact integers. Offline tests exercise both receive APIs rejecting invalid messages and then decoding valid messages on the same connection. These checks do not establish authenticated live verification from this workspace.

The source YAML hash for this targeted update is recorded separately below. The original YAML and JSON hashes are retained for provenance. JSON hashes use UTF-8 with LF line endings, matching Git's stored files.

## SHA-256

| Artifact | SHA-256 |
| --- | --- |
| Original openapi.yaml | `25f07afd9c09a7b005b1ccbfc06ad94928aa7d43d837ad3e43b0db7b7fc0283e` |
| Original asyncapi.yaml | `e5bce9e7c14d7ec65566ca05c399156474201c4639432b875498818255c82537` |
| Pinned openapi.json | `2939ba4d881bc8fbc768c3a25c544e24bf54f8a98c6bbb00475521357715db8a` |
| Original pinned asyncapi.json (October 20) | `7ee6beb93e5848b717e7c16168db15d8af7850e1620678ab18e60a3d599fa518` |
| Upstream asyncapi.yaml for clock update (October 27) | `b7d1b9574c98ec679ab81ce61612d1cccd0405ad57912df60cf462b0effac26e` |
| Pinned asyncapi.json with clock update | `5cf52fc64f81f412405015b93374e7ff68fcbb54dea757979bc9e7e8392d270c` |
