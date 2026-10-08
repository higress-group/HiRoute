# User resource settings — 2026-10-06

The user explicitly removed the experiment's token budget and requested the
default 272k context window. This supersedes the token allowance in the
[resource amendment](resource-amendment-20261006.md). There is no per-turn or
per-replicate token stop. Continue measuring all input/output, cached and
reasoning usage, preserving unknown components and previous stop records.

The parent also removes its executor-chosen aggregate replicate and session
deadlines so that waiting for harness repair or user feedback cannot silently
skip the remaining registered stages. The parent stated this execution choice
before applying it; it is not an additional requirement attributed to the user.
The user subsequently explicitly removed the twelve-minute native-turn timeout.
There is now no experiment-imposed per-turn, replicate or overall elapsed-time
stop. The executor passes an unlimited wait to the native process, with no
replacement timeout. Previously interrupted stages keep their original policy
and partial results; they do not become completed retrospectively.
The registered matrix, stage counts, fixed prompts, grading, transport-only
retry rule and prohibition on quality-driven replays remain unchanged.

Do not restart completed or interrupted native turns. A budget-stopped group
may continue only stages that have never executed, in the same native
conversation. Archive its earlier final/stop record before continuing; preserve
its original start time, all elapsed waiting and all consumption. Report results
under their actual resource policies. Earlier interrupted stages do not become
completed retrospectively. `F-1` remains an invalid primary comparison because
of its recorded harness protocol violation; keep its partial work and cost.

For the removal of the turn timeout, the writer had naturally completed `QM-2`
stage 2 and confirmed that no native request remained in flight. Only `QM-2`
stages 3 and 4 remain; both adopt unlimited waits. The audit lane is already done.
The parent owns the shared hold and resource/controller changes. No model probe,
extra editorial round or change to published plans is needed for this amendment.

The parent checked the real Desktop's smart-saving and custom-branch views:
context is **Automatic (default), 272,000 tokens**. All nine plan templates and
all 27 prepared isolated native environments have catalog `context_window` and
`max_context_window` of 272,000, with no explicit root/profile context override
or custom auto-compaction limit. The catalog retains the client's default 95%
context headroom; this is distinct from setting a smaller model window. This
setting was already in effect, so no product configuration or plan revision
changed. Preserve the private CU and catalog-check evidence.

No article quality assessment or blind mapping disclosure preceded this
amendment. Raw outputs and experimental observations remain local; later claims
must identify incomplete stages, provider fallbacks and policy changes instead
of presenting the experiment as an unchanged fully controlled benchmark.
