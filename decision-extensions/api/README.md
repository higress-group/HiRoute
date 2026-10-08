# Custom extension API v1

[简体中文](README.zh-CN.md) · [Decision models and routing mechanism](../README.md) · [Download OpenAPI 3.1](decision.openapi.json)

A custom extension is an **HTTP decision service that you deploy**. HiRoute sends it the current task, decision criteria and available history. The service returns a task category or complexity probabilities, optionally assessing the preceding execution stage. HiRoute then applies the routing plan, selects a model and calls that model to carry out the user's task.

This guide covers connecting a service, handling requests and returning results. You can deploy the [official Jev extension](../extensions/jev-decider/README.md), or implement the same interface with another decision model, an LLM or your own rules.

## Where an extension fits

```text
Published routing plan: task criteria, assessment rubric, thresholds and model groups
  → HiRoute sends: definition + current input + visible history + assessment target
  → Your HTTP extension returns: decision + optional assessment
  → HiRoute applies thresholds and competence protection, then selects a model group
  → Execution model handles the task; its recorded work informs later decisions
```

The extension turns the supplied questions into judgments. It controls provider calls, prompt construction or rule evaluation, along with any upstream credentials and response conversion. Definitions and assessment criteria arrive with each request. Following them allows one service to handle different routing plans.

| Configuration or behavior | Owner |
| --- | --- |
| Task categories, complexity criteria and assessment rubric | HiRoute, from the published plan |
| Computing a category, complexity probabilities and optional assessment | The extension |
| Thresholds, execution-model lists, eligibility checks and failover | HiRoute |
| Attributing assessments to actual execution stages and storing observations | HiRoute |

The response contains judgments. HiRoute manages execution-model IDs, model groups and candidate ordering; these are not extension response fields. The extension does not take over execution of the user's task.

**Built-in connections and custom extensions use different interfaces.** With a built-in decision model, HiRoute calls the provider's System One API directly. A custom extension receives the HTTP JSON request documented here. The [System One mapping](system-one-design.md) is relevant inside an extension that uses a compatible provider such as Jev. Your service can use a different upstream protocol.

## Connect a service

1. Implement an HTTP `POST` endpoint accepting JSON, or deploy the [official Jev extension](../extensions/jev-decider/README.md). Its path is `/v1/decisions`; a custom service may use another path.
2. Under **Models → Decision models**, open the add menu and choose **Connect custom extension**. Enter the complete endpoint, connection timeout and optional authentication header name and full value.
3. Save and test the connection. The test checks transport and required response fields; use representative tasks to evaluate decision quality.
4. Select the saved connection in a routing plan, configure task criteria and model groups, and publish. After editing a connection or draft, select the intended connection revision and republish to apply it.

HiRoute posts to the **complete configured endpoint** without appending a path. For example, `http://127.0.0.1:8080/v1/decisions` must point to the extension. For a remote deployment, use an address reachable from HiRoute. Configure the extension's upstream provider URL inside the extension.

## Two decision kinds and a separate assessment

Field names, `kind` values and status values are literal protocol identifiers. The current `decision.kind` values are `ordinal` and `categorical`:

| Structure | Question it answers | Extension output |
| --- | --- | --- |
| `ordinal` | How demanding is the current task? | `probabilities`: a complete distribution over the requested level IDs; current model routing uses `simple` and `complex` |
| `categorical` | What kind of task is this, such as writing or reviewing? | `choice`: one allowed option ID, plus its `ordinal` result if that option defines a `refinement` |
| `assessment` | How well was the preceding execution stage handled? | An optional object with required `score` and `partial`, plus optional `reason`; a separate response field, not a `decision.kind` |

A `refinement` is a further judgment within the chosen category. In current routing it measures that category's task complexity; return only the chosen category's result. IDs such as `simple`, `complex`, `writing` and `review` must exactly match the request. Do not translate or rename them. Instructions, criteria, user text and assessment reasons are natural-language content and may be written in any language.

## Minimal example: judge task complexity

The payloads below come from the [canonical examples](decision-examples.json). Numbers are illustrative. Both language editions use the same payloads; the Chinese instructions and user messages are valid example input, not protocol identifiers.

Smart saving submits one `ordinal` definition. The first request has no previous stage to assess, so `assessment_target` is `null`.

Request (save as `request.json`):

```json
{
  "decision": {
    "kind": "ordinal",
    "instructions": "判断当前任务需要的处理程度，不评价上一阶段表现。",
    "levels": [
      {
        "id": "simple",
        "criterion": "需求明确、边界清楚，可沿用已有模式完成。"
      },
      {
        "id": "complex",
        "criterion": "需要深入推理、跨模块诊断或设计新的方案。"
      }
    ]
  },
  "latest_user": [
    {
      "kind": "text",
      "text": "把这段说明改成三条清晰的要点。"
    }
  ],
  "visible_conversation": [],
  "history_partial": false,
  "assessment_target": null
}
```

Send it to the running service. Include the configured authentication header if enabled:

```sh
curl --fail-with-body 'http://127.0.0.1:8080/v1/decisions' \
  --header 'Content-Type: application/json' \
  --data-binary @request.json
```

Return HTTP `200`, `Content-Type: application/json` and this response:

```json
{
  "decision": {
    "kind": "ordinal",
    "probabilities": {
      "simple": 0.93,
      "complex": 0.07
    }
  }
}
```

The extension reports a **0.93** probability for `simple`. HiRoute compares it with the plan's threshold, **0.8** by default. With no competence protection applying here, HiRoute selects the economy group. The extension does not return a group or model ID. Omit `assessment` when there is no assessment target.

## Request fields

All five top-level fields are required. Use `[]` for empty history and `null` for an absent assessment target:

| Field | Meaning |
| --- | --- |
| `decision` | The current `ordinal` or `categorical` definition, including criteria and allowed IDs |
| `latest_user` | Complete, non-empty user content parts from the current request; repeated text is allowed |
| `visible_conversation` | Retained, closed execution turns in their original order; each item is `{user,status,steps}` |
| `history_partial` | Whether HiRoute knows of a gap anywhere in the visible history |
| `assessment_target` | `null`, or `{from,instructions,criteria}` identifying the preceding actual stage to assess |

### decision definitions

- **`ordinal`** contains `kind: "ordinal"`, `instructions` and `levels: [{id,criterion}]`. IDs are unique, with levels ordered from lowest to highest. The protocol allows multiple levels; current HiRoute model routing produces `simple` and `complex`.
- **`categorical`** contains `kind: "categorical"`, `instructions` and `options: [{id,criterion,refinement?}]`. Custom routing supplies 2–16 categories. Select one by the current task's main intent, following the supplied overlap and default-category rules.
- **`refinement`** is an optional nested `ordinal` definition. A category with a primary model group includes it; a category with only regular models omits it. Category criteria describe the type of task, while refinement criteria describe its complexity within that category.

`instructions` and `criterion` are non-empty text with no per-field character quota. See the [OpenAPI](decision.openapi.json) for ID, array-length and other structural constraints. Overall transport and upstream context limits still apply; they do not permit silently truncating decision criteria or assessment rubrics.

### visible_conversation: execution history

Each turn contains its original `user` content, `status` and `steps`. One step represents one execution-model request, so `steps` is an array of content-part arrays.

| Location | Supported values or content |
| --- | --- |
| Turn `status` | `completed`, `failed`, `interrupted`, `unknown` |
| Content parts in `user` and `latest_user` | `text`, or `unavailable` with a `source_kind` identifying the missing content type |
| Content parts in `steps` | `text`, `unavailable`, or `tool_activity` with a tool name in `tool` and a coarse `status` |
| Tool `status` | `completed`, `failed`, `unknown` |

`unknown` means there is no explicit terminal outcome; it does not establish success or failure. Tool status comes from explicit protocol facts. History excludes credentials, system/developer instructions, tool arguments and tool-result bodies.

### assessment_target: which work to assess

`assessment_target.from` is a zero-based index into `visible_conversation`. The suffix starting at that index belongs to one actual execution stage. For example, in a three-item history, `from: 1` targets only the last two items. The new task, which has not executed yet, is outside this target.

`instructions` and `criteria` contain the rubric frozen when that stage executed. The three `{score,criterion}` anchors are **0, 0.5 and 1**, in that order. The returned `score` may be any finite number in `[0,1]`, not just an anchor value. HiRoute retains the target's model and stage identity locally; the extension need not infer or echo it.

`history_partial` describes the whole visible history; `assessment.partial` concerns missing evidence within the target stage. A missing earlier prefix does not make an otherwise complete target partial. Return `partial: true` if the extension trims target evidence; omit assessment if it removes the entire target. Repeated user text, summaries or continuation messages do not by themselves establish failure.

## categorical examples: category and task complexity

### Select reviewing while assessing earlier writing

The request defines `writing` and `review`, each with its own `refinement`. The current task asks for a review, while the assessment target refers to the preceding writing stage.

<details>
<summary>Full request: two categories, their refinements and a historical assessment target</summary>

```json
{
  "decision": {
    "kind": "categorical",
    "instructions": "按当前请求的主要意图选择一个工作类别，不按任务难度或上次失败选择。条件重叠时选择最符合主要意图的一项；均不匹配时选择默认类别 writing。",
    "options": [
      {
        "id": "writing",
        "criterion": "撰写、续写或改写文章，包括根据反馈修稿。",
        "refinement": {
          "kind": "ordinal",
          "instructions": "假设当前任务交给写稿分支，判断写稿工作所需程度。",
          "levels": [
            {
              "id": "simple",
              "criterion": "事实和提纲充分，仅需常规组织或局部改写。"
            },
            {
              "id": "complex",
              "criterion": "需要综合冲突材料、构建论证或重组全文。"
            }
          ]
        }
      },
      {
        "id": "review",
        "criterion": "审阅、核实文章并提出意见，不直接重写全文。",
        "refinement": {
          "kind": "ordinal",
          "instructions": "假设当前任务交给审稿分支，判断审稿工作所需程度。",
          "levels": [
            {
              "id": "simple",
              "criterion": "范围明确的格式、措辞与已知事实核查。"
            },
            {
              "id": "complex",
              "criterion": "需要逐项核对来源、检查因果推断或全篇论证。"
            }
          ]
        }
      }
    ]
  },
  "latest_user": [
    {
      "kind": "text",
      "text": "请逐项核对这篇稿件的来源与因果论断，给出审稿意见。"
    }
  ],
  "visible_conversation": [
    {
      "user": [
        {
          "kind": "text",
          "text": "根据所给材料写一篇文章。"
        }
      ],
      "status": "completed",
      "steps": [
        [
          {
            "kind": "text",
            "text": "文章已写出，但将两个没有因果证据的现象写成了因果关系。"
          }
        ]
      ]
    }
  ],
  "history_partial": false,
  "assessment_target": {
    "from": 0,
    "instructions": "评价上一写稿阶段的材料忠实性与论证，不评价尚未发生的审稿。",
    "criteria": [
      {
        "score": 0,
        "criterion": "关键事实失真、无进展或需要大幅纠正。"
      },
      {
        "score": 0.5,
        "criterion": "有用但不完整，仍有明显事实或论证问题。"
      },
      {
        "score": 1,
        "criterion": "忠实于材料、归因准确、完成写稿要求。"
      }
    ]
  }
}
```

</details>

Response:

```json
{
  "decision": {
    "kind": "categorical",
    "choice": "review",
    "refinement": {
      "kind": "ordinal",
      "probabilities": {
        "simple": 0.28,
        "complex": 0.72
      }
    }
  },
  "assessment": {
    "score": 0.35,
    "partial": false,
    "reason": "上一写稿阶段存在没有证据支撑的因果断言。"
  }
}
```

`choice: "review"` selects reviewing, and `refinement` describes only the current review's complexity. Its `P(simple) = 0.28` is below the default threshold, so HiRoute selects the review category's primary group. The **0.35** assessment belongs to the earlier **writing** stage. A low writing score cannot trigger competence protection for reviewing.

### Select a category with one model group

A plan can mix categories with one or two model groups. Here, `writing` still has a `refinement`, while `review` has only regular models and omits it.

<details>
<summary>Full request: writing has a refinement; review does not</summary>

```json
{
  "decision": {
    "kind": "categorical",
    "instructions": "按当前请求的主要意图选择一个工作类别，不按任务难度或上次失败选择。条件重叠时选择最符合主要意图的一项；均不匹配时选择默认类别 writing。",
    "options": [
      {
        "id": "writing",
        "criterion": "撰写、续写或改写文章，包括根据反馈修稿。",
        "refinement": {
          "kind": "ordinal",
          "instructions": "假设当前任务交给写稿分支，判断写稿工作所需程度。",
          "levels": [
            {
              "id": "simple",
              "criterion": "事实和提纲充分，仅需常规组织或局部改写。"
            },
            {
              "id": "complex",
              "criterion": "需要综合冲突材料、构建论证或重组全文。"
            }
          ]
        }
      },
      {
        "id": "review",
        "criterion": "审阅、核实文章并提出意见，不直接重写全文。"
      }
    ]
  },
  "latest_user": [
    {
      "kind": "text",
      "text": "请逐项核对这篇稿件的来源与因果论断，给出审稿意见。"
    }
  ],
  "visible_conversation": [],
  "history_partial": false,
  "assessment_target": null
}
```

</details>

The response only needs to select `review`:

```json
{
  "decision": {
    "kind": "categorical",
    "choice": "review"
  }
}
```

HiRoute uses the review category's regular group. The extension does not invent complexity probabilities for that category. A later request can still assess its completed execution stage.

## Response rules and error handling

Return HTTP **200** and one JSON object, at most **64 KiB**. Do not wrap it in Markdown or a native provider response, or add duplicate or unknown fields.

| Field | Requirement |
| --- | --- |
| `decision.kind` | Must match the request |
| `decision.probabilities` | For `ordinal`: exactly the requested level IDs; every value finite and in `[0,1]`, with the sum within `1e-6` of 1; HiRoute normalizes this permitted floating-point error |
| `decision.choice` | For `categorical`: one exact ID from the request's `options` |
| `decision.refinement` | The selected option's defined `ordinal` result; omit when that option has no refinement |
| `assessment` | Optional; finite `score` in `[0,1]`, required `partial`, and optional `reason` of 1–1024 Unicode scalars |

Omit `assessment` when `assessment_target` is `null`. Preserve a valid zero score; an absent score is not zero and does not delete saved observations.

| Condition | HiRoute behavior |
| --- | --- |
| `choice` outside the allowed set, or an invalid overall response | Decision failure; use the plan fallback described below |
| Missing or invalid smart-saving `ordinal` probabilities | Use primary and record complexity as unavailable |
| Valid category but missing or invalid required `refinement` | Keep the category, use its primary group and record complexity as unavailable |
| Missing, invalid or untargeted optional assessment | Do not use that assessment; a valid current decision remains usable |
| Assessment with `partial: true` | Record partial evidence; do not trigger competence protection |

If your upstream answers complexity questions for several categories at once, convert only the selected category's answer. An invalid unselected answer must not discard a valid selected path.

## How HiRoute acts on the result

For a category with two groups, HiRoute selects regular/economy models when the current `P(simple)` reaches the configured threshold (default **0.8**) and no applicable fresh score is below the competence floor (default **0.5**). Otherwise it selects primary. Protection requires a complete valid assessment of the actual preceding stage, matching the current category, published plan version and rubric. Saved scores are not reused as new low scores on later requests. A single-group category records competence without a primary group to upgrade to.

Each new user message triggers a fresh decision. Recognized tool continuations and same-turn replay inherit a frozen decision while it remains reusable; discontinuous history or a decision that cannot be inherited triggers another call. The extension should process each request without requiring the user text to change.

HiRoute tries eligible models in the selected group in order. Exhausting regular models can lead to the same category's primary group; selecting primary directly stays within that group. Candidate failures do not create zero competence scores. A whole decision failure uses heuristic rules for smart saving, or the custom plan's default category with its primary group when available.

## Context, timeouts and call boundaries

The extension owns upstream integration and context preparation. Preserve the complete current user input, decision criteria and assessment rubric. If trimming is necessary, remove the oldest whole history turns first and adjust the target index passed upstream. Determine assessment coverage from the actual target range. HiRoute also includes target gaps it knows about; an extension's `partial: false` cannot override them.

The official Jev extension defaults to a **256 KiB** complete provider-request budget, configurable at deployment. HiRoute's built-in adapter also uses 256 KiB. This is a byte budget for the provider request, not a token or context limit imposed on every custom implementation by this API. If the current input and questions alone do not fit, reject the request instead of truncating them. The routing plan still controls the execution model's context.

HiRoute makes one production call to the extension, with no automatic background retries or redirects. Its total deadline includes preparation, credential access, connection and response reading. Keep the extension's own budget below the HiRoute connection timeout. Extension outages and request failures use the decision fallback above; source cancellation, source deadline or current-input integrity errors terminate the request.

See the [OpenAPI](decision.openapi.json) for exact types, the [canonical examples](decision-examples.json) for reusable payloads, and the [protocol design](decision-design.md) for background. The current runtime supports `ordinal` and `categorical`; the documented `subset` tool-selection design is not yet implemented.
