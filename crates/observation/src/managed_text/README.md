# Managed-text internal interface

[简体中文](README.zh-CN.md)

The entry points are the types in `hiroute_observation::managed_text` and the
`LocalObservationStore::managed_text_*` methods. They are not public RPCs and do not grant
permissions. Application/delegation services must construct `ManagedTextScope` from an
authenticated task/run and check content permission on every read. Permission cannot be
derived from a Worker payload, environment variable, or reference alone.

## Writes and retries

1. The delegation service first creates a trusted task/run, then calls `managed_text_put(input, now_ms)`.
2. `source_event_id + source_revision + scope` identifies one event. A repeated call returns
   the same reference. Changing purpose, original time, or import source causes `Conflict`
   and does not refresh the seven-day deadline.
3. `append` writes zero-based contiguous chunks of 1–65,536 bytes. The same bytes for the
   same chunk are retryable; different bytes conflict. A pending chunk is registered before
   it is written and is published to readers only after the complete write succeeds.
4. `finish` commits the exact number of completed chunks and returns a Complete reference;
   the delegation service then stores the business reference. There is no distributed transaction across
   stores. A failed business commit reuses the original reference, while a content failure
   leaves the delegation service responsible for preserving the real task result and missing-content state.

`original_created_at_ms` and `now_ms` come from trusted production events and the server
clock. A history import must provide the still-visible original reference with the same
original deadline. Without provable provenance, a caller must not present old content as a
new event. Continuing an old run should reuse its content reference and scope, with explicit
authorization for cross-run access at the upper layer; this interface does not relabel the
reference with a new run identity.

## Reads and continuation

`resolve` always reads current persistent visibility and ignores cached state/generation in
the reference. `read` returns at most 16 chunks (1 MiB) and a `next_chunk`, then verifies the
generation again after file I/O. After Stale, resolve again. Deleted, Expired, or Unavailable
must not fall back to a local cache. Complete means publication finished, not that storage can
never be damaged; consuming the body still requires a successful read. Storage errors are
not empty content and cannot prove that history is resumable.

## Deletion, expiry, and native caches

Application creates a preview under a separate management authorization and calls apply only
after precise confirmation. A stale preview cutoff/count/generation returns Stale, with zero
deletion before confirmation. The apply transaction first removes visibility and records
persistent cleanup work; `gc` then removes at most 200 chunks per batch. Late events at or
before the cutoff do not restore content. Proven events after the cutoff may be written.

The delegation service uses `pending_native_cleanup(scope, after_generation, limit)` to recover unconfirmed
native-cache cleanup. It handles only HiRoute-managed history in the same scope and at or
before the cutoff, and calls `native_gc_ack` only after actual success. Notifications may be
lost; active queries and continuation must still check the current reference. An apply retry
returns both current GC states and cannot claim full cleanup while either is pending.
`expire_scope` is the retention-worker entry point, using each event's seven-day deadline and
the same native-cleanup protocol.

Production maintenance schedules expired-scope discovery and batched object GC.
Application authorization and session deletion are wired through
[`ObservationControl`](../../../application/src/observation_query/control/v2.rs)
and [unified retention](../store/unified_retention.rs). Worker native cleanup is
owned by the daemon's [maintenance consumer](../../../daemon/src/control/runtime/delegation_maintenance.rs),
which claims pending work and acknowledges completion after actual success. It
delegates bounded filesystem deletion to [native_cleanup.rs](../../../daemon/src/delegation/native_cleanup.rs).
These paths are distinct from this storage interface's tests and the
[observation fixture checks](../../../../tools/product-e2e/tests/local_observation.rs).
Inspect the [installed CLI management journey](../../../daemon/tests/publication_process.rs) and
[Worker read/continue journey](../../../../tools/product-e2e/tests/worker_read.rs)
for production-entry evidence and its limits.
