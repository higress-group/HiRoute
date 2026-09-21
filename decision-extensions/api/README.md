# Decision API

[Simplified Chinese](README.zh-CN.md) · [Mechanism and usage](../README.md)

[Download the canonical OpenAPI 3.1 JSON](decision.openapi.json). This single schema is also embedded by Desktop's protocol dialog and Save OpenAPI action. The request represents an arbitrary map of allowed branches, not a hard-coded binary choice. Do not maintain a second schema in an extension.

The official extension and OAS use `POST /v1/decisions`. HiRoute sends to the **complete endpoint configured in the plan**, so a custom service may choose its own path. Existing Jev deployments must update their configured endpoint when installing this version; the service exposes no old-path alias.

## Request

| Required field | Meaning |
| --- | --- |
| `branches` | Allowed branch IDs mapped to their meanings; return one exact key |
| `latest_user` | Complete, non-empty user content projected from the current request; it may repeat a prior round or be a summary/continuation |
| `visible_conversation` | Retained sealed routing execution rounds in order, with branch, round-start user content, status and steps |
| `history_partial` | Whether retained history is incomplete |
| `assessment_from` | Zero-based start of the assessable suffix of history, or `null` when no reliable target exists |

Each step is one business-model request, containing accepted text, tool activity or unavailable-content markers. Tool activity carries `tool` and `status` (`completed`, `failed`, `unknown`), never arguments or results. Status comes from explicit protocol facts; free-form error text is not parsed into a failure. A routing execution round is one branch decision plus the model requests that inherited it; one user task can span several rounds. A round sealed only because HiRoute reached another decision boundary can remain `unknown`. It can also identify an `executed_branch_id` different from the selected branch. See the OAS for exact types and status values.

```sh
curl --fail-with-body --request POST 'http://127.0.0.1:8080/v1/decisions' \
  --header 'Content-Type: application/json' \
  --data '{
    "branches": {
      "smart_saving_simple": "A clear, well-scoped task for the economy branch.",
      "smart_saving_complex": "A task requiring deeper reasoning for the primary branch."
    },
    "latest_user": [{"kind":"text","text":"Fix this failing test."}],
    "visible_conversation": [],
    "history_partial": false,
    "assessment_from": null
  }'
```

Calling the official extension can incur OpenRouter charges. Add the configured authentication header if enabled. Store its complete value in a HiRoute Secret when configuring the plan; the OpenRouter key belongs only in the external service.

## Response and assessment

The first-turn example above should return only a branch, such as `{"branch_id":"smart_saving_simple"}`. For a later request with a non-null `assessment_from`, a service can additionally return:

```json
{
  "branch_id": "smart_saving_complex",
  "assessment": {
    "score": 0.25,
    "partial": false,
    "reason": "Repeated failed attempts did not resolve the previous task."
  }
}
```

`assessment` is optional; its `reason` is also optional. `score` is finite competence in `[0,1]`. `partial` says whether the service omitted evidence from the requested assessment suffix. HiRoute combines this with its own history gaps and binds the score to the snapshot's actual execution stage, not the branch being selected for the new round. Repeated user text, a summary, or a continuation is not by itself positive or negative feedback. Omission leaves an existing score unchanged. There is no model ID, stage ID or evaluator version for the service to echo.

Return one plain JSON object without a vendor envelope, Markdown, duplicate or unknown fields. The response limit is 64 KiB. An invalid branch invalidates the response; an invalid optional assessment is discarded while a valid branch can still be used.

## Context and failures

HiRoute does not impose an additional fixed input-size cutoff or truncate current user text. Its 8 KiB inline threshold selects storage location; larger content is read from ReplayStore. Retained turn history is separately memory-bounded and can be partial. The service owns model-specific context selection. If it trims any assessable evidence, report `partial: true`; if no assessable content remains, omit the assessment.

The plan's `timeout_ms` covers history preparation, credentials, connection and response reading, also bounded by source cancellation/deadline. The default is 3000 ms; the official service defaults to a 2.8-second total budget. Keep the service budget slightly lower than the plan's. HiRoute calls once without retries. External service failures use local rules; source cancellation/deadline and local Replay integrity/resource failures terminate rather than continuing under a false service-failure label.

For current smart-saving settings and precise tool-status mappings, see the [implementation guide](../../docs/smart-saving-model-classification.md).
