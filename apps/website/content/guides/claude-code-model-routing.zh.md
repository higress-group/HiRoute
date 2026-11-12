# Claude Code API 赠金与订阅路由

把 Claude 订阅模型和赠金对应的 Claude API 一起接入 HiRoute。用同名候选让订阅额度和 API 赠金互相备用，再把路由映射到 Claude Code 熟悉的 Opus、Sonnet 或 Haiku 预设。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/) · [使用 Claude Code 订阅的 API 赠金](#api-credits)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/claude-code-model-routing/
请帮我按本页完成获取模型、配置智能路由和接入 Agent，再用本页的轻量 Pi 场景说明如何用更少的主力模型 token 完成独立任务。按我的需要启用委派。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 有 API 赠金时，引导我添加领取赠金的组织 Key 或复用对应接入，再按我的优先顺序把订阅和 API 的同名模型加入候选。操作参考 https://hiroute.ai/zh/docs/claude-code-model-routing/#api-credits
- 查找订阅或 API：hiroute compute scan · compute connection options / login / prepare / preview / apply
- 创建计划和预设映射：routing preview / apply · agents connect preview / apply
- 需要任务委派时，配置执行方并核对：worker executors · worker plans · worker result · sessions timeline
界面和 CLI 共用配置和记录；由我完成授权，参数按 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 确认。
```

</details>
</aside>

[1. 扫描或登录，获取模型](#get-models) · [2. 配置智能路由](#configure-routing) · [3. 接入 Agent](#connect-agent) · [4. 了解任务委派](#task-delegation)

<h2 id="get-models">1. 扫描或登录，获取模型</h2>

已有 Claude 订阅时，先把账号里的模型导入 HiRoute。

1. 打开“模型 → 通用模型 → 添加模型 → **连接订阅**”。
2. 在 **Claude Code** 卡片点击“登录”，用自己的 Claude 账号在浏览器里完成授权，然后回到 HiRoute。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-subscription-login-zh.webp"><img src="/guide-assets/agent-guide-subscription-login-zh.webp" alt="HiRoute 订阅接入中的 Claude Code 登录入口" width="1200" height="900" loading="lazy" /></a>
  <figcaption>在 Claude Code 卡片点击“登录”，完成账号授权后回到 HiRoute。</figcaption>
</figure>

3. 点击 **“检查并选择模型”**，再点击 **“检查订阅”**，读取这个账号可用的模型。
4. 从账号目录勾选要使用的模型，例如 Sonnet 或 Opus。如需同时接入赠金，先[填写组织的 API Key](#api-credits)。点击 **“保存并接入”**。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-claude-subscription-models-zh.webp"><img src="/guide-assets/agent-guide-claude-subscription-models-zh.webp" alt="Claude Code 订阅检查后选择 Sonnet 与 Opus 并保存接入" width="1200" height="900" loading="lazy" /></a>
  <figcaption>模型以账号返回的目录为准，勾选后保存到 HiRoute。</figcaption>
</figure>

已经在本机登录 Claude Code 时，也可以从“添加模型 → **扫描本机**”复用登录，检查订阅并保存模型。HiRoute 支持读取现有文件和 macOS 钥匙串登录。有 Anthropic API Key 时，从“新建 API 接入”选择对应服务、检查并保存即可。

<h3 id="api-credits">接入 Claude 订阅的 API 赠金</h3>

符合条件的 Claude Max、Ultra 或 Team 订阅可以领取 Claude API 赠金。把赠金关联到 Claude Console 组织，再从该组织创建 API Key。Claude Code 选用 HiRoute 路由时，走赠金候选的请求通过 Claude API 计费，赠金在 Console 组织中抵扣。领取资格、余额和有效期见 [Claude 官方赠金说明](https://platform.claude.com/docs/en/about-claude/api-credits-for-subscribers)。

1. 在上方的订阅检查结果中勾选要用的模型。在 **Anthropic API Key（可选）** 填入领取赠金的组织 Key，或展开“使用已有 API 接入”选用同一个组织的接入。
2. 点击“保存并接入”。HiRoute 会将订阅模型与 API 目录中支持的同 ID 模型一并接入。已经保存订阅时，可在“通用模型 → 按接入”选中订阅，在“订阅与 API”中点击“关联 API 接入”或“调整关联”。
3. 想按赠金预算用 API，在接入时开启 **“限制 API 花费”**，或之后在该 Key 的详情中设置。按供应商显示的剩余额度填金额，赠金定期到账时可按实际更新日补充。核对模型价格后，已知价格的请求在额度不足时会尝试下一个可用候选。详见 [API Key 额度配置](/zh/docs/models/#api-key-allowance)。

接下来[将订阅与赠金 API 放进同一条路由](#credit-candidates)。

<h2 id="configure-routing">2. 配置智能路由</h2>

1. 打开“**智能路由 → 新建智能路由**”，命名为“Claude 日常开发”，选择 **“按难度分流”**。
2. 在“**常规模型**”组点击“添加模型”，选择第一步接入的快速或经济模型，处理读文件、找定义、整理资料等常规工作。
3. 在“**主力模型**”组添加适合复杂分析、方案判断或多文件修改的模型。两组可以使用不同订阅、API 或本地模型，按各自账号可用的模型组合。
4. “判断方式”先选 **“启发式规则”**，然后点击 **“启用”**。发布后，这条路由就能在下一步的 Agent 接入设置里选到。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-zh.webp"><img src="/guide-assets/agent-guide-difficulty-zh.webp" alt="HiRoute 按难度分流配置中的判断方式、常规模型组和主力模型组" width="1200" height="900" loading="lazy" /></a>
  <figcaption>常规组负责日常步骤，主力组处理复杂轮次。图中的模型可替换为你第一步接入的模型。</figcaption>
</figure>

想根据执行表现进一步调整时，可在同一位置选择决策模型，使用阶段评分和低分保护。[了解难度路由的判断方式](/zh/docs/model-routing/#difficulty-routing)

<h3 id="credit-candidates">把订阅与赠金 API 一起加入同名候选</h3>

如果想用同一个模型并在订阅与赠金间切换，可以用 **固定模型** 路由。回到“模型 → 通用模型 → 按模型”，打开要优先用的那个来源的模型详情，点击 **“创建智能路由”**。看到“发现同名模型”后选 **“一起加入候选”**，在候选列表里选好订阅与赠金 API 两条来源，核对顺序后启用。已有固定路由时，新候选会追加到末尾，可以在配置页重排后发布。下一步把这条路由映射到 Claude Code 的模型预设。

- **订阅优先。** 把订阅候选排在前面，订阅额度暂时不可用时再尝试用 API 赠金。
- **赠金优先。** 赠金快到期时，把 API 候选排在前面。按实际赠金余额设好 Key 花费额度，额度不足时再尝试订阅候选。

想继续按难度分流时，在上面的“添加模型”里分别选中同一模型的订阅和 API 来源，放进同一个常规组或主力组并排列顺序。使用一条路由就能同时选难度和来源。

<h2 id="connect-agent">3. 接入 Agent</h2>

打开“Agent → Claude Code → 模型路由”，展开“高级设置”，将 **Sonnet** 预设映射到刚发布的路由。也可以为 Opus 或 Haiku 预设分别选择路由，保存设置。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-claude-zh.webp"><img src="/guide-assets/agent-guide-claude-zh.webp" alt="Claude Code 的 Opus、Sonnet、Haiku 预设与 HiRoute 路由映射" width="1200" height="900" loading="lazy" /></a>
  <figcaption>将 Sonnet 预设映射到已发布的 HiRoute 路由。</figcaption>
</figure>

重启 `claude`，输入 `/model` 选择映射过的预设，再发一个小任务。HiRoute 会根据这条路由选择模型和来源。到“会话 → 路由时间线”查看实际选用的模型。

<h2 id="task-delegation">4. 了解任务委派</h2>

审查大改动时，可以让轻量的 Pi 先对照 diff 和现有测试，列出缺少覆盖的路径。给 Pi 配经济模型，逐文件查看的开销由低单价模型承担，Claude Code 只读简报并判断哪些缺口值得补。

新建“Pi 测试缺口”固定模型路由，在“更多设置 → 任务委派”开启委派并绑定 Pi，填写使用场景并启用。再在“Agent → Claude Code → 任务路由”开启委派。

> 使用 HiRoute，把当前 diff 的测试覆盖检查委派给“Pi 测试缺口”。返回最多三项，每项给出文件位置和建议用例。

让 Claude Code 取回结果并决定补哪些测试。[查看 Pi 委派配置和用量核对](/zh/docs/task-routing/#lightweight-pi)
