# Worker ACP boundary

This owner starts or loads one exact native session, selects the frozen Plan's
model and native permission mode, then sends one prompt. It does not resolve user
configuration, choose a Plan, retry a prompt, or replace a failed Continue with a
new session. See the [Worker launcher](../local_worker/README.md) for process and
temporary-material ownership.

[The stdio transport](transport.rs) accepts incoming ACP frames up to 30 MiB,
including image tool updates from adapters that send both raw and rendered
output. Outgoing ACP frames and accumulated result text each have their own
1 MiB limit. An inbound size rejection emits `ACP_FRAME_TOO_LARGE` through the
Worker progress stream with the byte limit. The existing connection-loss path
records the outcome and stops the owned process.

The profile applies startup overrides, but those alone do not establish the
session's model. Native resume and user model settings can change the adapter's
effective selection. [Model selection](model.rs) therefore checks the `model`
config selector returned by both `session/new` and `session/load`. If necessary,
it sets the exact frozen alias and requires the reply to confirm it before the
journal records prompt intent. Missing, ambiguous, rejected or unconfirmed model
selection is `CapabilityUnavailable`. Cancellation and deadline remain their own
errors. No approximate alias resolution or legacy RPC fallback is permitted.

The model-selection wire behavior was inspected in `codex-acp` 1.1.5 and
`claude-agent-acp` 0.60.0. Both expose `configOptions` with `id: "model"` and
support `session/set_config_option`. Codex reports its base model separately from
the reasoning-effort selector. Claude can catch a failed resume model override
and return the prior live model; the check here prevents prompting that session
under a different Plan. These observations justify the capability check, not a
version allowlist. This owner does not infer effort semantics from model IDs.

Codex adapters must also advertise the negotiated AIR v1 `sessionFailure`
extension in their initialize response before a session or prompt is sent.
`codex-acp` 1.1.5 can report a rejected model request as ordinary assistant text
followed by `end_turn`; it cannot establish task success. The official 2.1.1
adapter advertises terminal failure metadata. Missing capability fails closed;
error-looking assistant prose is never parsed as a substitute terminal signal.
The 1.1.5 model-selection observation above does not establish Worker compatibility.
On a missing capability, this owner writes a fixed `[HiRoute]` explanation to the
existing bounded progress capture, naming `codex-acp`, the capability and the
update/reselect action. The lifecycle flushes it after owned-process cleanup, so
ordinary Worker progress reads can retrieve it after reopen. It is not a model
result and contains no adapter-supplied text. The public error remains
`CapabilityUnavailable`; no version allowlist or dependency preflight is added.

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

A successful Claude terminal response can precede the exact native transcript's
last append. The [lifecycle terminal checkpoint](../lifecycle/terminal.rs) publishes
the result, retains its finalization lease and revokes the run credential before
observing that transcript on the still-live ACP transport. Terminal permission
and text admission are already closed. The original bounded history deadline
controls Continue eligibility; interruption cannot replace a published result or
claim that history settled. Only the outer lifecycle stops the owned process.
[Causal regressions](../lifecycle/tests/terminal.rs) cover EOF, cancellation,
disconnect and publication failure. Real installed Claude tool and Continue
acceptance remains a separate required native gate.
