# Official specification snapshots

The JSON files are structural YAML-to-JSON conversions of the published Predictions specifications downloaded during this task on **October 20, 2026 (America/New_York)**. They are upstream API descriptions; SDK implementation and generator code are independently authored.

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

Public fixture files in `tests/fixtures/public-*.json` were captured from unauthenticated production reads on the audit date. `ws-official.json` contains the 38 server-message examples from this exact AsyncAPI snapshot. These are distinct from authenticated integration evidence.

## SHA-256

| Artifact | SHA-256 |
| --- | --- |
| Original openapi.yaml | `25f07afd9c09a7b005b1ccbfc06ad94928aa7d43d837ad3e43b0db7b7fc0283e` |
| Original asyncapi.yaml | `e5bce9e7c14d7ec65566ca05c399156474201c4639432b875498818255c82537` |
| Pinned openapi.json | `2939ba4d881bc8fbc768c3a25c544e24bf54f8a98c6bbb00475521357715db8a` |
| Pinned asyncapi.json | `7ee6beb93e5848b717e7c16168db15d8af7850e1620678ab18e60a3d599fa518` |
