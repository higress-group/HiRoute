# Frozen Worker catalog

`codex-worker-serial-546.json` is the exact output of
`codex_private_worker_catalog` at `546623eb4bb876e339026785817c6228718cb370`,
the preceding installed 0.5.0 development candidate. Its synthetic Plan is the
existing `codex_catalog_plan_tests::plan` fixture. The producer and inputs were
unchanged; a local test-only export emitted the bytes. SHA-256:
`e7a69da03f826198d7716ea85d649f28ee530fc32e89a24484624ee21f85ec73`.

This is retained intermediate development data, not a claimed stable-release
format. Continue may retain this exact serial projection only when the frozen
Plan omitted `limits.codex_parallel_tool_calls`. All other catalog bytes and
private-root checks remain exact. Do not regenerate the historical input using
the current producer.
