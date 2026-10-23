# Custom extension API

[简体中文](README.zh-CN.md) · [Decision models and routing](../README.md) · [Download OpenAPI](decision.openapi.json) · [Canonical examples](decision-examples.json)

A custom extension is an HTTP decision service you deploy. On each new user turn, HiRoute supplies the published categories, conditions and available groups with safe user content and history. Your extension returns the category and `regular` or `primary` group directly; HiRoute validates and executes the candidates. Deploy the [official Jev extension](../extensions/jev-decider/README.md), another decision model, or your own rules.

## Connect

Under Models → Decision models, add a custom extension with its complete endpoint, timeout and optional authentication. Save and test it, then select that saved revision in a route and publish. HiRoute POSTs to the complete endpoint without appending a path; the reference implementation uses `/v1/decisions`. The synthetic connection test calls no execution model and does not establish decision quality.

## Request

All five top-level fields are required: `route`, `latest_user`, `visible_conversation`, `history_partial`, and `preceding_stage`.

For `route.kind: "smart_saving"`, groups are `["regular", "primary"]`. For `branches`, categories are 2–16 `{id, condition, groups}` entries and `default_category` names a published category. Every category has regular; primary is present only when configured. The extension cannot choose unpublished categories, models, candidate positions or tools.

`latest_user` is the current user content array. Each `visible_conversation` item is `{user,status,steps}`, with status `completed`, `failed`, `interrupted` or `unknown`. Content is `{kind:"text",text}` or `{kind:"unavailable",source_kind}`; steps may also contain `{kind:"tool_activity",tool,status}`, whose status is `completed`, `failed` or `unknown`. Credentials, system/developer instructions, tool arguments and tool-result bodies are excluded. Unknown status is not failure evidence.

`preceding_stage` is null or `{from,category,group,complete}`. From indexes the start of the preceding actual stage in retained history; group is the actual execution group, and complete describes that stage's record. HiRoute checks exact plan and connection revisions, published category and unambiguous execution locally. Internal stage/model IDs never leave HiRoute. The request identifies the actual preceding category; if the response chooses another category, its assessment is discarded. `history_partial` and target completeness are independent: loss of an earlier prefix does not make a completely retained target partial.

No degree prompt, threshold, rubric or assessment anchors are sent. The extension's deployment configuration owns its policy and scoring method.

## Response and failures

The response may contain `category`, `group`, `reason`, and `assessment`. Branch routes require a published category; difficulty routing omits category. Dual-group scopes require group; single-group categories ignore group and always execute regular. Optional reason contains 1–1024 characters. Optional assessment is exactly `{score,partial}` with a finite score in 0..1 and a boolean partial flag. Assessments are observation-only and never influence HiRoute routing.

Return HTTP 200 and strict JSON with no duplicate or unknown members, at most 64 KiB. A valid category with missing/invalid group keeps that category and uses primary with decision-unavailable attribution. Invalid assessment is discarded alone. A failed call, invalid category, legacy response, unknown member or transport failure uses heuristic fallback for difficulty routing, or the default category's primary group (regular when no primary exists) for branches. Source cancellation, total deadline and replay-integrity failure terminate without fallback.

## Turn handling and compatibility

Every new user turn decides again. Same-turn tool continuations and replay inherit the frozen decision while history and authority remain valid. HiRoute retains held-candidate preference within the selected group and regular-to-primary availability relay; direct primary selection never downgrades. Model failures cannot manufacture scores.

The current shape replaces the earlier v1 probability response in place, without a legacy reader or version selector. Custom execution supports only the current format without judgment settings; old custom-only persisted formats have no compatibility adapter. Current saved connection fields are unchanged, and supported built-in data and shared Observation history retain their existing readers. An un-upgraded extension fails save-and-test and uses the execution fallback above. Drafts retain built-in judgment edits, but extension routes neither display nor publish those execution settings.

## Examples

### smart_saving

Request:

```json
{
  "latest_user": [
    {
      "kind": "text",
      "text": "把这段说明改成三条清晰的要点。"
    }
  ],
  "visible_conversation": [],
  "history_partial": false,
  "route": {
    "kind": "smart_saving",
    "groups": [
      "regular",
      "primary"
    ]
  },
  "preceding_stage": null
}
```

Response:

```json
{
  "group": "regular"
}
```

### writing_review

Request:

```json
{
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
          "text": "检查这篇文章的事实和因果论断。"
        }
      ],
      "status": "completed",
      "steps": [
        [
          {
            "kind": "text",
            "text": "已完成审稿，但遗漏了没有证据支持的因果断言。"
          }
        ]
      ]
    }
  ],
  "history_partial": false,
  "route": {
    "kind": "branches",
    "categories": [
      {
        "id": "writing",
        "condition": "撰写、续写或改写文章，包括根据反馈修稿。",
        "groups": [
          "regular",
          "primary"
        ]
      },
      {
        "id": "review",
        "condition": "审阅、核实文章并提出意见，不直接重写全文。",
        "groups": [
          "regular",
          "primary"
        ]
      }
    ],
    "default_category": "writing"
  },
  "preceding_stage": {
    "from": 0,
    "category": "review",
    "group": "regular",
    "complete": true
  }
}
```

Response:

```json
{
  "group": "primary",
  "category": "review",
  "assessment": {
    "score": 0.35,
    "partial": false
  },
  "reason": "Previous review stage needs primary protection"
}
```

### single_group_category

Request:

```json
{
  "latest_user": [
    {
      "kind": "text",
      "text": "请逐项核对这篇稿件的来源与因果论断，给出审稿意见。"
    }
  ],
  "visible_conversation": [],
  "history_partial": false,
  "route": {
    "kind": "branches",
    "categories": [
      {
        "id": "writing",
        "condition": "撰写、续写或改写文章，包括根据反馈修稿。",
        "groups": [
          "regular",
          "primary"
        ]
      },
      {
        "id": "review",
        "condition": "审阅、核实文章并提出意见，不直接重写全文。",
        "groups": [
          "regular"
        ]
      }
    ],
    "default_category": "writing"
  },
  "preceding_stage": null
}
```

Response:

```json
{
  "group": "regular",
  "category": "review"
}
```
