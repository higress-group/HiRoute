# OpenCode 多模型路由：复用 API 配置，在 /models 中选用

已有 OpenCode provider？把支持的接入复用到 HiRoute，让快速模型读代码、强模型做复杂分析。发布后在 OpenCode 的 `/models` 选用，原有模型仍在。需要第二种执行工具时，可以把独立检查交给 Pi。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/opencode-model-routing/
请帮我复用可识别的 OpenCode API 配置，再新增一条难度路由。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 发现和保存 API 来源：hiroute compute scan · compute connection prepare / preview / apply
- 创建路由并加入 OpenCode：routing preview / apply · agents connect preview / apply
- 核对模型与另一执行方：sessions timeline · worker plans · worker result
界面和 CLI 共用配置和记录。先读取现状，参数按 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 确认。
```

</details>
</aside>

## 1. 扫描 OpenCode 中已有的 API

进入“模型 → 通用模型 → 添加模型 → 扫描本机”。选择可识别的 OpenCode 静态 provider、模型与 API Key 来源，检查并保存。OpenCode 自己的 `/models` 也可能包含动态发现或 OAuth 插件提供的模型；如果扫描没找到，按 [接入模型](/zh/docs/models/) 连接对应 API，或选已有的订阅与本地模型。

## 2. 新增一条 OpenCode 难度路由

创建“OpenCode 代码审查”计划，选“按难度分流”。常规组收集差异、定位相关文件；主力组评估改动影响和边界条件。选择“启发式规则”开始，或用[决策模型](/zh/docs/model-routing/#difficulty-routing)给阶段表现打分。发布。

进入“Agent → OpenCode → 模型路由”，勾选刚发布的计划，按提示选兼容协议并保存。重启 OpenCode，在 `/models` 中选新增路由。原有 provider 和默认模型保留。试着让 OpenCode 总结一个小改动，然后看 HiRoute “会话 → 路由时间线”。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-opencode-zh.webp"><img src="/guide-assets/agent-guide-opencode-zh.webp" alt="OpenCode 模型路由设置中选择新增的 HiRoute 计划" width="1200" height="900" loading="lazy" /></a>
  <figcaption>路由作为一个新选项加入 OpenCode，可以在原生模型选择器中按需使用。</figcaption>
</figure>

## 3. 让 Pi 做独立检查

将复用的模型放入“Pi 独立检查”计划。设置常规组和主力组后，在“更多设置 → 任务委派”开启委派，选择 **Pi**，填好用途并检查依赖。发布，在 OpenCode [开启任务路由](/zh/docs/task-routing/#delegation-skill)。

试一句：“使用 HiRoute，把这份 diff 的测试缺口检查委派给 Pi 独立检查，只汇报发现。”打开“Agent → 任务记录”查看执行方和结果。如果希望 OpenCode 自己被其他 Agent 调用，在另一条可委派计划中选择 OpenCode。

下一步：[准备 Pi 作为执行 Agent](/zh/docs/pi-model-routing/) · [选择其他接入教程](/zh/docs/agent-integrations/)
