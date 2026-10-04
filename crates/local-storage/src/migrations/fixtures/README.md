# Stable schema 23 Worker journal fixture

`schema23-worker-operations.json` is frozen output from the original schema 23
producer at `cdbb2cec` (product code identical to `c509481d`). The one-shot exporter
is commit `5d8306cc`, based directly on that producer. It was run with:

```text
cargo test --locked -p hiroute-local-storage --lib export_schema23_worker_operations -- --ignored --nocapture
```

Managed run: `3724c06e070e4d00bd51bab284c8ab49`, green. Exact exporter:
`5d8306cc2f9ce5e95d79657e4d406cfd9e1ca062`. Fixture SHA-256:
`1886b84980aab4ddb7b3cd9ed21da0a654b13474e448f02883a233dc312d043f`.

The exporter opened the production `LocalStorageSet`, admitted real Worker
selection Operations and saved their effects and step journals through the old
storage API. It exported the exact SQL row values for Codex and Claude at staged,
activated and succeeded checkpoints, with a prior successful selection. It did
not export keys, authentication material or user data. Timestamps are original
synthetic-run values and are deliberately not normalized.

The current test reconstructs the published v20 Worker tables and schema 23
version/marker, inserts these frozen rows verbatim, then enters production
startup. Fresh keys and store identities are generated per test. This isolates
the actual historical contract from the current serializer while keeping the
fixture independent of machines and credentials.

Read `worker_dependencies_v24_tests.rs` for the user-visible invariants: old
Operations remain replayable, activation does not increment a revision twice,
compensation respects the current owner, interrupted upgrades require the exact
backup set, and completed backups survive the next upgrade batch. The ordinary
current-format selection test also covers the Qoder single-CLI form.

Do not regenerate this fixture from the new producer to make a failing migration
pass. Its removal requires an explicit end to schema 23 upgrade support, retaining
a documented supported recovery path.
