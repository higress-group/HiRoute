# Custom decision service for smart saving

[Simplified Chinese](smart-saving-model-classification.zh-CN.md)

Smart saving uses HiRoute's deterministic built-in rules by default. To use a custom policy,
select “Custom decision service” in the plan editor and enter its address. Authentication may
be omitted or configured with a header name and a newly created or existing Secret. The
service may use Jev, an LLM, or another custom policy. HiRoute consumes only the HTTP JSON
contract below and never calls or parses a provider-specific protocol directly.

The official deployable reference is
[`decision-extensions/extensions/jev-decider`](../decision-extensions/extensions/jev-decider/README.md).
It demonstrates one OpenRouter Jev Decisions call that both selects the current branch and,
when applicable, scores the previous execution stage. It is also a starting point for custom
policies.

Desktop's “View integration protocol” dialog provides copyable `curl` request/response
examples and downloads the single current
[OpenAPI 3.1 document](../decision-extensions/api/decision.openapi.json). Its endpoint is an
example only; runtime always calls the complete endpoint configured in the AgentPlan.

## Configuration

```json
{
  "kind": "rest",
  "endpoint": "http://127.0.0.1:8080/v1/decisions",
  "timeout_ms": 3000,
  "auth_header": {
    "name": "Authorization",
    "value_secret_ref": "secret/jev-decider"
  }
}
```

- `endpoint` is a complete HTTP/HTTPS URL trusted by the operator. Userinfo and fragments are
  rejected, and redirects are not followed.
- `timeout_ms` is the millisecond deadline for the whole decision path. Its valid range is
  `1..=3_600_000`; Desktop starts at `3000`. The source request deadline can shorten the
  effective deadline.
- `auth_header` is optional. Its Secret stores the complete header value, such as `Bearer ...`;
  HiRoute does not add a scheme.
- A plan stores no plaintext Secret and configures no classifier instructions, plan purpose,
  or editable branch descriptions.
- “Test decision” explicitly sends a fixed synthetic first-turn request through the same
  Secret, HTTP, timeout, and response validation as production. Save and publish never call
  the service automatically. A test may incur an external charge and never creates a quality
  sample.
- The external service's own total timeout should be slightly shorter than `timeout_ms` so
  HiRoute can serialize and close the network operation. The official Jev decider uses
  `JEV_REQUEST_TIMEOUT_SECONDS`, defaulting to `2.8` seconds.

## Request contract

Each new Agent turn sends at most one `POST`. Tool continuations within that turn inherit the
frozen branch and do not call the decision service again.

```json
{
  "branches": {
    "smart_saving_simple": "Use the economy model group for a clear, well-scoped task.",
    "smart_saving_complex": "Use the primary model group for an ambiguous, cross-module, diagnostic, concurrent, or deep-reasoning task."
  },
  "latest_user": [{"kind": "text", "text": "Fix this failing test."}],
  "visible_conversation": [{
    "branch_id": "smart_saving_simple",
    "user": [{"kind": "text", "text": "Fix the type error first."}],
    "status": "completed",
    "steps": [
      [{"kind": "tool_activity", "tool": "functions.run_tests", "status": "failed"}],
      [{"kind": "text", "text": "Fixed and verified again."}]
    ]
  }],
  "history_partial": false,
  "assessment_from": 0
}
```

The top level has exactly five fields:

- `branches`: allowed branch IDs and HiRoute's built-in descriptions. The service must return
  one of these IDs.
- `latest_user`: complete content parts for this turn. A service may choose to inspect only
  this field.
- `visible_conversation`: completed Agent turns held in memory. Each step is one business-model
  request and retains only accepted response text plus tool name, order, and coarse status.
- `history_partial`: true when restart, TTL, LRU eviction, or a capture gap made history
  incomplete.
- `assessment_from`: the start index of the previous contiguous execution stage to assess;
  null means that no reliable assessment target exists.

Tool status comes only from explicit ingress-protocol facts. Messages uses `is_error`;
Responses function output uses `completed/incomplete/in_progress`; provider-native web search
uses its explicit terminal state. Chat tool results, Responses custom output, and Responses
function output without a status are `unknown`. HiRoute never parses tool-output prose to
guess failure. A Chat Agent can still tell the business model about an error in result text,
but classification history does not promote that free-form text to a structured failure.

The protocol excludes system/developer text, reasoning, tool arguments and results, provider
state, credentials, plan purpose, plan/model/session internal IDs, and per-step models. If only
the actual fallback branch produced accepted output, a turn also includes
`executed_branch_id`; mixed-model contribution is not a single-model assessment target.

HiRoute imposes no extra byte limit on a decision request and does not truncate `latest_user`
or text blocks. Fields above 8 KiB are streamed from the request's ReplayStore; 8 KiB is a
storage-location threshold, not a REST protocol limit. A decision service with a 32K-token
model limit must use its own tokenizer and policy to trim complete turns and return `partial`
accurately.

## Response contract

Minimal success:

```json
{"branch_id":"smart_saving_complex"}
```

With previous-stage competence:

```json
{
  "branch_id": "smart_saving_complex",
  "assessment": {
    "score": 0.25,
    "partial": false,
    "reason": "Optional explanation of visible behavior"
  }
}
```

- `branch_id` is required and must be present in request `branches`.
- `assessment` is optional. Omission means “do not update the score,” not zero.
- `score` is model competence in `[0,1]`, not confidence, success probability, or task
  complexity.
- `partial` is required within an assessment and states whether the service reduced the
  assessed interval.
- `reason` is optional. A Jev implementation should not invent text when Jev supplies none.

With a valid branch and invalid assessment, HiRoute uses the branch and drops the score. An
invalid branch invalidates the complete response and stores no score. A successful body is at
most 64 KiB. Unknown fields, duplicate fields, multiple objects, Markdown, and provider
envelopes are rejected.

## Stage scoring and queries

HiRoute identifies an execution stage by contiguous plan revision, selected/actual branch,
actual model configuration, and effective profile. Credential rotation does not split a
stage. A real change of model, profile, branch, or plan revision starts a new stage after
execution. A service may return a score every turn or omit it. A valid new score replaces the
latest score for that stage rather than creating a per-turn series.

Desktop's session “Model performance” and plan-editor “Runtime performance” read the same
data. The plan page lists models selected by the current effective revision and shows their
stage competence over the selected time range, without asking for an internal model ID. CLI
queries can still filter by plan or session and combine revision, exact model, time, strict
greater-than, and strict less-than filters, for example:

```sh
hiroute observation plan-quality samples \
  --plan-id plan/codex-daily \
  --model model/config-a \
  --score-lt 0.5
```

An unrated sample is not zero, and `score_lt 0.5` excludes exactly 0.5. A human or authorized
main Agent can use scores to judge whether a model is competent under a specific AgentPlan and
whether a narrower scenario would help. HiRoute never modifies or creates a plan automatically.

## Failure boundary

- History preparation, Secret resolution, DNS/connect, and HTTP I/O share `timeout_ms` and
  remain bounded by the source request deadline and cancellation.
- A turn calls the service at most once, with no retry and no trim-and-retry path.
- External timeout, unavailability, input rejection, or invalid output uses the local rules
  once and records a structured reason.
- Source cancellation, source deadline, Replay integrity failure, or local resource failure
  terminates directly; it cannot masquerade as a REST failure followed by fallback.
- Observation-write failure does not block the model answer. Only a persisted score appears in
  queries.
