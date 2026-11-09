# WorkBuddy 模型接入 HiRoute：通过 Magpie headless 让 Pi 执行

已有 WorkBuddy 账号时，可以把账号可用的模型接进 HiRoute，让便于处理常规工作的模型先上，需要深度判断时用主力，再用 Pi 或 Codex CLI 的工具完成任务。WorkBuddy 提供模型，Pi 或 Codex CLI 是你在路由中选择的执行 Agent。

[下载 HiRoute](/zh/download/) · [查看接入总览](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/workbuddy-model-routing/
请按我的 WorkBuddy 账号目录复用 Magpie 接入，配置按难度分流并绑定我选的执行 Agent。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 读取和保存 API 接入：hiroute compute list · compute connection options / test / preview / apply
- 配置分流和执行器：routing options · routing preview / apply · worker executors
- 查看执行和模型：worker result · sessions timeline
界面和 CLI 共用配置和记录。由我完成 WorkBuddy 授权；只用实际目录里的模型。参数按 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 确认。
```

</details>
</aside>

<h2 id="connect-workbuddy">1. 通过 Magpie 读取 WorkBuddy 模型</h2>

[启动 Magpie headless 服务](/zh/docs/magpie-model-sources/#start-magpie)，或复用正在运行的实例。然后安装 [WorkBuddy 社区插件](https://github.com/magpie-community/plugins/blob/main/registry.json)并登录账号：

```sh
magpie plugin add @magpie-community/opencode-workbuddy-auth
magpie plugin login workbuddy
```

上面的 `workbuddy` 对应国内版（CodeBuddy 套餐）。国际版用 `magpie plugin login workbuddy-ai`。完成授权后[查询模型目录](/zh/docs/magpie-model-sources/#model-catalog)，记下账号可用的完整模型 ID。

用选中的 WorkBuddy 模型[在 HiRoute 保存 Magpie API](/zh/docs/magpie-model-sources/#save-api)。如果已经接入了同一个 Magpie 网关，可以把模型加到已有接入，然后继续配置路由。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-magpie-api-zh.webp"><img src="/guide-assets/agent-guide-magpie-api-zh.webp" alt="在 HiRoute 自定义 API 表单填写承载 WorkBuddy 模型的 Magpie 地址" width="1200" height="900" loading="lazy" /></a>
  <figcaption>将 Magpie 的本机地址添加到 HiRoute，选择账号目录中可用的模型。</figcaption>
</figure>

## 2. 配一条适合文档与代码的难度路由

新建“WorkBuddy 资料整理”计划，选“按难度分流”。常规组选适合查找文件、提取要点的模型，主力组选适合比较方案、形成结论的模型。账号只提供一个合适的模型时，可以从 [已接入的 API、订阅或本地模型](/zh/docs/models/) 补齐另一组。

先用“启发式规则”上手；希望根据执行表现调整时，按 [智能模型路由](/zh/docs/model-routing/#difficulty-routing) 配置决策模型。选能完成任务所需工具调用的模型，再发布。

## 3. 让 Pi 执行，或换成 Codex CLI

打开计划的“更多设置 → 任务委派”，允许委派，写下用途“读取项目中的文档并归纳要点”，选择 **Pi**，完成依赖检查并发布。需要用 Codex 的工具时，可以用同一组模型另建一份计划，执行 Agent 选 **Codex CLI**。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-pi-worker-zh.webp"><img src="/guide-assets/agent-guide-pi-worker-zh.webp" alt="在 HiRoute 的可委派计划中绑定 Pi 执行 Agent" width="1200" height="900" loading="lazy" /></a>
  <figcaption>在这里选择执行工具，计划上方的模型组决定怎样分流。</figcaption>
</figure>

在你平时对话的 Agent 上[开启任务路由](/zh/docs/task-routing/#delegation-skill)，例如让它“使用 HiRoute，把整理当前项目 README 的任务委派给 WorkBuddy 资料整理，只输出三条要点”。任务完成后查看“Agent → 任务记录”和会话的路由时间线。

下一步：[Qoder 订阅模型与 Magpie](/zh/docs/qoder-model-routing/) · [把路由加到 Pi 模型选择器](/zh/docs/pi-model-routing/)
