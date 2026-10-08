# 自定义扩展 API v1

[English](README.md) · [决策模型与路由机制](../README.zh-CN.md) · [下载 OpenAPI 3.1](decision.openapi.json)

本 API 用于实现 HiRoute 的 HTTP 决策扩展。内置决策模型直接调用供应商的 System One API，请求结构不同，见[供应商映射](system-one-design.md)。部署扩展是可选项。

HiRoute 向自定义连接中保存的完整接入点发送一次 `POST`，不会自行追加路径。官方扩展提供 **`POST /v1/decisions`**。一次调用可以判断当前任务，同时评估上一实际执行阶段。扩展遵守传入的定义和冻结评分标准；HiRoute 负责阈值、模型组、候选选择与可用性接力。

## 请求契约

顶层恰有五个字段：

| 字段 | 含义 |
| --- | --- |
| `decision` | 有序程度定义，或任务类别及可选程度定义 |
| `latest_user` | 完整当前用户内容，允许与先前文本重复 |
| `visible_conversation` | 按原顺序保留的已封存执行轮次：`{user,status,steps}` |
| `history_partial` | HiRoute 已知的可见历史缺口 |
| `assessment_target` | `null`，或 `{from,instructions,criteria}`，指定上一实际阶段 |

**有序程度**定义包含 `kind`、`instructions` 和有序的 `levels:[{id,criterion}]`。level ID 唯一，数组顺序表示程度从低到高；当前模型路由使用 `simple`、`complex` 两档。

**任务类别**定义包含 `kind`、`instructions` 和 `options:[{id,criterion,refinement?}]`。自定义路由有 2–16 个任务类别。有主力模型的类别附一个 ordinal `refinement`；只有常规模型的类别不附。按当前请求的主要意图和传入的条件选择，遵守定义中的条件重叠与默认类别规则。类别条件描述任务是什么，程度条件描述该类别内的工作要求。

instructions 和 criterion 为非空文本，**不设逐字段字符上限**。ID 和外围结构保留 OpenAPI 中的约束。整体传输预算与供应商上下文限制仍适用，不能因此静默截断条件或评分标准。

### 可见历史与评分目标

历史轮次包含 `user` 内容、`status` 和 `steps`。轮次状态为 `completed`、`failed`、`interrupted` 或 `unknown`；steps 是内容数组的数组，每个 step 对应一次业务模型请求。内容包括已接受文本、带显式粗粒度状态的工具名称，或不可用内容标记；不含凭据、system/developer 指令、工具参数或工具结果正文。

`assessment_target.from` 是 `visible_conversation` 的零基下标，从该下标到末尾属于同一个上一实际阶段。`instructions` 和三项 `{score,criterion}` 按顺序固定为 **0、0.5、1**，使用该阶段执行时冻结的标准，不能被部署端固定规则替代。HiRoute 在本地绑定实际阶段，扩展无需返回内部模型或执行身份。

`history_partial` 描述整段可见历史的缺口，评分的 `partial` 描述目标阶段内的证据缺失。更早的消息未采集，本身不会使后续完整采集的目标阶段成为部分证据。目标为 `null` 时省略 assessment。重复用户文本、摘要或继续消息本身不是失败证据。

## 完整示例

下列请求和响应来自[规范示例](decision-examples.json)，数值用于说明，不是供应商实测结果。两种语言使用相同 JSON，保持示例 ID 与条件一致。

### 智能省钱：首次决策，无历史评分

请求：

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

响应：

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

没有上一阶段，`assessment_target` 为 `null`，响应不含 `assessment`。默认阈值下，该概率分布选择省钱组。

### 写稿与审稿：类别、选中类别程度及上一阶段评分

请求：

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

当前选择 `review`，响应只返回审稿的程度。**0.35** 分属于上一**写稿**阶段，不评价尚未发生的审稿，也不能把写稿低分用于审稿类别的保护。HiRoute 根据审稿本次程度选择模型组。

### 只有常规模型的类别

请求：

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

响应：

```json
{
  "decision": {
    "kind": "categorical",
    "choice": "review"
  }
}
```

review 选项没有 `refinement`，响应也不附 refinement；HiRoute 使用该类别的常规组。writing 仍可有两组模型，不因此要求审稿判断程度。后续请求也可以评估单组类别的上一阶段。

## 响应校验

返回 HTTP **200** 和一个纯 JSON 对象，最大 **64 KiB**。Markdown、重复或未知字段、原生供应商 envelope 都无效。

有序结果使用本次准确的 level ID，覆盖全部档位，返回有限 `[0,1]` 概率，总和与 1 的误差不超过 `1e-6`；HiRoute 校验后归一化这部分浮点误差。分类结果返回允许的 `choice`，有程度定义时只返回该选项的 `refinement`。扩展不能返回模型 ID、候选顺序或路由阈值。

可选 `assessment` 包含有限 `[0,1]` 的 `score`、必填 `partial`，以及可选的 1–1024 个 Unicode 标量 `reason`。有效零分必须保留；缺失、非法或部分评分不是零分，不能驱动低分保护。省略评分不会删除已保存观测；旧评分不会被当作本次新评分复用。

非法类别使当前决策无效。类别有效但选中程度非法时，保留该类别、使用它的主力组并记录程度不可用。非法可选评分独立丢弃，不影响合法当前决策。供应商未选中的程度答案错误，不应污染选中路径。

## HiRoute 如何使用结果

有两组模型时，本次 `P(simple)` 达到配置阈值（默认 **0.8**），且没有适用的本次评分低于胜任下限（默认 **0.5**），才选择常规或省钱组；否则选择主力。低分保护要求上一实际阶段的完整有效评分，与本次类别、已发布计划版本和评分标准一致。单组类别记录胜任度，不提供升级组。

每条新用户消息重新决策，可能选择任一组。工具续接和同轮重放只有在 HiRoute 可识别为同轮且冻结决策可复用时，才继承该决策；历史重建不连续或决策无法继承时仍会重新判断。可用性接力为常规 → 同类别主力 → 失败；直接选主力时只在主力组内接力。候选故障不会制造胜任度零分。

整体决策失败时，智能省钱使用启发式规则；自定义使用默认类别，有主力组则选择主力。源取消、源 deadline 和当前输入完整性错误直接终止请求。

## 供应商集成与请求限制

扩展负责供应商集成和上下文准备。完整当前用户输入不可截断。为适配供应商限制，可以从最旧完整历史轮次开始裁剪，并调整保留历史中的目标下标。目标证据被裁剪时返回 `partial:true`；目标全部移除时省略评分。HiRoute 还会合并自身已知的目标证据缺口，扩展的 `partial:false` 不能抵消这些事实。

内置适配器限制完整供应商请求为 **256 KiB**，包括问题和 state。官方 Jev 扩展默认使用相同预算，并允许部署配置。这是字节分配预算，不是 token 数或供应商上下文上限。当前输入与问题本身超限时拒绝请求，不截断它们。业务模型上下文仍由路由计划设置决定。

HiRoute 的总 deadline 覆盖准备、认证读取、连接和响应读取。生产传输只调用一次，无自动后台重试，禁止跳转。扩展自身预算应能容纳在 HiRoute 连接超时内。

[协议设计](decision-design.md)描述当前 v1 边界；[System One 映射](system-one-design.md)描述供应商 Choice/Score 调用。工具精选仅保留未来文档，当前路由与扩展 API 运行时不支持。
