# Pi Coding Agent 多模型路由：复用已有模型，按难度分流

在 Pi 的 `/model` 中选一条 HiRoute 路由，继续用熟悉的工具和工作流。读文件、找定义等常规工作用经济模型，需要跨模块推理时用主力；也能让 Codex CLI 独立验证结果。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/pi-model-routing/
请帮我复用 Pi 已有的模型来源，添加一条可在 Pi 中选用的难度路由。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 扫描和检查可复用配置：hiroute compute scan · compute connection prepare / preview / apply
- 配置计划并接入 Pi：routing preview / apply · agents connect preview / apply
- 准备与核对独立执行：worker executors · worker plans · worker result
界面和 CLI 共用配置和记录；参数以 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。
```

</details>
</aside>

## 1. 从 Pi 的模型配置开始

在“模型 → 通用模型 → 添加模型 → 扫描本机”查看 Pi 来源。HiRoute 会读取支持的 `models.json` 声明、API Key 来源，以及可识别的已安装 Pi 模型目录。选中接入，检查并保存你要用的模型。插件运行时才生成的模型，可以用对应 API 单独添加。

你也可以直接选已接入的订阅、API 或本地模型。例如 [通过 Magpie 导入 Qoder 模型](/zh/docs/qoder-model-routing/)，把这套模型带进 Pi。

## 2. 在 Pi 的模型选择器加入难度路由

新建“Pi 日常开发”计划，选“按难度分流”。常规组处理查找符号、收集代码片段等小步骤，主力组处理设计修改和难以复现的问题。用“启发式规则”即可开始，想按阶段表现调整时再加[决策模型](/zh/docs/model-routing/#difficulty-routing)。

发布后，到“Agent → Pi → 模型路由”添加这条路由，按候选模型和界面建议选择协议。重启 Pi，用 `/model` 选新增路由。原有模型仍可从选择器使用。做一个小任务，在 HiRoute 会话时间线检查选模。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-pi-zh.webp"><img src="/guide-assets/agent-guide-pi-zh.webp" alt="Pi 模型路由设置中的可用路由和接口协议" width="1200" height="900" loading="lazy" /></a>
  <figcaption>在 Pi 的配置里勾选要新增的路由，重启后到 /model 选择。</figcaption>
</figure>

## 3. 让 Codex CLI 单独验证

用同一批模型新建“Codex 回归检查”计划。展开“更多设置 → 任务委派”，选择 **Codex CLI**，将用途写成“复现问题并运行相关测试”。依赖检查通过后发布，在 Pi [开启任务路由](/zh/docs/task-routing/#delegation-skill)。

在 Pi 中说：“使用 HiRoute，把这个模块的回归验证委派给 Codex 回归检查，只运行相关测试并汇总。”在任务记录中核对执行方和测试结果。反过来，若想让其他主 Agent 使用 Pi 的执行工具，就在可委派计划里选 Pi，检查 Pi CLI 和 Node。

下一步：[配置 Codex CLI 路由](/zh/docs/codex-model-routing/) · [了解任务记录与继续执行](/zh/docs/task-routing/)
