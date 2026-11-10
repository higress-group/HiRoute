# WorkBuddy 模型接入 HiRoute：通过 Magpie headless 让 Pi 执行

把 WorkBuddy 账号里的模型接入 HiRoute，让快速模型处理常规工作，主力模型负责深度判断，再到 Pi 或 Codex CLI 中使用这条路由。WorkBuddy 提供模型，你可以继续使用喜欢的 Agent 来读文件、改代码和整理资料。

[下载 HiRoute](/zh/download/) · [查看接入总览](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/workbuddy-model-routing/
请帮我按本页完成获取模型、配置智能路由和接入 Agent，再介绍任务委派的适用场景，按我的需要选择启用。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 读取和保存 API 接入：hiroute compute list · compute connection options / test / preview / apply
- 配置智能路由并接入 Pi：routing options · routing preview / apply · agents connect preview / apply
- 查看实际选模，需要任务委派时再选择执行方：sessions timeline · worker executors · worker result
界面和 CLI 共用配置和记录。由我完成 WorkBuddy 授权；只用实际目录里的模型。参数按 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 确认。
```

</details>
</aside>

[1. 扫描或登录，获取模型](#connect-workbuddy) · [2. 配置智能路由](#configure-routing) · [3. 接入 Agent](#connect-agent) · [4. 了解任务委派](#task-delegation)

<h2 id="connect-workbuddy">1. 扫描或登录，获取模型</h2>

通过 Magpie headless 登录 WorkBuddy，读取账号可用的模型目录，再接入 HiRoute。[安装 Magpie CLI](https://usemagpie.ai/docs/start) 后运行以下命令，已有后台服务时可以直接添加插件并登录。

```sh
magpie daemon install --no-web
magpie plugin add @magpie-community/opencode-workbuddy-auth
magpie plugin login workbuddy
curl -fsS http://127.0.0.1:3425/v1/models \
  -H 'Authorization: Bearer magpie'
```

`workbuddy` 对应国内版的 CodeBuddy 套餐，国际版用 `magpie plugin login workbuddy-ai`。在浏览器完成账号授权后，重新查询目录，记下要用的完整模型 ID。

在 HiRoute 打开“模型 → 通用模型 → 添加模型 → **新建 API 接入 → 自定义 API**”，按下面填写并检查。

| 设置 | 填写内容 |
| --- | --- |
| API 地址 | `http://127.0.0.1:3425/v1`，网关改过地址时用实际值 |
| 协议 | Responses，适用于下面的 Pi 或 Codex CLI 示例 |
| API Key | 默认本机网关为 `magpie`，改过 Key 时填自己的值 |
| 模型 | 从返回目录选择，保留完整来源前缀 |

点击“检查并继续”，选择模型并保存。目录没有显示时，在“高级设置”把模型目录路径设为 `/v1/models`，也可以手动填写刚查到的完整 ID。如果已经有受支持的 Agent 配置指向这个网关，可先用“扫描本机”发现并复用接入。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-magpie-api-zh.webp"><img src="/guide-assets/agent-guide-magpie-api-zh.webp" alt="在 HiRoute 自定义 API 中填写 Magpie 地址和 Responses 协议" width="1200" height="900" loading="lazy" /></a>
  <figcaption>把 WorkBuddy 模型所在的 Magpie API 保存到 HiRoute。</figcaption>
</figure>

<h2 id="configure-routing">2. 配置智能路由</h2>

1. 打开“**智能路由 → 新建智能路由**”，命名为“WorkBuddy 资料整理”，选择 **“按难度分流”**。
2. 在“**常规模型**”组点击“添加模型”，选择第一步接入的快速或经济模型，处理读文件、找定义、整理资料等常规工作。
3. 在“**主力模型**”组添加适合复杂分析、方案判断或多文件修改的模型。两组可以使用不同订阅、API 或本地模型，按各自账号可用的模型组合。
4. “判断方式”先选 **“启发式规则”**，然后点击 **“启用”**。发布后，这条路由就能在下一步的 Agent 接入设置里选到。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-zh.webp"><img src="/guide-assets/agent-guide-difficulty-zh.webp" alt="HiRoute 按难度分流配置中的判断方式、常规模型组和主力模型组" width="1200" height="900" loading="lazy" /></a>
  <figcaption>常规组负责日常步骤，主力组处理复杂轮次。图中的模型可替换为你第一步接入的模型。</figcaption>
</figure>

想根据执行表现进一步调整时，可在同一位置选择决策模型，使用阶段评分和低分保护。[了解难度路由的判断方式](/zh/docs/model-routing/#difficulty-routing)

<h2 id="connect-agent">3. 接入 Agent</h2>

### 在 Pi 中使用 WorkBuddy 模型

打开“Agent → Pi → 模型路由”，勾选刚发布的“WorkBuddy 资料整理”，选择与接入一致的 Responses 协议并保存。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-workbuddy-pi-zh.webp"><img src="/guide-assets/agent-guide-workbuddy-pi-zh.webp" alt="Pi 的模型路由选择界面" width="1200" height="900" loading="lazy" /></a>
  <figcaption>在 Pi 的接入设置中，选择你刚创建的 WorkBuddy 路由。</figcaption>
</figure>

重启 Pi，输入 `/model` 选择这条路由，试着让它归纳项目 README 的三条要点。模型来自 WorkBuddy 接入，读文件等操作由 Pi 执行。在 HiRoute“会话 → 路由时间线”可查看实际模型。

也可以把同一计划勾选到“Agent → Codex CLI → 模型路由”，用“按需使用 · 仅 CLI”给出的命令启动。图文步骤见 [Codex 接入教程](/zh/docs/codex-model-routing/#connect-agent)。

<h2 id="task-delegation">4. 了解任务委派</h2>

整理资料时，可以让 Pi 批量阅读文档，让 Codex CLI 单独检查文档中的代码示例，再把两边结果汇总到主对话。WorkBuddy 提供模型，你可以按任务需要选择执行 Agent。

想使用时，在路由的“更多设置 → 任务委派”中绑定执行 Agent，并在发起任务的主 Agent 上开启“任务路由”。执行方会按照这份计划选择模型，结果可在“Agent → 任务记录”查看。[继续阅读 Agent 智能路由教程](/zh/docs/task-routing/)
