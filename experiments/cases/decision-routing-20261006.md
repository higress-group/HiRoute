# Decision-routing experiments — execution contract

Protocol date: 2026-10-06. Status: prepared, no formal outcomes yet. The commit
containing this protocol must be recorded before the first paid formal run.
Related cases: [usage reconciliation](usage-reconciliation/README.md) and
[writer/reviewer](writer-reviewer/README.md). These are new experiments, not reruns
of the research-card or HTTPX studies.

## Common controls

- Use the real ordinary Debug Desktop, daemon and native Codex client. All model
  requests go through a published HiRoute plan. Configure sources, profiles,
  plans and client connections through Desktop; CLI/DB reads only corroborate.
  Never use the Pilot with real credentials. Do not change daily user settings.
- Business models use the user's authorized Bailian Token Plan connection. Record
  exact source option, endpoint, actual model ID, reasoning settings, native engine
  version/hash, application source SHA/build, plan revision and decision revision.
  A newer docs commit is not the source SHA of an older running application.
- Freeze one OpenRouter Jev decision-service revision for all routed groups,
  using the already accepted `typesafe/jev-1.13` endpoint. The provider is a
  control, not a factor in these experiments. Preserve its configured timeout.
- Per-model reasoning profiles apply at the gateway; a client's visible alias or
  effort selector alone does not prove the upstream setting. Verify actual
  request parameters before formal runs. Do not silently substitute model IDs.
- Use a fresh project, HOME, native conversation and output directory for each
  replicate. Continue the same conversation across that replicate's stages.
  Record context resets/compactions; compare the affected pair with that limit.
- Subject files are an explicit allowlist. Reuse the repository's inherited Mac
  OS read boundary (`.agents/experiments/read_isolation.py` and
  `lifecycle_isolation.py`). Graders, controller records, other arms, credentials,
  human reference, and reference implementations stay outside its readable roots.
  Probe real files and symlink access. No unrestricted fallback. Subject outbound
  access is limited to the local HiRoute listener; disable native web search/MCP.
  Test the network restriction before use, including from a child process.
- A subject receives only its current task/materials and its own earlier outputs.
  No group name, intended winning model, hidden tests or other model's results.
  The executor cannot coach a weak group. Shared neutral instructions and fixed
  stage prompts are identical across comparison groups.
- Cache controls: finish setup first; run the same small non-scored warm-up through
  each unique plan. Do not warm a formal prompt. Keep cached and uncached tokens
  separate when reported. No simultaneous formal runs or background probe loops.

## Effort settings and preflight

The [official Responses API documentation](https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-responses)
lists Qwen 3.8 native levels `low`, `medium`, `xhigh` (plus `none`), while GLM 5.3
uses `low`, `high`, `max`. Thus the writing experiment's middle band uses
Qwen `medium` / GLM `high`; its highest band uses Qwen `xhigh` / GLM `max`.
This aligns ordinal settings, not compute budgets. Preserve native values in
evidence rather than implying that the two models have identical effort scales.

Preflight is limited to sixteen non-scored native turns total: confirm each model,
profile and plan can complete a small local file task; writing plans must show
both roles. Reuse a warm-up as the matching preflight. Test inputs differ from the
formal task. If sixteen turns do not establish readiness, report the blocker instead
of consuming the formal experiment as debugging. Freeze configuration hashes and
the preflight count before proceeding.

## Bounds, failures and repairs

- Formal work: 12 writing pipelines × 4 turns, plus 6 audit sessions × 6 turns.
  No quality-driven retries, extra editorial round or best-of selection.
- One transport-only retry of an identical stage is allowed when no successful
  completion was received (429/5xx/disconnect). Retain both attempts, their time,
  cost and any partial artifacts. Never retry a valid but poor answer.
- Native turn wall limit: 12 minutes. Each replicate: 50 minutes; the writing
  pipeline also has a 40-minute limit. Terminate the process group on expiration;
  save a partial result. At a completed-turn boundary, stop a replicate if its
  measured total exceeds 500,000 tokens. This is a boundary stop, not a hard API
  token cap; any overshoot remains included. Unknown usage never becomes zero.
- Executor session: at most 8 hours, including setup. Stop and report completed
  and unexecuted cells if the bound is reached. No unbounded probes or polling.
- On a HiRoute product defect, stop the affected run, preserve its reproduction
  and proactively return to the parent. Do not patch product code or work around
  routing by calling providers directly. Parent repairs/redeploys, then dispatches
  the next round. Re-run the affected matched block on the fixed version, with
  fresh projects. Preserve the original block and costs as diagnostic evidence.
- Provider semantic misrouting, weak writing and incorrect subject code are
  experimental outcomes, not permission to tune prompts/thresholds after seeing
  results. Environment or harness failures are distinct and need evidence.

## Evidence and accounting

For every stage, retain prompt/material hashes, native session/request IDs,
completion state, actual branch/model/profile, all attempts, tool pairing, saved
artifact hashes, latency, input/cache/output/reasoning tokens and decision usage
when available. Missing components stay `null` with a reason. For resumed native
sessions compute deltas from cumulative counters; empty usage arrays are unknown.
Do not double count native counters and gateway records.

Report whole-delivery gates first, then stage correctness/editorial scores,
latency and tokens. Group totals include failures and retries. For matched
successful pairs, also report their comparison separately; never silently drop
unsuccessful pairs. A two-repeat, one-task study is descriptive, not a population
estimate or proof of general model superiority.

Token Plan subscription consumption is not a cash invoice. Default money result
is unknown. A secondary API-equivalent estimate is allowed only with dated
official prices, units, cache treatment and explicit formulas frozen before
formal runs. Do not infer zero cost from metadata/catalog zero price hints or
reuse rates for a different Alibaba product. Without defensible rates, make
token/latency claims only; do not invent a savings percentage in yuan/dollars.

Use Desktop to inspect the plan's running performance and original session
history for at least one complete replicate of every plan. Match displayed
branch, reasoning profile, feedback status, score and request link to actual
records. Router competence scores are not the independent experiment grade.
Unscored/partial and missing usage remain visible. Record screenshots and a
compact discrepancy table, not just a successful process exit.

Keep raw traces in a task-owned local evidence directory outside the repository,
mode 0700. Only allowlisted summaries, public task inputs, redacted provenance,
checksums and reproduced results may be committed. No provider keys, account
state, request auth, private reasoning traces or original article full text.
Publishing a Release asset, website or promotional article is a later action.

## Report and handoff

The executor actively reports completion or a blocking product issue to the
parent and includes exact paths plus unexecuted cells. The parent waits without
polling, repairs product issues, and personally performs the writing assessment.
Before interpretation, freeze a blind bundle containing code-named draft/final
pairs without model/effort/replicate labels. Keep its mapping separate until the
parent has recorded scores. Do not add a model-judge consensus to disguise this
single evaluator limitation. Any later publicity uses all registered outcomes,
including results unfavorable to routing.
