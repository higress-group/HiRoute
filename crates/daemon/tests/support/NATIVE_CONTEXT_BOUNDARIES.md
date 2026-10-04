# Borrowed native context: boundary acceptance

`native_context_boundaries.py` complements the two core cases in
`native_context_product.py`. It reuses the same normal API source, Plan,
installation selection, public Worker CLI, isolated native roots and synthetic
Responses/Messages server. It does not build binaries or replace ACP. Run it
only with the exact candidate binaries and explicitly selected real installations:

```sh
python3 crates/daemon/tests/support/native_context_boundaries.py \
  /absolute/candidate/checkout FULL_CANDIDATE_SHA --harness codex
# Repeat with --harness claude and that harness's selected installation.
```

Use the managed product runner to prepare the candidate binaries and environment.
The direct entry consumes `HIROUTE_VALIDATION_PRODUCT_BIN_DIR` and the existing
`HIROUTE_WORKER_*` installation variables; it never discovers a daily account.
Its JSON report has required case IDs, per-case outcomes, candidate and binary
hashes. Process success alone is not a product verdict.

| Stable capability | Smallest useful evidence |
| --- | --- |
| `worker.context.native-skills` | Core real Harness case: user and project skill discovery, content and actual shell receipts; conflicting settings and neighboring files unchanged. |
| `worker.context.concurrent-routing` | Boundary real Harness case: two tasks concurrently run native shell children in one HOME/configuration/workspace, with distinct Plan, source endpoint, source credential, upstream model and native session. Each source rejects the other task's prompt marker. |
| `worker.context.run-authority` | Existing daemon journal tests: `persistent_journal_streams_result_before_revoking_the_exact_run_credential`, `stop_evidence_reconciles_the_exact_lease_after_formal_cancel_advances_revision`, and `interleaved_runs_are_reconstructable_from_their_own_tokens`. They preserve exact lease/token assertions that model success cannot establish. |
| `worker.context.exact-continue` | Core immediate/restarted Continue retains native ID and prior tool history after publication of a different source/model; replay sends no request. Boundary removes only the fixture's exact transcript: public Continue must reject, or fail native load without a result, model request, new history or changed neighboring history. |
| `worker.context.cancel-owned-work` | Boundary formal cancel: target native tool PID disappears and heartbeat stops while the neighbor PID/heartbeat survives and its task succeeds. Keep `local_worker_platform` process-group/root-exit tests for precise OS cleanup failure branches. |
| `worker.context.retention-ownership` | Daemon consumer tests `deleted_worker_body_revokes_continue_and_cleans_only_owned_metadata` and `expired_worker_body_revokes_continue_and_cleans_only_owned_metadata`: managed-text visibility, actual Continue refusal and maintenance remove only owned metadata while preserving borrowed roots, native history and the neighboring task. This is consumer evidence, not a public delete API or real-Harness deletion verdict. |

The boundary case observes PIDs from its own synthetic scripts but never signals
those PIDs. Cleanup releases only those scripts and delegates Worker shutdown to
the product. The missing-history case restores its exact original transcript;
if a regression creates a replacement, it refuses to overwrite it and retains
both files for diagnosis. No caller-owned history is removed.

`python3 scripts/test-native-context-boundaries.py` checks the oracle, rejection
branches and two test-owned shell processes. It starts neither HiRoute nor an
Agent and cannot satisfy the real-Harness cases. The original delegation product
lifecycle/progress tests remain useful for public reads, replay and restricted
policy; their terminal `cancelled` assertion alone cannot prove native tool or
neighbor isolation. Native macOS Desktop remains a separate real Pilot journey.

Claude setup also writes all uppercase/lowercase HTTP(S)/ALL proxy variables to
an independent loopback trap in user `settings.json`, with deliberately
nonmatching `NO_PROXY`/`no_proxy`. Real execution must succeed while
`assert_preserved(fixture)` verifies both unchanged user bytes and zero proxy
requests. The trap records only a request marker for every HTTP method, never
headers, URLs or tokens. The normal model-server lifetime owns it, so the same
`prepare_desktop` helper preserves this check across real Pilot takeover; its
`proxy_trap_events` output identifies the synthetic evidence file.
