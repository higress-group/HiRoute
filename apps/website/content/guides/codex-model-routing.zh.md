# Codex API 赠金与订阅路由

把 ChatGPT 订阅和领取了 API 赠金的 OpenAI 接入一起导入 HiRoute。一个 GPT 模型可以有两条候选来源，订阅额度用完时用赠金，也可以先用快到期的赠金。配好路由后，在 Codex Desktop 独立窗口或 Codex CLI 里直接选用。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/) · [使用 Codex 订阅的 API 赠金](#api-credits)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/codex-model-routing/
请帮我按本页完成获取模型、配置智能路由和接入 Agent，再用本页的轻量 Pi 场景说明如何用更少的主力模型 token 完成独立任务。按我的需要启用委派。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 有 API 赠金时，引导我添加领取赠金的组织 Key 或复用对应接入，再按我的优先顺序把订阅和 API 的同名模型加入候选。操作参考 https://hiroute.ai/zh/docs/codex-model-routing/#api-credits
- 我需要用决策模型判断难度时，复用同一份 OpenAI API 接入。参考 decision connections options / list / apply / test 和 https://hiroute.ai/zh/docs/codex-model-routing/#openai-decision-model
- 查找 Codex 接入和配置目标：hiroute compute scan · compute connection options · quickstart
- 按我使用的客户端接入 Codex Desktop 或 Codex CLI。创建路由和预览接入用 routing preview / apply · agents connect preview / apply / status。
- 接入 Desktop 后引导我从 HiRoute 启动独立窗口。也可在本机图形会话用 agents desktop launch。
- 需要委派时，用 worker executors · worker plans · worker result 选择执行方并核对结果。
界面和 CLI 共用配置和记录。参数以 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。
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
4. 勾选想使用的模型。如需同时接入赠金，先[填写组织的 API Key](#api-credits)。点击 **“保存并接入”** 后，在“通用模型”确认模型已出现。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-codex-subscription-models-zh.webp"><img src="/guide-assets/agent-guide-codex-subscription-models-zh.webp" alt="Codex 订阅检查后选择模型并保存接入" width="1200" height="900" loading="lazy" /></a>
  <figcaption>勾选账号可用的模型，再点“保存并接入”，完成订阅模型导入。</figcaption>
</figure>

如果本机已经配置过 Codex，也可以从“添加模型 → **扫描本机**”选择发现的订阅或 API 接入，检查并保存。Codex 的本机登录复用支持文件中的凭据，使用系统钥匙串时，在上面的“连接订阅”里独立登录即可。有 API Key 时，也可以从“新建 API 接入”添加模型。

<h3 id="api-credits">接入 ChatGPT 订阅的 OpenAI API 赠金</h3>

符合条件的 ChatGPT Pro 或 Business 订阅可以领取 OpenAI API 赠金。把订阅关联到接收赠金的 API 组织，再从该组织的项目创建 API Key。按量请求通过该 Key 使用组织里的赠金。领取资格、余额和有效期见 [OpenAI 官方赠金说明](https://developers.openai.com/api/docs/guides/chatgpt-api-credits)。

1. 在上方的订阅检查结果中勾选要用的模型。在 **OpenAI API Key（可选）** 填入领取赠金的组织 Key，或展开“使用已有 API 接入”选用同一个组织的接入。
2. 点击“保存并接入”。HiRoute 会将订阅模型与 API 目录中支持的同 ID 模型一并接入。已经保存订阅时，可在“通用模型 → 按接入”选中订阅，在“订阅与 API”中点击“关联 API 接入”或“调整关联”。
3. 想按赠金预算用 API，在接入时开启 **“限制 API 花费”**，或之后在该 Key 的详情中设置。按供应商显示的剩余额度填金额，赠金定期到账时可按实际更新日补充。核对模型价格后，已知价格的请求在额度不足时会尝试下一个可用候选。详见 [API Key 额度配置](/zh/docs/models/#api-key-allowance)。

接下来可以[组合订阅与赠金的同名候选](#credit-candidates)，也可以[复用赠金接入配置 OpenAI 决策模型](#openai-decision-model)。

<h2 id="configure-routing">2. 配置智能路由</h2>

1. 打开“**智能路由 → 新建智能路由**”，命名为“日常开发”，选择 **“按难度分流”**。
2. 在“**常规模型**”组点击“添加模型”，选择第一步接入的快速或经济模型，处理读文件、找定义、整理资料等常规工作。
3. 在“**主力模型**”组添加适合复杂分析、方案判断或多文件修改的模型。两组可以使用不同订阅、API 或本地模型，按各自账号可用的模型组合。
4. “判断方式”先选 **“启发式规则”**，然后点击 **“启用”**。发布后，这条路由就能在下一步的 Agent 接入设置里选到。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-zh.webp"><img src="/guide-assets/agent-guide-difficulty-zh.webp" alt="HiRoute 按难度分流配置中的判断方式、常规模型组和主力模型组" width="1200" height="900" loading="lazy" /></a>
  <figcaption>常规组负责日常步骤，主力组处理复杂轮次。图中的模型可替换为你第一步接入的模型。</figcaption>
</figure>

<h3 id="openai-decision-model">用赠金 API 接入 OpenAI 决策模型</h3>

可以让 Codex 的订阅候选继续执行任务，同时用第一步的赠金 API 接入做难度判断和阶段评分。

1. 打开上面创建的 **按难度分流** 路由。在“判断方式”中选择 **“决策模型”**，点击 **“添加决策模型”**。
2. 在“凭据”选择 **“使用已有 API 接入”**，选第一步保存的 OpenAI 赠金接入。HiRoute 会复用这份接入中的 Key。
3. 模型 ID 填 `gpt-6-luna`，完整地址填 `https://api.openai.com/v1/decisions`，协议选 **“OpenAI 决策接口”**。已有预填时核对这三项。取名“OpenAI 难度判断”，点击 **“保存并测试”**。[OpenAI Decisions 官方文档](https://developers.openai.com/api/docs/guides/decisions)
4. 回到路由，确认“使用的决策模型”选中了这份连接，再点击 **“发布更改”**。接入 Codex 后发一个小任务，在“会话 → 路由时间线”查看难度判断和实际执行模型。

决策调用计入这份接入对应的 OpenAI API 组织，可以用该组织的赠金抵扣。即使执行候选也使用同一个 Key，决策调用也不占用 HiRoute 的 [本地 Key 花费额度](/zh/docs/models/#api-key-allowance)。[了解阶段评分和低分保护](/zh/docs/model-routing/#difficulty-routing)

<h3 id="credit-candidates">把订阅与赠金 API 一起加入同名候选</h3>

如果想用同一个模型并在订阅与赠金间切换，可以用 **固定模型** 路由。回到“模型 → 通用模型 → 按模型”，打开要优先用的那个来源的模型详情，点击 **“创建智能路由”**。看到“发现同名模型”后选 **“一起加入候选”**，在候选列表里选好订阅与赠金 API 两条来源，核对顺序后启用。已有固定路由时，新候选会追加到末尾，可以在配置页重排后发布。在 Codex 的模型选择器中选这条路由即可。

- **订阅优先。** 把订阅候选排在前面，订阅额度暂时不可用时再尝试用 API 赠金。
- **赠金优先。** 赠金快到期时，把 API 候选排在前面。按实际赠金余额设好 Key 花费额度，额度不足时再尝试订阅候选。

想继续按难度分流时，在上面的“添加模型”里分别选中同一模型的订阅和 API 来源，放进同一个常规组或主力组并排列顺序。使用一条路由就能同时选难度和来源。

<h2 id="connect-agent">3. 接入 Agent</h2>

按你的使用方式选择 [Codex Desktop 独立窗口](#codex-desktop)或 [Codex CLI](#codex-cli)。

<h3 id="codex-desktop">使用 Codex Desktop</h3>

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
  <figcaption>在独立窗口的右下角确认路由。图中选中的是“Qoder Responses”，使用时选择你刚创建的路由。点击截图查看原图。</figcaption>
</figure>

更新路由设置后，退出这个独立实例，再用同一按钮重新启动。

<h3 id="codex-cli">使用 Codex CLI</h3>

打开“Agent → Codex → 模型路由”，在“高级设置”中确认 **“按需使用 · 仅 CLI”**。勾选刚发布的路由。若显示“默认选择”，选中这条路由。点击“启用并复制启动命令”，把命令粘贴到项目终端中运行。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-editor-zh.webp"><img src="/guide-assets/agent-editor-zh.webp" alt="Codex CLI 的按需接入和默认路由配置" width="1200" height="900" loading="lazy" /></a>
  <figcaption>选择按需 CLI 接入，再用界面给出的启动命令进入这条路由。</figcaption>
</figure>

启动后用一个小任务确认接入，在 HiRoute 的会话时间线查看实际选模。

<h2 id="task-delegation">4. 了解任务委派</h2>

修复 CI 时，可以先让轻量的 Pi 梳理失败日志，由 Codex 专注定位根因和改代码。给 Pi 配支持工具调用的经济模型，长日志和中间工具输出就由这个模型处理，主会话只需读一段摘要。

建一条“Pi 日志初筛”固定模型路由，在“更多设置 → 任务委派”中开启委派并选 Pi，填写用途并启用。然后到“Agent → Codex → 任务路由”开启委派。使用独立 Desktop 窗口的读者可以切到 [Codex CLI](#codex-cli) 发起任务。

> 用 HiRoute 把这次 CI 日志的初筛交给“Pi 日志初筛”。返回第一处失败、相关文件和一条复现命令。

让 Codex 取回结果，在“Agent → 任务记录”核对执行方，再到“会话 → 路由时间线”查看模型与用量。[配置 Pi 轻量委派](/zh/docs/task-routing/#lightweight-pi)
