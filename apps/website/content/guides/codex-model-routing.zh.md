# Codex 多模型路由：复用订阅和 API，按难度选模型

在 Codex 里继续用熟悉的工具，把读代码和小改动交给经济模型，复杂调试交给主力。用 HiRoute 的按需 profile 先在 Codex CLI 试一条路由，再决定是否给日常配置启用。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/codex-model-routing/
请复用我的 Codex 模型来源并配置按需路由。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 查找 Codex 接入和配置目标：hiroute compute scan · compute connection options · quickstart
- 创建难度路由并映射 Codex：routing preview / apply · agents connect preview / apply / status
- 选择另一个执行方并核对任务：worker executors · worker plans · worker result
界面和 CLI 共用配置和记录；先预览按需接入，参数以 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。
```

</details>
</aside>

## 1. 找到 Codex 已有的模型来源

打开“模型 → 通用模型 → 添加模型 → 扫描本机”。已有 Codex provider 配置时，选择识别出的接入，检查并保存可用模型。已有 Codex 订阅时，可以复用受支持的文件登录，或在“连接订阅”中独立登录。原生登录只存在系统钥匙串时，用独立登录。

如果只想补一个便宜的常规模型，从“新建 API 接入”添加它。你也可以使用 [Magpie 提供的 Qoder 模型](/zh/docs/qoder-model-routing/) 或 [已接入的本地模型](/zh/docs/models/)。

## 2. 建一条“Codex 日常编码”路由

在“智能路由 → 新建智能路由”选择“按难度分流”。常规组选便于处理文件查找、测试归纳和机械修改的模型，主力组保留你熟悉的 Codex 订阅模型或 API 模型。先用“启发式规则”，需要阶段评分与低分保护时再接入[决策模型](/zh/docs/model-routing/#difficulty-routing)。发布后再映射到 Codex。

打开“Agent → Codex → 模型路由”。选择“按需使用 · 仅 CLI”，勾选路由并设置默认，然后用“启用并复制启动命令”给出的命令启动 Codex。先让它找出一个模块的测试入口，再试一个需要跨文件分析的问题，回“会话 → 路由时间线”看实际模型。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-editor-zh.webp"><img src="/guide-assets/agent-editor-zh.webp" alt="Codex 模型路由对话框中的按需 CLI 接入与可用路由" width="1200" height="900" loading="lazy" /></a>
  <figcaption>先用按需 CLI profile 试用。日常 Codex 的入口和配置保持原样。</figcaption>
</figure>

### 在 Codex Desktop 使用

在 macOS 或 Windows 的 HiRoute 桌面应用里，可以选择单独的 **Codex Desktop** 条目，保存路由后“启动独立窗口”。如果选“设为默认 · CLI 和 Desktop”，共享配置目录的入口都会用新设置；早先会话按 provider 筛选，可从“查找原有会话”找回。

## 3. 保持 Codex 对话，让 DSH 做独立分析

用已接入的模型另建“DSH 分析”计划，按同样方式设置常规组和主力组。打开“更多设置 → 任务委派”，允许委派，写明用途并选择 **DeepSeek Harness**，按提示检查依赖，再发布。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-dsh-worker-zh.webp"><img src="/guide-assets/agent-guide-dsh-worker-zh.webp" alt="HiRoute 任务委派计划选择 DeepSeek Harness 作为执行 Agent" width="1200" height="900" loading="lazy" /></a>
  <figcaption>给 DSH 单独设一个用途，Codex 更容易把合适的任务交给它。</figcaption>
</figure>

在 Codex [开启任务路由](/zh/docs/task-routing/#delegation-skill)，然后说：“使用 HiRoute，把这次测试失败的原因分析委派给 DSH 分析，给出复现步骤和最小修复建议。”在“Agent → 任务记录”看执行结果。两条计划可以共用模型，委派也可以在 Codex 保留原有模型的情况下单独开启。

下一步：[配置 DeepSeek Harness](/zh/docs/deepseek-harness-model-routing/) · [查看模型接入选项](/zh/docs/models/)
