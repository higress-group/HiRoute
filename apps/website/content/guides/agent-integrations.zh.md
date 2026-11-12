# 按 Agent 接入 HiRoute

继续用你熟悉的 Agent，把已有订阅、API 或本地模型组合成一条路由。让经济模型处理读代码、整理资料等常规工作，难点交给主力；需要另一种工具时，再把独立任务交给合适的执行 Agent。

[下载 HiRoute](/zh/download/) · [第一次使用 HiRoute](/zh/docs/)

## 选你常用的 Agent

每篇教程都按四步展开：获取模型、配置智能路由、接入 Agent、了解任务委派。路由配置步骤和截图直接放在各篇文档中，跟着一篇就能完成接入。

<ul class="integration-catalog">
  <li><a href="/zh/docs/codex-model-routing/">Codex 订阅和 API 赠金路由</a><p>组合订阅与赠金候选，再打开 Codex Desktop 独立窗口或接入 CLI。</p></li>
  <li><a href="/zh/docs/claude-code-model-routing/">Claude Code 订阅和 API 赠金路由</a><p>给同名候选设置备用顺序，再映射到 Opus、Sonnet 或 Haiku 预设。</p></li>
  <li><a href="/zh/docs/qoder-model-routing/">Qoder CLI 路由与订阅模型</a><p>用 Magpie 发现模型，配置分流，再接入 Qoder CLI 或 Codex Desktop。</p></li>
  <li><a href="/zh/docs/pi-model-routing/">Pi 多模型路由</a><p>导入 Pi 的模型，在 <code>/model</code> 里选路由，再交给 Codex CLI 独立验证。</p></li>
  <li><a href="/zh/docs/deepseek-harness-model-routing/">DeepSeek Harness 模型路由</a><p>在 Web 或桌面版选用路由，也可以让 DeepSeek Harness 执行委派任务。</p></li>
  <li><a href="/zh/docs/opencode-model-routing/">OpenCode 多模型路由</a><p>复用已有 API 配置，在 <code>/models</code> 里选路由，再让 Pi 独立检查。</p></li>
  <li><a href="/zh/docs/hermes-model-routing/">Hermes Agent 模型路由</a><p>复用当前 Hermes profile 的模型，把独立编码任务交给 Codex CLI。</p></li>
</ul>

## 接入更多模型来源

<ul class="integration-catalog">
  <li><a href="/zh/docs/workbuddy-model-routing/">WorkBuddy 模型接入 Pi 或 Codex CLI</a><p>接入 WorkBuddy 账号目录，配一条适合整理资料的路由。</p></li>
  <li><a href="/zh/docs/magpie-model-sources/">Magpie headless 模型接入</a><p>启动本地网关，查询模型目录，再把模型接入 HiRoute。</p></li>
</ul>

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/agent-integrations/
请按我现有的模型来源和常用 Agent 选择接入教程。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 读取已有接入：hiroute quickstart · compute list · compute scan
- 发布路由并连接 Agent：routing preview / apply · agents connect preview / apply
- 查看实际模型和执行方：sessions timeline · worker plans · worker result
界面和 CLI 共用配置和记录；先复用现有接入，以 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。
```

</details>
</aside>

## 从模型来源到执行 Agent

1. **接入模型。** 从“模型 → 通用模型 → 添加模型”复用订阅、扫描本机受支持的配置或添加 API。Magpie 已经在运行时，可直接[添加本地 API](/zh/docs/magpie-model-sources/#save-api)。
2. **按难度分流。** 在“智能路由”选择“按难度分流”，添加常规模型与主力模型。第一次可以用启发式规则；需要阶段评分和低分保护，再接入决策模型。见 [智能模型路由](/zh/docs/model-routing/#difficulty-routing)。
3. **接入 Agent。** 把发布后的路由映射到你日常使用的 Agent，再从该客户端的模型入口选用。
4. **按需委派任务。** 在计划中绑定执行 Agent，并在主 Agent 中开启“任务路由”。日志初筛和测试检查适合交给[轻量 Pi 委派计划](/zh/docs/task-routing/#lightweight-pi)。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-zh.webp"><img src="/guide-assets/agent-guide-difficulty-zh.webp" alt="HiRoute 按难度分流计划中的常规模型与主力模型组" width="1200" height="900" loading="lazy" /></a>
  <figcaption>把容易的工作分给常规模型，复杂轮次交给主力模型。每组都可以添加多个候选。</figcaption>
</figure>

一次从小任务开始。模型怎么选，去“会话 → 路由时间线”看；哪种 Agent 执行、产出了什么，去“Agent → 任务记录”看。[查看任务委派步骤](/zh/docs/task-routing/)
