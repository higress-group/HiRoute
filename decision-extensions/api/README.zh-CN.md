# 决策 API

[English](README.md) · [机制与使用方式](../README.zh-CN.md)

[下载唯一当前 OpenAPI 3.1 JSON](decision.openapi.json)。Desktop 协议弹窗和“保存 OpenAPI”也使用该文件，不为扩展维护另一份副本。`branches` 是通用允许分支映射，协议并不限定二分类。

官方扩展和 OAS 统一使用 `POST /v1/decisions`。HiRoute 向计划配置的**完整 endpoint** 发请求，自建服务仍可以选择自己的路径。已有 Jev 部署更新到此版本时须同步修改计划 endpoint，官方服务不保留旧路径别名。

## 请求

| 必填字段 | 含义 |
| --- | --- |
| `branches` | 允许的分支 ID 及含义，响应必须选择其中一个 |
| `latest_user` | 从当前请求投影的完整非空用户 content parts；可以与上一执行轮次相同，也可以是摘要/继续消息 |
| `visible_conversation` | 按顺序保留的已封存路由执行轮次，含分支、轮次开始时的用户内容、状态和 steps |
| `history_partial` | 已保留历史是否存在缺口 |
| `assessment_from` | 可评分历史后缀的零基起始下标；没有可靠目标时为 `null` |

一个 step 对应一次业务模型请求，保留接受的文本、工具活动或不可用标记。工具活动仅含名称和 `completed/failed/unknown` 状态，不含参数和结果；状态来自协议显式事实，不通过自由文本猜测失败。一个路由执行轮次包含一次分支决策及其后继承该决策的模型请求，一个用户任务可以跨多个轮次。仅因进入下一决策边界而封存的轮次可以保持 `unknown`。轮次可以用 `executed_branch_id` 表达与选择分支不同的实际兜底分支。完整类型见 OAS。

```sh
curl --fail-with-body --request POST 'http://127.0.0.1:8080/v1/decisions' \
  --header 'Content-Type: application/json' \
  --data '{
    "branches": {
      "smart_saving_simple": "边界清晰的任务，使用省钱组合。",
      "smart_saving_complex": "需要深入推理的任务，使用主力组合。"
    },
    "latest_user": [{"kind":"text","text":"修复这条失败测试。"}],
    "visible_conversation": [],
    "history_partial": false,
    "assessment_from": null
  }'
```

请求官方扩展可能产生 OpenRouter 费用。如果启用了认证，补上所配置的 header；配置 HiRoute 时将完整 header 值放在 Secret 中。OpenRouter key 仅保存在外部服务侧。

## 响应与评分

上面的首轮请求没有评分目标，应只返回分支，例如 `{"branch_id":"smart_saving_simple"}`。后续请求有非空 `assessment_from` 时可以同时返回：

```json
{
  "branch_id": "smart_saving_complex",
  "assessment": {
    "score": 0.25,
    "partial": false,
    "reason": "多次失败尝试仍未解决上一阶段的问题。"
  }
}
```

`assessment` 可省略，其中 `reason` 也可省略。`score` 是 `[0,1]` 有限胜任度数值；`partial` 表示服务是否删掉了目标后缀中的证据。HiRoute 结合自身历史缺口，将评分绑定到请求快照中的真实执行阶段，不能把它记到刚选择、尚未执行的新分支。重复用户文本、摘要或继续消息本身不是正面或负面反馈。省略评分时保留旧值。服务不需要回传模型 ID、阶段 ID 或评分器版本。

返回单个纯 JSON 对象，不加供应商 envelope、Markdown、重复或未知字段。成功响应上限 64 KiB。非法分支使整个响应无效；合法分支搭配非法可选评分时仍使用分支，但丢弃评分。

## 上下文和失败处理

HiRoute 不设置额外固定输入字节限额，也不截断当前用户文本；8 KiB inline 阈值只是存储位置选择，更大内容从 ReplayStore 读取。内存 turn 历史有独立容量边界，可能不完整。决策服务根据自身模型限制裁剪；删减了评分目标应返回 `partial: true`，目标内容全被移除则省略评分。

计划 `timeout_ms` 覆盖历史准备、凭据、连接和响应读取，同时受源请求取消与 deadline 限制。默认 3000 ms，官方服务默认总预算 2.8 秒；服务预算应略小于计划值。HiRoute 只调用一次，不自动重试。外部服务失败回退本地规则；源取消/deadline、本地 Replay 完整性或资源错误直接终止，不伪装成服务失败继续执行。

当前智能省钱配置与具体工具状态映射见[实现使用说明](../../docs/smart-saving-model-classification.zh-CN.md)。
