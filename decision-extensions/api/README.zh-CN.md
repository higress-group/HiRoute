# 自定义扩展 API

[English](README.md) · [决策模型与路由机制](../README.zh-CN.md) · [下载 OpenAPI](decision.openapi.json) · [完整示例](decision-examples.json)

自定义扩展是你部署的 HTTP 决策服务。每个新用户轮次，HiRoute 发送已发布的类别、条件和可用模型组，以及安全的用户内容和历史。扩展直接返回任务类别和 `regular`（常规）或 `primary`（主力）组；HiRoute 验证结果并执行候选模型。可部署[官方 Jev 扩展](../extensions/jev-decider/README.zh-CN.md)，也可使用自有模型或规则。

## 连接

在“模型 → 决策模型”新增自定义扩展，填写完整 endpoint、超时和可选认证。保存并测试后，在路由中选择这个已保存版本并发布。HiRoute 向完整地址 POST，不拼接路径。参考实现使用 `/v1/decisions`。保存测试不调用执行模型，也不证明判断质量。

## 请求

五个顶层字段均必需：`route`、`latest_user`、`visible_conversation`、`history_partial` 和 `preceding_stage`。

`route.kind` 为 `smart_saving` 时有固定 `groups: ["regular", "primary"]`。为 `branches` 时，`categories` 列出 2–16 个 `{id, condition, groups}`，`default_category` 必须指向其中一个。每类都有常规组，只有配置了主力候选的类别才带主力组。扩展不能选择未发布类别、具体模型、候选序号或工具。

`latest_user` 是当前用户内容数组。`visible_conversation` 中每项为 `{user,status,steps}`；status 为 `completed`、`failed`、`interrupted` 或 `unknown`。内容可为 `{kind:"text",text}` 或 `{kind:"unavailable",source_kind}`；steps 还可包含 `{kind:"tool_activity",tool,status}`，工具状态为 `completed`、`failed` 或 `unknown`。不发送系统/开发者指令、凭据、工具参数或工具结果正文。未知状态不代表失败。

`preceding_stage` 为空，或包含 `{from,category,group,complete}`。from 是历史中前一实际阶段的起始下标，group 是实际执行的组，complete 表示该阶段记录是否完整。HiRoute 在本地验证同一计划版本和连接版本、类别存在、实际归属唯一；内部阶段/模型标识不出站。请求携带前段真实类别；如果响应选择另一个类别，其 assessment 会被丢弃。`history_partial` 与目标阶段是否完整分别记录，不能把缺少早期前缀直接当作当前目标不完整。

HiRoute 不向扩展发送程度提示词、阈值、评分量表或评分锚点。扩展自己的部署配置决定策略和评分方法。

## 响应与错误

响应可包含 `category`、`group`、`reason` 和 `assessment`。分支路由必须返回已发布 category；按难度分流省略 category。双组范围必须返回 group；单组类别忽略 group，始终执行常规组。reason 可省略，存在时为 1–1024 个字符。assessment 可省略，存在时只能为 `{score,partial}`，score 必须是 0–1 有限数，partial 为布尔值。评分只用于观察，不影响 HiRoute 的路由选择。

返回 HTTP 200 和严格 JSON，不允许重复键或未知成员，响应不超过 64 KiB。合法类别缺少或返回无效 group 时保留类别并使用其主力组，记录“扩展未提供有效模型组”。无效 assessment 单独丢弃。类别无效、旧格式、未知字段、传输失败等整体失败时，按难度分流使用启发式规则；分支路由使用默认类别的主力组，没有主力则使用常规组。源请求取消、总期限或回放完整性失败直接终止，不兜底。

## 当前轮次与兼容

每个新用户轮次重新决策。同一轮工具继续和回放在历史及权限仍有效时继承冻结结果。HiRoute 保留同组已成功候选的优先选择和常规到主力的可用性接力；直接选主力不会降回常规。模型故障不会制造评分。

当前协议原位替换之前的 v1 概率响应，不提供旧格式读取或版本选择。自定义扩展仅支持不含判断设置的当前执行格式，不提供旧自定义扩展数据适配。当前连接字段不变，内置服务已发布数据与共用观察历史保留既有读取支持；未升级扩展会明确测试失败，并按上述执行兜底。草稿保留内置连接的判断设置，但扩展路由不展示或发布这些执行设置。

## 示例

### smart_saving

请求：

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

响应：

```json
{
  "group": "regular"
}
```

### writing_review

请求：

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

响应：

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

请求：

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

响应：

```json
{
  "group": "regular",
  "category": "review"
}
```
