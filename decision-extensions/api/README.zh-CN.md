# 自定义扩展 API v1

[English](README.md) · [决策模型与路由机制](../README.zh-CN.md) · [下载 OpenAPI 3.1](decision.openapi.json)

自定义扩展是你部署的 **HTTP 决策服务**。HiRoute 把当前任务、判断条件和可见历史发给它；服务返回任务分类或复杂度概率，并可同时评价上一执行阶段。HiRoute 根据这些结果和路由计划选择模型，再调用模型完成用户任务。

本文面向扩展开发者，说明如何接入服务、处理请求和返回结果。可以直接部署[官方 Jev 扩展](../extensions/jev-decider/README.zh-CN.md)，也可以用其他决策模型、LLM 或自有规则实现同一接口。

## 扩展在路由中做什么

```text
已发布的路由计划：任务条件、评分标准、阈值和模型组
  → HiRoute 发出决策请求：定义 + 当前输入 + 可见历史 + 评分目标
  → 你的 HTTP 扩展：执行判断，返回 decision 和可选 assessment
  → HiRoute：应用阈值和低分保护，选择类别内的模型组
  → 执行模型：处理用户任务；执行记录供之后的决策参考
```

扩展负责把请求中的问题变成判断结果。如何调用上游、组织提示词或执行规则由扩展实现；如果使用上游供应商，其凭证和响应转换也由扩展管理。判断条件和评分标准随每次请求传入，服务必须遵守它们，才能让同一个服务用于不同路由计划。

| 配置或行为 | 负责方 |
| --- | --- |
| 任务类别、复杂度条件、评分标准 | HiRoute 从已发布计划中提供 |
| 计算类别、复杂度概率和可选评分 | 扩展 |
| 阈值、执行模型列表、模型资格检查和故障接力 | HiRoute |
| 将评分归属到实际执行阶段并保存观测 | HiRoute |

这个接口返回的是判断结果。执行模型 ID、模型组和候选顺序由 HiRoute 管理，不放进扩展响应。扩展也不接管用户任务的实际执行。

**内置接入与自定义扩展是两种连接方式。** 内置决策模型由 HiRoute 直接调用供应商的 System One API；自定义扩展则接收本文的 HTTP JSON 请求。只有使用 Jev 等兼容供应商时，扩展内部才需要做 [System One 映射](system-one-design.md)。自建服务可以使用其他上游协议。

## 接入一个服务

1. 实现接收 JSON 的 HTTP `POST` 接口，或部署[官方 Jev 扩展](../extensions/jev-decider/README.zh-CN.md)。官方路径是 `/v1/decisions`；自建服务可选其他路径。
2. 在 **模型 → 决策模型** 中打开添加菜单，选择 **接入自定义扩展**。填写完整 endpoint、连接超时，以及可选的认证头名和完整值。
3. 保存并测试连接。连接测试检查传输和必要响应字段；实际判断效果需用代表性任务验证。
4. 在路由计划中选择这个连接，配置任务条件与模型组，然后发布。修改连接或草稿后，需要选择相应连接版本并重新发布才会生效。

HiRoute 向配置的**完整 endpoint** 发请求，不自行追加路径。例如 `http://127.0.0.1:8080/v1/decisions` 必须指向扩展服务；远程部署时改为 HiRoute 可访问的地址。扩展使用的上游供应商地址在扩展内部配置。

## 两种 decision，以及独立的 assessment

字段名、`kind` 和状态值保留协议原文。当前 `decision.kind` 支持 `ordinal` 和 `categorical`：

| 结构 | 回答的问题 | 扩展返回什么 |
| --- | --- | --- |
| `ordinal` | 当前任务需要多高的处理能力？ | `probabilities`：请求中各个 level ID 的完整概率分布；当前模型路由使用 `simple`、`complex` |
| `categorical` | 当前任务属于哪一类，例如写稿还是审稿？ | `choice`：一个允许的 option ID；该选项带 `refinement` 时，同时返回它的 `ordinal` 结果 |
| `assessment` | 上一实际执行阶段完成得如何？ | 可选评分对象，含必填 `score`、`partial` 和可选 `reason`；它是响应的独立字段，不是 `decision.kind` |

`refinement` 是选定类别内的进一步判断。在当前路由中，它表示该类任务的复杂度；只返回选中类别的结果。`simple`、`complex`、`writing`、`review` 等 ID 必须与本次请求完全一致，不能翻译或自行改名。`instructions`、`criterion`、用户文本和 `reason` 是自然语言内容，可以使用中文。

## 最小示例：判断当前任务的复杂度

以下示例来自[规范示例](decision-examples.json)。数值仅用于说明；中英文文档共用这些请求和响应，其中的中文条件和用户输入也是合法的示例数据。

智能省钱提交一个 `ordinal`。首次请求没有历史评分目标，将 `assessment_target` 设为 `null`。

请求（保存为 `request.json`）：

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

向已启动的服务发送请求；启用了认证时加上配置的认证头：

```sh
curl --fail-with-body 'http://127.0.0.1:8080/v1/decisions' \
  --header 'Content-Type: application/json' \
  --data-binary @request.json
```

服务返回 HTTP `200`、`Content-Type: application/json` 和以下响应：

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

扩展给出 `simple` 的概率 **0.93**；HiRoute 将它与计划阈值比较。默认阈值为 **0.8**，这次没有低分保护，因此选择省钱组。扩展无需返回“省钱组”或具体模型 ID。没有评分目标时省略 `assessment`。

## 请求字段

顶层五个字段均必填，空历史使用 `[]`，没有评分目标使用 `null`：

| 字段 | 含义 |
| --- | --- |
| `decision` | 本次判断定义：`ordinal` 或 `categorical`，包含条件和允许的 ID |
| `latest_user` | 当前请求的完整、非空用户 content parts；允许与先前文本重复 |
| `visible_conversation` | 已结束并保留的执行轮次，按原顺序排列；每项为 `{user,status,steps}` |
| `history_partial` | HiRoute 是否已知整段可见历史存在缺口 |
| `assessment_target` | `null`，或 `{from,instructions,criteria}`，指定要评分的上一实际阶段 |

### decision 定义

- **`ordinal`**：包含 `kind: "ordinal"`、`instructions` 和 `levels: [{id,criterion}]`。ID 唯一，数组顺序表示从低到高的程度。协议结构允许多个 level，当前 HiRoute 模型路由生成 `simple`、`complex` 两档。
- **`categorical`**：包含 `kind: "categorical"`、`instructions` 和 `options: [{id,criterion,refinement?}]`。自定义路由提供 2–16 个类别；按当前任务的主要意图及传入的重叠、默认类别规则选择一项。
- **`refinement`**：可选的嵌套 `ordinal` 定义。有主力模型组的类别提供它，只有常规模型组的类别省略它。类别条件描述任务是什么，`refinement` 条件描述这类任务的工作复杂度。

`instructions` 和 `criterion` 是非空文本，没有逐字段字符上限。ID、数组长度和其他结构约束见 [OpenAPI](decision.openapi.json)。整体传输和上游上下文限制仍适用，不能因此静默截断判断条件或评分标准。

### visible_conversation：执行历史

每个轮次包含当时的 `user` 内容、`status` 和 `steps`。一个 step 对应一次执行模型请求，所以 `steps` 是 content parts 数组的数组。

| 位置 | 支持的值或内容 |
| --- | --- |
| 轮次 `status` | `completed`、`failed`、`interrupted`、`unknown` |
| `user` 和 `latest_user` 中的 content part | `text`（文本）或 `unavailable`（无法提供的内容，以 `source_kind` 说明来源类型） |
| `steps` 中的 content part | `text`、`unavailable`，或 `tool_activity`（工具名称 `tool` 与粗粒度 `status`） |
| 工具 `status` | `completed`、`failed`、`unknown` |

`unknown` 表示缺少明确的终态，不能据此推断成功或失败。工具状态来自显式协议事实。历史不包含凭据、system/developer 指令、工具参数或工具结果正文。

### assessment_target：评价哪段历史

`assessment_target.from` 是 `visible_conversation` 的零基下标。从该下标到数组末尾，属于同一个要评分的实际执行阶段。例如历史有三项，`from: 1` 只评价后两项。当前尚未执行的任务不属于这个目标。

`instructions` 和 `criteria` 使用该阶段执行时冻结的评分标准。`criteria` 恰有三项 `{score,criterion}`，锚点按顺序为 **0、0.5、1**；返回的 `score` 可以是 `[0,1]` 内任意有限数值，不限于这三个锚点。HiRoute 在本地保存目标的模型与阶段归属，扩展不需要猜测或回传内部身份。

`history_partial` 描述整段历史，响应中的 `assessment.partial` 描述评分目标内是否缺少证据。更早的非目标历史缺失，不会自动使完整的目标阶段变成部分证据。扩展裁剪了目标证据时返回 `partial: true`；目标全部移除时省略评分。重复用户文本、摘要或继续消息本身不算失败证据。

## categorical 示例：类别与类内复杂度

### 选择审稿，同时评价上一写稿阶段

请求的两个类别是 `writing` 和 `review`，各自有独立的 `refinement`。本次任务要求审稿，评分目标则指向上一写稿阶段。

<details>
<summary>完整请求：两个类别、各自的 refinement，以及历史评分目标</summary>

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

响应：

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

`choice: "review"` 选择审稿，`refinement` 只描述这次审稿的复杂度。`P(simple) = 0.28` 低于默认阈值，HiRoute 选择审稿的主力组。**0.35** 分评价的是上一**写稿**阶段；写稿低分不能用于审稿类别的低分保护。

### 选中的类别只有一组模型

一个计划内可以同时有双组类别和单组类别。这里 `writing` 仍有 `refinement`，`review` 只有常规模型组，因此没有 `refinement`。

<details>
<summary>完整请求：writing 有 refinement，review 没有</summary>

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

响应只需选择 `review`：

```json
{
  "decision": {
    "kind": "categorical",
    "choice": "review"
  }
}
```

HiRoute 使用审稿的常规组。扩展不为该类别补造复杂度概率；后续请求仍可评价它已执行阶段的胜任度。

## 响应规则与错误处理

返回 HTTP **200** 和单个 JSON 对象，响应最大 **64 KiB**。不要包裹 Markdown 或供应商原生响应，也不要添加重复或未知字段。

| 字段 | 要求 |
| --- | --- |
| `decision.kind` | 与请求一致 |
| `decision.probabilities` | `ordinal` 结果使用本次所有 level ID，不缺项、不多项；每项是有限 `[0,1]` 概率，总和与 1 的误差不超过 `1e-6`；HiRoute 会归一化这部分浮点误差 |
| `decision.choice` | `categorical` 结果必须是本次 `options` 中的一个 ID |
| `decision.refinement` | 仅返回选中选项定义的 `ordinal` 结果；该选项没有定义时省略 |
| `assessment` | 可选；包含有限 `[0,1]` 的 `score`、必填 `partial`，以及可选的 1–1024 个 Unicode 标量的 `reason` |

`assessment_target: null` 时省略 `assessment`。有效零分必须保留；没有评分不等于零分，也不会删除已保存观测。

| 情况 | HiRoute 的处理 |
| --- | --- |
| `choice` 不在允许集合，或整体响应无效 | 当前决策失败，走下文的计划兜底 |
| 智能省钱的 `ordinal` 概率缺失或非法 | 使用主力组，记录复杂度不可用 |
| 类别合法，但所需 `refinement` 缺失或非法 | 保留类别，使用该类别主力组，记录复杂度不可用 |
| 可选评分缺失、非法或无对应目标 | 不使用该评分；合法当前决策仍可用 |
| 评分 `partial: true` | 记录部分证据，不能触发低分保护 |

如果上游一次回答多个类别的复杂度，只转换被选中类别的答案。未选中答案有误，不应连带废弃有效的选中路径。

## HiRoute 如何执行判断结果

有两组模型时，本次 `P(simple)` 达到配置阈值（默认 **0.8**），且没有适用的本次评分低于胜任下限（默认 **0.5**），才选择常规或省钱组；否则选择主力。低分保护要求评分完整有效，且上一实际阶段与本次的类别、已发布计划版本、评分标准一致。旧评分不会在后续请求中作为新低分反复使用。单组类别只记录胜任度，没有可升级的主力组。

每条新用户消息重新决策。工具续接和同轮重放在可识别为同轮、且冻结决策仍可复用时继承它；历史不连续或无法继承时重新判断。扩展不用靠比较用户文本是否变化来决定是否处理请求。

选定组后，HiRoute 按顺序尝试合格模型。常规组耗尽可接力同类别主力组；直接选择主力时只在主力组内接力。候选故障不会制造胜任度零分。整体决策失败时，智能省钱使用启发式规则；自定义路由使用默认类别，有主力组则选择主力。

## 上下文、超时与调用边界

扩展负责上游集成和上下文准备。保留完整当前用户输入和传入的判断条件、评分标准。需要裁剪时，从最旧的完整历史轮次开始移除，并调整传给上游的目标下标；是否影响评分按 `assessment_target` 的实际范围判断。HiRoute 还会合并自身已知的目标证据缺口，扩展的 `partial: false` 不能抵消这些事实。

官方 Jev 扩展默认将完整供应商请求限制为 **256 KiB**，可通过部署配置调整；HiRoute 内置适配器也使用 256 KiB 预算。这是供应商请求的字节预算，不是本 API 要求所有自建扩展采用的 token 或上下文上限。当前输入与问题本身放不下时拒绝请求，不截断它们。执行模型的上下文仍由路由计划配置。

HiRoute 对扩展的生产调用只有一次，无自动后台重试，不跟随重定向。总 deadline 覆盖准备、认证读取、连接和响应读取；扩展自身预算应小于 HiRoute 连接超时。扩展不可用或请求失败会进入上述决策兜底；源请求取消、源 deadline 或当前输入完整性错误则直接终止请求。

完整类型见 [OpenAPI](decision.openapi.json)，可复制数据见[规范示例](decision-examples.json)，更多背景见[协议设计](decision-design.md)。当前运行时支持 `ordinal` 和 `categorical`；设计中的 `subset` 工具精选尚未开放。
