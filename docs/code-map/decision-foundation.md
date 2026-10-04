# Foundation for decision providers and consumers

[Architecture](architecture.md) · [Gateway map](../../crates/gateway/README.md)

This is an engineering boundary note, researched on 2026-10-03. Built-in Jev,
configurable System One providers, natural-language routing branches and tool-set
selection are future features, not capabilities introduced by this cleanup.

## Verified provider differences

System One describes a class of structured decision models; Jev is TypeSafe's
model. Keep the model, serving provider and decision purpose distinct. The common
shape is a state and typed questions, rather than a chat completion. See
[TypeSafe's concepts](https://docs.typesafe.ai/concepts/system-one).

| Service | Documented evaluation endpoint and model example | Integration implication |
| --- | --- | --- |
| TypeSafe | `https://api.typesafe.ai/v1/systemone`, `jev-latest` | Bearer authentication; `model`, `state`, `questions`; typed answers and input/output usage. [API reference](https://docs.typesafe.ai/api) |
| OpenRouter | `https://openrouter.ai/api/alpha/decisions`, `typesafe/jev-1.13` | Dedicated Decisions API; model IDs and cost-bearing usage belong to this provider. [Official tutorial](https://openrouter.ai/blog/tutorials/how-to-use-jev/) |
| Alibaba Cloud Model Studio | `https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/systemone`, `decision-model-preview`; Singapore has a different regional host | Workspace and region participate in endpoint identity; the documented response includes `request_id`, `usage.input_tokens` and `latency_ms`. [API reference](https://help.aliyun.com/zh/model-studio/decision-model-api) |
| System One hosted service | `https://system-one.dev/v1/systemone`, `jev-latest` | Its advertised request limits and credit accounting differ from token billing; hosted availability is distinct from SDK adapters. [API](https://system-one.dev/en/api), [service scope](https://system-one.dev/en/about) |

The references describe Choice, Score and Noul primitives. They do not establish
identical predictions, calibration, limits or failure semantics. The Model Studio
API page currently gives conflicting Score limits (2–10 in parameter details,
2–255 in its limits section); resolve that before claiming broad compatibility.
No authenticated provider call was made for this research. Treat documentation
examples as contract leads, not frozen live responses or latency evidence.

## Responsibilities to keep separate

| Responsibility | Current owner / next extension seam | Must not absorb |
| --- | --- | --- |
| Configuration and credentials | Existing Application/control and secret-management paths; future provider kind, endpoint and model selection | Credentials in WebView DTOs, prompts or fixture snapshots |
| Provider transport | Current Jev service shell; future adapter owns path construction, auth, wire encoding, limits, errors and usage normalization | Smart-saving thresholds, tool retention policy, execution permission |
| Typed evaluation | Pure input preparation and Choice/Score validation in the Jev reference implementation; future reusable evaluation contract | A mandatory model-route or HTTP-server lifecycle dependency for every consumer |
| Model routing policy | Gateway classification plus the current smart-saving preset | Tool-set fallback or tool execution |
| Tool-set policy | Future consumer of evaluation at existing ContextHold boundaries | A second session state machine or a new permission system |
| Effect admission and execution | Gateway planning/runtime and existing client permissions | Authority granted merely because a model returned a name or probability |

Future provider configuration should explicitly identify protocol/adapter,
endpoint, credential reference and model. Do not overload the business model pool
or copy a provider switch into each decision consumer. Preserve missing usage as
unknown rather than zero; preserve provider identity and resolved model in
evidence. Capabilities and limits should come from the verified provider contract.
These are design inputs, not new DTOs or a plugin framework in this batch.

Existing Jev smart-saving questions and threshold behavior remain a named policy.
Extracting pure logic does not make that policy appropriate for every use case.
A model-route answer selects one allowed branch; tool filtering may retain several
capability groups. Do not force a tool set into a single route's result type.

The current five-field HiRoute `/v1/decisions` contract is specifically a branch
decision envelope. It is distinct from the provider's `state/questions` protocol.
Keep that public envelope stable during cleanup; future tool selection needs its
own purpose-specific input and result validation over a shared evaluation seam.
Do not smuggle tool catalogs into branch descriptions or expose raw system prompts,
credentials and tool arguments by treating every internal context as model state.

## Tool selection context

Future tool selection needs the following engineering constraints established
before implementation:

- Reuse allowed ContextHold decision boundaries; keep the selected tool set stable
  during the execution segment. Inspect [ContextHold](../../crates/gateway/src/context_hold)
  and [classification](../../crates/gateway/src/core_runtime/classification.rs).
- Select only from tools currently declared by the client, preferably using
  namespace/capability summaries. Filter definitions without deleting historical
  calls/results or rewriting their IDs. Existing protocol adapters remain owners
  of valid wire projection.
- Uncertain, failed or invalid selection retains the original tool set. This is
  a tool-selection policy, not the model-routing fallback policy.
- Resolve explicit `tool_choice`, delayed discovery, changing catalogs, retries,
  model switches and restart reuse before implementation. A hidden capability
  cannot be assumed discoverable by the model later.
- Selection does not expand authorization or execute tools. Do not introduce a
  new persistence or plugin system merely to cache a selection.

A token reduction alone does not demonstrate task success. Keep stochastic
quality evidence separate from deterministic protocol and ownership regressions.

## Durable tests for the future feature tasks

| Suite | Stable product assertion |
| --- | --- |
| Provider contract fixtures | Same question intent can be encoded for each supported provider; normalize answers, errors and usage without fabricating missing fields; reject malformed and foreign options |
| Routing policy | Allowed branch selection, thresholds, fallback, deadline/cancel behavior and previous-stage assessment attribution remain intact |
| Tool selection policy | Retained set is a subset of the current catalog; mandatory/explicitly selected tools and protocol constraints are honored; invalid/failed decisions preserve the original set |
| Continuation and projection | Selection remains stable inside ContextHold; allowed boundaries re-evaluate; history, tool IDs and results remain unchanged through the real listener |
| Live quality evaluation | Paired repeated tasks with fixed model/Agent/tool versions; compare completion, missed tools, total latency/cost and cache effects, including decision overhead |

Keep deterministic contract tests separate from stochastic model evaluation. First
validate tool selection independently of model routing, then measure their
combination. Future tasks should define outcomes and independent expected values
before adding new provider adapters or decision consumers.
