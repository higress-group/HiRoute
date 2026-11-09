# Hermes Agent 模型路由：复用 profile，让 Codex 执行编码

继续在 Hermes 聊天和协调工作。接入当前 profile 的可用模型，简单请求用常规组，需要深入推理的对话用主力；需要实现一项独立的代码改动时，再交给 Codex CLI。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/hermes-model-routing/
请按我的 Hermes 实际 profile 复用模型、配置难度分流，并准备单独的 Codex 计划。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 确认 profile 和可用接入：hiroute quickstart --hermes-root PATH · compute scan · compute connection prepare
- 发布路由并设置 Hermes 主模型：routing preview / apply · agents connect preview / apply
- 跟进 Codex 执行：worker executors · worker plans · worker result
界面和 CLI 共用配置和记录。先确认所选配置目录，参数以 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。
```

</details>
</aside>

## 1. 确认 Hermes 正在用的 profile

在“Agent → Hermes”确认配置目录。若有多个 profile，选择你日常对话实际使用的一个。到“模型 → 通用模型 → 添加模型 → 扫描本机”，需要时指定相同的 Hermes 配置目录，检查并保存识别到的 API 模型。

未发现想要的模型时，可直接添加 API，或使用已接入的来源。例如把 [Qoder 订阅模型](/zh/docs/qoder-model-routing/) 和你熟悉的主力模型组合起来。

## 2. 将难度路由设为 Hermes 默认主模型

创建“Hermes 日常对话”，选“按难度分流”。让常规组处理简短问答与资料提取，让主力组处理多步计划。选择“启发式规则”，或配置[决策模型](/zh/docs/model-routing/#difficulty-routing)，发布计划。

回到“Agent → Hermes → 模型路由”，选这条路由作为所选 profile 的默认主模型。保存后按提示重新加载 Hermes；已有会话如果保留其他模型选择，试一个新会话。发一条消息，在 HiRoute 会话中查看路由和实际模型。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-hermes-zh.webp"><img src="/guide-assets/agent-guide-hermes-zh.webp" alt="Hermes 模型路由设置显示配置目录与默认主模型计划" width="1200" height="900" loading="lazy" /></a>
  <figcaption>先确认配置目录，再设置这个 Hermes profile 的默认主模型。</figcaption>
</figure>

## 3. 在 Hermes 对话中把代码工作交给 Codex

新建“Codex 编码”计划，可以复用同样的模型，也可以给编码单独选常规与主力模型。进入“更多设置 → 任务委派”，选择 **Codex CLI**，写下用途“实现范围明确的修改并运行相关测试”，检查依赖、发布。

<figure class="guide-shot-panel">
  <a href="/guide-assets/routing-worker-zh.webp"><img src="/guide-assets/routing-worker-zh.webp" alt="HiRoute 的任务委派配置中选择 Codex CLI 作为执行 Agent" width="1200" height="900" loading="lazy" /></a>
  <figcaption>在执行计划中选择 Codex CLI，Hermes 可以直接把合适的工作交给它。</figcaption>
</figure>

在当前 Hermes profile [开启任务路由](/zh/docs/task-routing/#delegation-skill)，试一句：“使用 HiRoute，把当前项目里这个小改动委派给 Codex 编码，完成后汇总修改与测试。”在“Agent → 任务记录”看 Codex 执行结果。只开启委派时，Hermes 可以继续用原来的主模型。

下一步：[设置 Codex CLI](/zh/docs/codex-model-routing/) · [查看 Hermes 与多执行 Agent 的案例](/zh/news/hermes-super-agent/)
