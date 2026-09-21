# 智能省钱自定义分类服务

[English](smart-saving-model-classification.md)

智能省钱默认使用 HiRoute 内置的确定性规则。需要自定义策略时，在计划编辑器选择“自定义分类服务”，填写服务地址；认证可留空，也可选择一个 header 名称并创建或引用 Secret。服务可基于 Jev、LLM 或其他自定义策略实现；HiRoute 只消费本文的 HTTP JSON 合同，不直接调用或解析供应商协议。

官方可部署参考实现位于 [`decision-extensions/extensions/jev-decider`](../decision-extensions/extensions/jev-decider/README.zh-CN.md)。它演示用一次 OpenRouter Jev Decisions 请求同时完成本轮 branch 选择和上一执行阶段的可选胜任度评分，也可作为自定义策略的起点。

Desktop 的“查看接入协议”弹窗提供可复制的 `curl` 请求和响应示例，并可下载单一当前 [OpenAPI 3.1 文档](../decision-extensions/api/decision.openapi.json)。弹窗中的地址只是示例；运行时始终向 AgentPlan 配置的完整 endpoint 发起请求。

## 配置

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

- `endpoint` 接受操作者授信的完整 HTTP/HTTPS URL，不允许 userinfo 或 fragment，也不跟随 redirect。
- `timeout_ms` 是整个分类链路的毫秒期限，合法范围 `1..=3_600_000`；Desktop 初值为 `3000`。有效 deadline 还会受源请求 deadline 限制。
- `auth_header` 可省略。Secret 保存完整 header 值，例如 `Bearer ...`；HiRoute 不自动补 scheme。
- 计划不保存明文 Secret，不配置分类 instructions、计划用途或可编辑 branch descriptions。
- “测试决策”显式发送固定合成首轮请求，使用与生产相同的 Secret、HTTP、timeout_ms 和响应校验。保存与发布不会自动调用服务；测试可能产生外部费用，也不会产生质量样本。
- 外置服务自己的总超时应略小于 timeout_ms，为 HiRoute 序列化和网络收尾留余量。官方 Jev 决策器通过 `JEV_REQUEST_TIMEOUT_SECONDS` 设置，默认 `2.8` 秒。

## 请求合同

每个新的 Agent turn 最多发一个 `POST`。同一 turn 的工具续轮继承已冻结分支，不再调用决策器。

```json
{
  "branches": {
    "smart_saving_simple": "Use the economy model group for a clear, well-scoped task.",
    "smart_saving_complex": "Use the primary model group for an ambiguous, cross-module, diagnostic, concurrent, or deep-reasoning task."
  },
  "latest_user": [{"kind": "text", "text": "修复这条失败测试。"}],
  "visible_conversation": [{
    "branch_id": "smart_saving_simple",
    "user": [{"kind": "text", "text": "先修复类型错误。"}],
    "status": "completed",
    "steps": [
      [{"kind": "tool_activity", "tool": "functions.run_tests", "status": "failed"}],
      [{"kind": "text", "text": "已修复并重新验证。"}]
    ]
  }],
  "history_partial": false,
  "assessment_from": 0
}
```

顶层恰好五个字段：

- `branches`：本次允许的 branch ID 和 HiRoute 内置说明。服务只能返回其中一个 ID。
- `latest_user`：本轮完整用户 content parts。服务可以只看它。
- `visible_conversation`：内存中已结束的 Agent turns。每个 step 是一次业务模型请求，只保留回答文本和工具名称、顺序、粗粒度状态。
- `history_partial`：重启、TTL、LRU 或捕获缺口导致历史不完整时为 true。
- `assessment_from`：需要评价的上一连续执行阶段在数组中的起始下标；null 表示没有可靠评分目标。

工具状态只来自入站协议的显式事实：Messages 使用 `is_error`，Responses function output 使用 `completed/incomplete/in_progress`，provider 原生 web search 使用其明确终态；Chat tool result、Responses custom output 或缺省 Responses function status 均为 `unknown`。HiRoute 不解析工具输出正文猜测失败，因此 Chat Agent 可以在结果内容中告知业务模型错误，但分类历史不会把该自由文本冒充结构化失败。

协议不发送 system/developer、reasoning、工具参数和结果、provider state、凭据、计划用途、plan/model/session 内部 ID 或逐 step 模型。若只有实际兜底分支产生被接受输出，turn 会额外包含 `executed_branch_id`；混合模型贡献不会成为单模型评分目标。

HiRoute 不给分类请求设置额外字节上限，也不截断 `latest_user` 或文本块。超过 8 KiB 的字段从同请求 ReplayStore 流式读取；8 KiB 是存放位置阈值，不是 REST 协议限额。决策服务若有 32K token 等模型限制，应在服务内部按自己的 tokenizer 和策略裁剪完整 turns，并正确返回 partial。

## 响应合同

最小成功响应：

```json
{"branch_id":"smart_saving_complex"}
```

带上一阶段胜任度：

```json
{
  "branch_id": "smart_saving_complex",
  "assessment": {
    "score": 0.25,
    "partial": false,
    "reason": "可选的可见行为说明"
  }
}
```

- `branch_id` 必填且必须属于请求的 `branches`。
- `assessment` 可省略；省略表示不更新评分，不是 0 分。
- `score` 是 `[0,1]` 的模型胜任度，不是置信度、成功概率或问题复杂度。
- `partial` 必填，表示服务是否删减了被评分区间。
- `reason` 可省略；Jev 没有文字 reason 时无需拼造。

合法 branch 配非法 assessment 时，HiRoute 仍使用 branch、丢弃评分；branch 非法时整次响应无效，评分也不会保存。成功正文上限 64 KiB，不接受未知字段、重复字段、多个对象、Markdown 或供应商 envelope。

## 阶段评分和查询

HiRoute 按连续的计划版本、选择/实际分支、实际模型配置和有效 profile 识别执行阶段。Key 轮换不切阶段；实际模型、profile、分支或计划版本变化在真实执行后开启新阶段。服务每轮都可返回上一阶段评分，也可以省略；合法新评分覆盖该阶段最新值，不为每轮创建独立分数。

Desktop 的会话“模型表现”和计划编辑“运行表现”读取同一数据。计划页默认列出当前生效版本已经选择的模型，并按所选时间范围展示这些模型的阶段胜任度；用户无需输入内部模型 ID。CLI 仍可按计划或会话查询，并组合版本、精确模型、时间、严格大于和严格小于筛选，例如：

```sh
hiroute observation plan-quality samples \
  --plan-id plan/codex-daily \
  --model model/config-a \
  --score-lt 0.5
```

未评分样本不会被当作 0；`score_lt 0.5` 不包含等于 0.5。评分用于人工或获准主 Agent 分析某模型在特定 AgentPlan 下是否胜任，以及是否值得拆出更专门的场景；HiRoute 不会自动修改或新建计划。

## 失败边界

- 历史准备、Secret 解析、DNS/连接和 HTTP 读写共享 timeout_ms，并同时受源请求 deadline 和取消控制。
- 服务每轮最多调用一次，不重试，不裁剪后重试。
- 外部超时、不可用、拒绝输入或非法输出使用一次本地规则兜底并记录结构化原因。
- 源取消、源 deadline、Replay 完整性或本地资源错误直接终止，不能伪装成 REST 失败后继续。
- Observation 写入失败不阻塞模型回答；只有真正持久化的评分才会出现在查询中。
