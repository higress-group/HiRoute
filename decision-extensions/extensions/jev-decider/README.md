# HiRoute Jev decider

[Simplified Chinese](README.zh-CN.md) · [Decision mechanism](../../README.md) · [API and OpenAPI](../../api/README.md)

This is HiRoute's deployable reference implementation of the five-field REST branch-decision protocol. It accepts a HiRoute decision request, makes exactly one OpenRouter Decisions call with `typesafe/jev-1.13`, and returns `branch_id` plus an optional assessment of the preceding execution segment.

It is a separate trusted service, not an LLM path embedded in HiRoute. HiRoute remains responsible for routing execution-round boundaries, allowed branches, execution, assessment attribution, and persistence; this service owns Jev prompts, model-context trimming, and policy.

## Strategy

Choose exactly one mode at deployment:

- `auto` asks Jev for the final branch and, when an assessable prior segment exists, its competence Score in the same upstream request. The Choice is instructed to consider both the new task and observed prior performance; only its selected branch is required, because auto does not consume Choice probabilities.
- `rules` asks for simple/complex probabilities and the optional competence Score in the same upstream request. It supports HiRoute's current two smart-saving branches only.

The rules formula is deliberately small:

```text
choose smart_saving_simple when
  P(smart_saving_simple) >= JEV_SIMPLE_THRESHOLD
  AND (no valid competence score OR score >= JEV_COMPETENCE_FLOOR)
otherwise choose smart_saving_complex
```

Defaults are `0.80` and `0.50`. They are starting values, not calibrated guarantees.

**Competence guards against risky cost-cutting; complexity identifies opportunities to save.** A good score does not make a genuinely complex new task simple; a low score can prevent an otherwise-simple task from being routed back to a branch that is currently performing poorly. A missing assessment is not zero and does not block the economy branch. A partial assessment remains useful evidence but must not be presented as complete.

## Run locally

Python 3.12+:

```sh
cd decision-extensions/extensions/jev-decider
python -m venv .venv
. .venv/bin/activate
pip install .
printf '%s' 'replace-with-key' > /absolute/path/openrouter-key
chmod 600 /absolute/path/openrouter-key
OPENROUTER_API_KEY_FILE=/absolute/path/openrouter-key hiroute-jev-decider
```

Container:

```sh
docker build -t hiroute-jev-decider decision-extensions/extensions/jev-decider
docker run --rm -p 127.0.0.1:8080:8080 \
  -v /absolute/path/openrouter-key:/run/secrets/openrouter-key:ro \
  -e OPENROUTER_API_KEY_FILE=/run/secrets/openrouter-key \
  -e JEV_MODE=auto \
  hiroute-jev-decider
```

Then configure a HiRoute smart-saving plan with:

```json
{
  "kind": "rest",
  "endpoint": "http://127.0.0.1:8080/v1/decisions",
  "timeout_ms": 3000
}
```

`GET /health` never calls OpenRouter. Saving or publishing a HiRoute plan also does not call this service.

The container build command runs from the repository root. Existing deployments must update the plan endpoint to `/v1/decisions`; this version does not retain an old-path alias.

## Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `OPENROUTER_API_KEY_FILE` | required | Absolute path to a UTF-8 file; its value is sent only as the OpenRouter Bearer credential |
| `JEV_MODE` | `auto` | Exactly `auto` or `rules` |
| `JEV_MODEL` | `typesafe/jev-1.13` | OpenRouter Decisions model |
| `OPENROUTER_DECISIONS_URL` | `https://openrouter.ai/api/alpha/decisions` | Override only for a controlled gateway/test upstream |
| `JEV_REQUEST_TIMEOUT_SECONDS` | `2.8` | Whole request budget, including validation, context preparation, queueing and the upstream call; must be in `(0, 3600]`; set it slightly below the matching HiRoute plan's `timeout_ms` and keep the default for a 3000 ms plan |
| `JEV_MAX_STATE_TOKENS` | `24000` | Conservative state budget described below |
| `JEV_MAX_CONCURRENCY` | `32` | Maximum simultaneous upstream decisions; queueing consumes the same request budget |
| `JEV_SIMPLE_THRESHOLD` | `0.80` | Rules-mode simple probability floor; rejected when explicitly set in auto mode |
| `JEV_COMPETENCE_FLOOR` | `0.50` | Rules-mode competence guard; rejected when explicitly set in auto mode |
| `DECIDER_AUTH_HEADER_NAME` / `DECIDER_AUTH_HEADER_VALUE` | unset | Optional inbound shared header; set both or neither |
| `HOST` / `PORT` | `127.0.0.1` / `8080` | Listener |

Outbound OpenRouter calls honor the deployment's standard `HTTP_PROXY`, `HTTPS_PROXY`, and `NO_PROXY` environment variables (including their lowercase forms). The service keeps one client connection pool, so later decisions can reuse an established route. Containers must receive any intended proxy variables explicitly; if none are set, the service connects directly. This is standard client routing, not a HiRoute proxy configuration layer.

If inbound authentication is enabled, store the full header value in a HiRoute Secret and configure the matching `auth_header`. For example, the Secret may contain `Bearer ...`; HiRoute does not prepend a scheme.

## Context and privacy boundary

HiRoute sends the complete, non-empty `latest_user` plus its in-memory `visible_conversation`. A decision can begin because there is no inheritable branch, a user message was appended, or ContextHold no longer has a preferred candidate; the service does not detect compaction. `latest_user` can therefore repeat a prior round or be a client-generated summary/continuation. That shape is not itself feedback. Each visible item is a sealed routing execution round, and an item sealed at a decision boundary may have `unknown` status while still containing assessable progress.

This service never asks HiRoute to truncate or retry a request. It keeps the newest whole rounds and removes only complete oldest rounds before calling Jev. The current input, branch definitions, and fixed questions are never truncated. If those fixed parts do not fit, the service returns 413. If trimming removes any part of the assessment target, a returned assessment has `partial: true`; if no target content remains, no Score question or assessment is produced.

Jev's context limit is measured in tokens, not kilobytes. To avoid a tokenizer dependency, this reference service counts each UTF-8 byte as one conservative upper-bound token for state and reserves the rest of the model window for questions and output. `24000` therefore means at most 24,000 UTF-8 bytes of state, not an assertion that 24 KB equals 24K model tokens. Deployments can lower or carefully raise this budget after measuring their prompts and chosen model.

The HiRoute protocol intentionally excludes system/developer messages, reasoning, model/provider/plan identities, tool arguments, and tool results. Tool activity contains only a name, order, and coarse status. OpenRouter receives only the resulting state and fixed Jev questions. This service does not log bodies or credentials.

## Contract and extension point

`POST /v1/decisions` accepts exactly:

```json
{
  "branches": {"smart_saving_simple": "...", "smart_saving_complex": "..."},
  "latest_user": [{"kind": "text", "text": "..."}],
  "visible_conversation": [],
  "history_partial": false,
  "assessment_from": null
}
```

It returns:

```json
{
  "branch_id": "smart_saving_simple",
  "assessment": {"score": 0.73, "partial": false}
}
```

`assessment` is optional. Its score is competence in `[0,1]`, not model confidence. Jev's three-level `0..2` Score is divided by two. Jev does not provide a textual reason here, so this implementation omits the optional `reason` field.

To implement a different strategy, keep the HTTP validation and response contract, then replace `questions()` and `decision_response()` in `jev_decider/server.py`. Do not add vendor fields to the HiRoute request or make a second scoring call: use the five protocol fields as state and return one allowed branch with an optional competence assessment.

## Offline tests

The suite starts the real HTTP handler and a controlled upstream server. It does not read a real key or access the network:

```sh
python -m unittest -v
```

The suite covers auto/rules single-call behavior, the OAS route and rejection of old routes, first-round omission, standard proxy routing, formula boundaries, invalid optional Score, invalid probabilities and branch sets, whole-round trimming/partial marking, fixed-context rejection, strict input, inbound authentication, health isolation, upstream 401/402/429, timeout, oversized output, and concurrency. A real OpenRouter smoke test is intentionally opt-in and must not be placed in normal CI or retried automatically.

## Opt-in HiRoute → Jev smoke

After the offline suite passes, start this service with a real key file as shown above. From the repository root, run exactly the ignored production-path test against that local endpoint:

```sh
HIROUTE_LIVE_CLASSIFIER_ENDPOINT=http://127.0.0.1:8080/v1/decisions \
  cargo test -p hiroute-e2e --test p0_gateway_runtime \
  live_hiroute_to_jev_smoke_selects_and_records_an_assessment \
  -- --ignored --exact --nocapture
```

The smoke performs two paid Jev requests with no retry. It drives the real HiRoute listener and REST classifier transport, uses controlled business-model providers, starts a second routing execution round, and requires a normalized competence assessment in emitted Observation facts. A local-rules fallback, the unused controlled classifier, or a missing assessment fails the test. Keep the service bound to loopback unless you intentionally secure it; never put the OpenRouter key in the HiRoute plan or command line.
