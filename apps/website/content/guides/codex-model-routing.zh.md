# Codex 订阅模型路由与 Codex Desktop 接入

把自己的 ChatGPT 订阅和 API 模型接入 HiRoute，让经济模型处理读代码和小改动，主力模型负责复杂调试。本教程带你完成模型导入、难度路由配置，以及 Codex Desktop 独立窗口或 Codex CLI 接入。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/codex-model-routing/
请帮我按本页完成获取模型、配置智能路由和接入 Agent，再介绍任务委派的适用场景，按我的需要选择启用。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 查找 Codex 接入和配置目标：hiroute compute scan · compute connection options · quickstart
- 创建难度路由并映射 Codex：routing preview / apply · agents connect preview / apply / status
- 选择另一个执行方并核对任务：worker executors · worker plans · worker result
界面和 CLI 共用配置和记录；先预览按需接入，参数以 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。
```

</details>
</aside>

[1. 扫描或登录，获取模型](#get-models) · [2. 配置智能路由](#configure-routing) · [3. 接入 Agent](#connect-agent) · [4. 了解任务委派](#task-delegation)

<h2 id="get-models">1. 扫描或登录，获取模型</h2>

已有 ChatGPT 订阅时，可以直接登录并选用订阅中的 Codex 模型。

1. 打开“模型 → 通用模型 → 添加模型 → **连接订阅**”。
2. 在 **Codex** 卡片点击“登录”，在浏览器里用自己的 ChatGPT 账号完成授权，再回到 HiRoute。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-subscription-login-zh.webp"><img src="/guide-assets/agent-guide-subscription-login-zh.webp" alt="HiRoute 连接订阅页面中的 Codex 与 Claude Code 登录按钮" width="1200" height="900" loading="lazy" /></a>
  <figcaption>在 Codex 卡片点“登录”，使用自己的 ChatGPT 订阅账号完成浏览器授权。</figcaption>
</figure>

3. 看到“已登录，可检查并选择模型”后，点击 **“检查并选择模型”**，再点击 **“检查订阅”**。检查完成后，会显示这个账号可用的模型。
4. 勾选想使用的模型，点击 **“保存并接入”**。回到“通用模型”确认模型已经出现，下一步就能把它们加入路由。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-codex-subscription-models-zh.webp"><img src="/guide-assets/agent-guide-codex-subscription-models-zh.webp" alt="Codex 订阅检查后选择模型并保存接入" width="1200" height="900" loading="lazy" /></a>
  <figcaption>勾选账号可用的模型，再点“保存并接入”，完成订阅模型导入。</figcaption>
</figure>

如果本机已经配置过 Codex，也可以从“添加模型 → **扫描本机**”选择发现的订阅或 API 接入，检查并保存。Codex 的本机登录复用支持文件中的凭据，使用系统钥匙串时，在上面的“连接订阅”里独立登录即可。有 API Key 时，也可以从“新建 API 接入”添加模型。

<h2 id="configure-routing">2. 配置智能路由</h2>

1. 打开“**智能路由 → 新建智能路由**”，命名为“日常开发”，选择 **“按难度分流”**。
2. 在“**常规模型**”组点击“添加模型”，选择第一步接入的快速或经济模型，处理读文件、找定义、整理资料等常规工作。
3. 在“**主力模型**”组添加适合复杂分析、方案判断或多文件修改的模型。两组可以使用不同订阅、API 或本地模型，按各自账号可用的模型组合。
4. “判断方式”先选 **“启发式规则”**，然后点击 **“启用”**。发布后，这条路由就能在下一步的 Agent 接入设置里选到。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-zh.webp"><img src="/guide-assets/agent-guide-difficulty-zh.webp" alt="HiRoute 按难度分流配置中的判断方式、常规模型组和主力模型组" width="1200" height="900" loading="lazy" /></a>
  <figcaption>常规组负责日常步骤，主力组处理复杂轮次。图中的模型可替换为你第一步接入的模型。</figcaption>
</figure>

想根据执行表现进一步调整时，可在同一位置选择决策模型，使用阶段评分和低分保护。[了解难度路由的判断方式](/zh/docs/model-routing/#difficulty-routing)

<h2 id="connect-agent">3. 接入 Agent</h2>

### 使用 Codex Desktop

1. 在 macOS 或 Windows 上安装 ChatGPT / Codex 桌面应用，然后打开 **HiRoute 桌面应用 → Agent → Codex Desktop**。
2. 点击“选择路由并启用”。已有接入时，点“模型路由”右侧的“调整”。勾选刚发布的计划。只有一条路由时会自动作为默认；选择多条时，在“默认选择”中选定常用路由。点击“启用”或“保存配置”。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-codex-desktop-zh.webp"><img src="/guide-assets/agent-guide-codex-desktop-zh.webp" alt="Codex Desktop 模型路由对话框中勾选日常开发路由" width="1200" height="900" loading="lazy" /></a>
  <figcaption>勾选刚创建的路由；只有一条时，独立窗口会默认使用它。</figcaption>
</figure>

3. 回到 Codex Desktop 详情，在“独立窗口”区域点击 **“启动独立窗口”**。HiRoute 会打开使用这套路由的独立桌面窗口。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-codex-desktop-launch-zh.webp"><img src="/guide-assets/agent-guide-codex-desktop-launch-zh.webp" alt="HiRoute 的 Codex Desktop 详情页，显示启动独立窗口按钮" width="1200" height="900" loading="lazy" /></a>
  <figcaption>保存路由后，点击右上方的“启动独立窗口”，在新窗口中开始工作。</figcaption>
</figure>

4. 在新窗口选择项目，点击“新聊天”，在输入框右下角确认或选择刚创建的路由。再发一条任务，例如“找出这个项目的启动入口并说明调用关系”。到 HiRoute“会话 → 路由时间线”查看实际选用的模型。

<figure class="guide-shot-window">
  <a href="/guide-assets/agent-guide-codex-desktop-window-zh.webp"><img src="/guide-assets/agent-guide-codex-desktop-window-zh.webp" alt="Codex Desktop 独立窗口中的新聊天，右下角已选择 Qoder Responses 路由" width="2560" height="1640" loading="lazy" /></a>
  <figcaption>独立窗口打开后，在右下角确认路由。图中选中的是“Qoder Responses”，使用时选择你刚创建的路由。</figcaption>
</figure>

更新路由设置后，退出这个独立实例，再用同一按钮重新启动。

### 使用 Codex CLI

打开“Agent → Codex CLI → 模型路由”，选择 **“按需使用 · 仅 CLI”**，勾选刚发布的路由并设置默认。点击“启用并复制启动命令”，把复制的命令粘贴到项目终端中运行。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-editor-zh.webp"><img src="/guide-assets/agent-editor-zh.webp" alt="Codex CLI 的按需接入和默认路由配置" width="1200" height="900" loading="lazy" /></a>
  <figcaption>选择按需 CLI 接入，再用界面给出的启动命令进入这条路由。</figcaption>
</figure>

启动后用一个小任务确认接入，在 HiRoute 的会话时间线查看实际选模。

<h2 id="task-delegation">4. 了解任务委派</h2>

写代码时，想同时让另一种 Agent 调查报错原因或检查测试覆盖，可以让 Codex CLI 把这项独立工作交给 DSH 或 Pi，再把结果带回当前对话。

想使用时，在路由的“更多设置 → 任务委派”中绑定执行 Agent，并在发起任务的主 Agent 上开启“任务路由”。执行方会按照这份计划选择模型，结果可在“Agent → 任务记录”查看。[继续阅读 Agent 智能路由教程](/zh/docs/task-routing/)
