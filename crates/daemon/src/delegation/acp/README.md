# Worker ACP boundary

This owner starts or loads one exact native session, selects the frozen Plan's
model and native permission mode, then sends one prompt. It does not resolve user
configuration, choose a Plan, retry a prompt, or replace a failed Continue with a
new session. See the [Worker launcher](../local_worker/README.md) for process and
temporary-material ownership.

The profile applies startup overrides, but those alone do not establish the
session's model. Native resume and user model settings can change the adapter's
effective selection. [Model selection](model.rs) therefore checks the `model`
config selector returned by both `session/new` and `session/load`. If necessary,
it sets the exact frozen alias and requires the reply to confirm it before the
journal records prompt intent. Missing, ambiguous, rejected or unconfirmed model
selection is `CapabilityUnavailable`. Cancellation and deadline remain their own
errors. No approximate alias resolution or legacy RPC fallback is permitted.

The supported wire behavior was inspected in `codex-acp` 1.1.5 and
`claude-agent-acp` 0.60.0. Both expose `configOptions` with `id: "model"` and
support `session/set_config_option`. Codex reports its base model separately from
the reasoning-effort selector. Claude can catch a failed resume model override
and return the prior live model; the check here prevents prompting that session
under a different Plan. These observations justify the capability check, not a
version allowlist. This owner does not infer effort semantics from model IDs.

The Rust ACP dependency provides typed config options and a typed setter; its
current v1 schema does not retain legacy `models` response fields. A legacy
`currentModelId` claim cannot substitute for confirming the current selector.

[Model regressions](model_tests.rs) exercise new and continued tasks, an already
correct selection, a corrected selection, and each failure before prompt intent.
They prove protocol behavior, not that a real installed Agent loaded user skills
or sent the selected model to Gateway. The real Worker product journey must prove
those effects through its actual adapter and native client.

Pi's bundled bridge has a closed `hiroute.pi-worker-failure/v1` error payload.
[Failure ingestion](pi_failure.rs) recognizes only the exact schema, error code
and allowed stage values, and preserves existing public error categories. Unknown
or extended payloads remain protocol failures; raw adapter messages are not copied.
[Protocol tests](tests.rs) verify startup/load errors precede prompt intent and
model errors fail the accepted run without replay. The bridge's process tests live
beside its producer under `profile/pi_worker_bridge.test.mjs`.
