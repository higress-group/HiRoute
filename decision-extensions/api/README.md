# Custom extension API v1

[简体中文](README.zh-CN.md) · [Decision models and routing mechanism](../README.md) · [Download OpenAPI 3.1](decision.openapi.json)

Use this API to implement an HTTP decision extension for HiRoute. Built-in decision
models call the provider's System One API directly using a different request shape;
see the [provider mapping](system-one-design.md). Deploying an extension is optional.

HiRoute sends one `POST` to the complete endpoint saved for the custom connection.
The official extension serves **`POST /v1/decisions`**; HiRoute does not append a path.
One call can judge the current task and assess the preceding actual execution stage.
The extension follows the supplied definitions and frozen assessment standard;
HiRoute owns thresholds, model groups, candidate selection and availability relay.

## Request contract

The request has exactly five top-level fields:

| Field | Meaning |
| --- | --- |
| `decision` | An ordinal degree definition or a categorical task definition with optional degree refinements |
| `latest_user` | The complete current user content parts; repeated text is allowed |
| `visible_conversation` | Retained sealed execution turns in original order: `{user,status,steps}` |
| `history_partial` | Whether HiRoute knows of a gap in the visible history |
| `assessment_target` | `null`, or `{from,instructions,criteria}` for one preceding actual stage |

An **ordinal** definition contains `kind`, `instructions` and ordered
`levels:[{id,criterion}]`. Its level IDs are unique; their order represents increasing
degree. HiRoute's current routing uses two levels, `simple` and `complex`.

A **categorical** definition contains `kind`, `instructions` and
`options:[{id,criterion,refinement?}]`. Custom routing has 2–16 task categories.
Each category with primary models has an ordinal `refinement`; a category with
only regular models omits it. Choose by the current request's main intent using
the supplied conditions, including its overlap and default-category rules.
Category conditions describe what the task is; degree conditions describe the
work required within that category.

Instructions and criteria are non-empty text with **no per-field character quota**.
IDs and the surrounding structures retain the constraints in the OpenAPI.
Overall transport budgets and provider context limits still apply; they do not
permit silently truncating conditions or assessment standards.

### Visible history and the assessment target

A history turn has `user` content parts, a `status` of `completed`, `failed`,
`interrupted` or `unknown`, and `steps`, an array of content-part arrays. Each step
represents one business-model request. Parts contain accepted text, tool names with
explicit coarse status, or unavailable-content markers. HiRoute omits credentials,
system/developer instructions, tool arguments and tool-result bodies.

`assessment_target.from` is a zero-based index in `visible_conversation`. The
suffix from that index through the end belongs to one actual preceding stage.
`instructions` and the three `{score,criterion}` anchors, ordered **0, 0.5, 1**,
are frozen for that stage. Use them rather than deployment-specific scoring rules.
HiRoute binds the score to the actual stage locally; the extension does not return
internal model or execution identifiers.

`history_partial` describes gaps in the whole visible history. Assessment `partial`
describes missing evidence within the target stage. An uncaptured earlier prefix
alone does not make a later fully captured target stage partial. If the target is
`null`, omit assessment. Repeated user text, a summary or a continuation is not by
itself evidence of failure.

## Complete examples

These are illustrative requests and responses from the
[canonical examples](decision-examples.json), not measured provider results.
The same JSON is used in both language guides to keep sample IDs and criteria aligned.

### Smart saving: first decision, no assessment

Request:

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

Response:

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

There is no prior stage: `assessment_target` is `null`, and the response has no
`assessment`. With the default threshold, this distribution selects economy models.

### Writing and review: category, selected degree and prior-stage score

Request:

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

The current task selects `review`; only review's degree appears in the response.
The score **0.35** belongs to the previous **writing** stage. It does not evaluate
review work that has not happened and does not apply writing's low-score protection
to the review category. HiRoute uses review's current degree to select its group.

### A category with only regular models

Request:

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

Response:

```json
{
  "decision": {
    "kind": "categorical",
    "choice": "review"
  }
}
```

The review option has no `refinement`, so its response omits `refinement` too.
HiRoute uses that category's regular group. The writing option can still have two
groups; this does not require degree evaluation for review. A later request can
also assess the single-group category's preceding stage.

## Response validation

Return HTTP **200** and one strict JSON object, at most **64 KiB**. Markdown,
duplicate or unknown fields, and native provider envelopes are invalid.

An ordinal result returns the exact requested level IDs and finite probabilities
in `[0,1]`, covering every level and summing to one within `1e-6`. HiRoute validates
and normalizes this permitted floating-point error. A categorical result returns
an allowed `choice` and only that option's defined `refinement`, when present.
The extension cannot return model IDs, candidate order or routing thresholds.

Optional `assessment` contains a finite `score` in `[0,1]`, required `partial`,
and an optional `reason` of 1–1024 Unicode scalars. Preserve a valid zero score.
Missing, invalid or partial assessment is not zero and cannot drive low-score
protection. Omission does not delete saved observations; an old score is not reused
as a fresh assessment.

An invalid category invalidates the decision. A valid category with an invalid
selected degree remains selected, and HiRoute uses its primary group while
recording degree as unavailable. An invalid optional assessment is discarded
independently of a valid current decision. Errors in unselected upstream degree
answers must not invalidate the selected path.

## How HiRoute consumes the result

For a category with two groups, HiRoute selects regular/economy models when the
current `P(simple)` is at or above the configured threshold (default **0.8**) and
there is no applicable fresh score below the competence floor (default **0.5**).
Otherwise it selects primary. Protection requires a complete valid score for one
actual preceding stage with the same category, published plan version and rubric.
A single-group category records competence without an upgrade group.

Each new user message decides again; it can select either group. Tool continuations
and same-turn replay reuse the frozen decision only when HiRoute can recognize the
same turn and that decision remains reusable. Discontinuous reconstructed history
or a decision that cannot be inherited requires a new judgment. Availability relay
follows regular → same-category primary → failure, or stays within primary when that
group was selected directly. A candidate failure does not manufacture a competence score.

A whole decision failure uses heuristic smart saving or the custom plan's default
category, preferring its primary group when configured. Source cancellation,
source deadline and current-input integrity failures terminate the request.

## Provider integration and request limits

The extension owns its provider integration and context preparation. Preserve the
complete current user input. To fit provider limits, remove only the oldest whole
history turns and adjust the retained target index. If trimming removes target
evidence, return `partial:true`; if it removes the whole target, omit assessment.
HiRoute also accounts for target evidence it knows is missing; the extension's
`partial:false` cannot override those facts.

The built-in adapter bounds the complete provider request, including questions and
state, to **256 KiB**. The official Jev extension defaults to the same budget and
allows deployment configuration. This is a byte allocation budget, not a token
count or a replacement for provider context limits. If current input and questions
alone exceed the budget, reject the request instead of truncating them.
Business-model context remains the routing plan's setting.

HiRoute's total deadline covers preparation, credential access, connection and
response reading. The production transport makes one call with no automatic
background retry and does not follow redirects. Configure the extension's own
budget to fit within the HiRoute connection timeout.

The [protocol design](decision-design.md) describes the current v1 boundary;
the [System One mapping](system-one-design.md) describes provider Choice/Score calls.
Tool subset selection remains future documentation only and is not supported by
current routing or the extension API runtime.
