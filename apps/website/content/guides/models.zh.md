# 接入模型

HiRoute 在本机集中管理订阅、API 和本地模型。“模型”页面分两类：“通用模型”负责回答用户、调用工具和完成任务；“决策模型”只负责判断任务难度和评分，接入方法见 [智能模型路由](/zh/docs/model-routing/)。

本文介绍通用模型：怎样接入、怎样把订阅和 API 里的同名模型组合成一条路由，以及怎样限制 API Key 的花费。桌面应用和 Linux Web UI 的操作相同；命令行对应的 `compute` 命令见 [HiRoute CLI](/zh/docs/cli/)。

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>

复制以下指引到 Agent 对话中，再说明你想完成的操作。内容包含文档链接、内置技能入口和 CLI 线索。

<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
请先阅读以下文档，结合我在对话中提出的目标，用 HiRoute CLI 完成“接入模型”相关操作。目标不明确时，只询问必要信息。

本页文档：https://hiroute.ai/zh/docs/models/
CLI 参考：https://hiroute.ai/zh/docs/cli/

请使用产品内置的 HiRoute 管理技能；尚未启用时，按图示引导我在“Agent → HiRoute 管理 → 启用”中开启：https://hiroute.ai/zh/docs/#management-skill

操作线索（按本次目标选用；下面均为 hiroute 子命令）：
- 查看或复用已有模型：compute list · compute show · compute connection options
- 扫描本机、连接订阅或 API：compute scan · compute connection prepare / authorize / login / test
- 选择模型并保存接入：compute connection preview → compute connection apply
- 把同名模型加入同一路由：routing options → routing preview → routing apply
- 在接入详情中设置 API Key 额度。用 prices effective · prices override preview / apply 查看或修改模型价格。
- 设置接入代理：compute network show / preview / apply

界面与 CLI 连接同一个 HiRoute 服务，共用配置和执行记录。先读取现状并复用已有配置；参数以本机命令 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。按内置技能完成所需预览、应用与核验，最后说明结果。
```

</details>
</aside>

## 添加模型

打开“模型 → 通用模型”，选择“添加模型”：

- **连接订阅**：在 HiRoute 中独立登录 Codex 或 Claude Code 订阅。HiRoute 会自动刷新这份登录凭据，不需要保持原生客户端运行。
- **扫描本机**：发现本机已有的 Agent、Codex / Claude Code 订阅和模型配置，直接复用。复用订阅时，登录凭据的刷新仍依赖原生客户端；Codex 只能复用保存在文件中的登录，钥匙串等存储方式请改用独立登录。
- **新建 API 接入**：从模板开始，填写 API Key，读取服务的模型目录后选择要加入的模型。读取目录不会发起付费推理，API Key 只保存在本机。没有合适的模板时，选择“自定义 API”，手动填写地址和协议；代理服务和本地模型也从这里接入。
- **向已有接入添加模型**：复用已保存的端点和凭据，补充同一服务里的其他模型。
- **浏览免费模型**：查看各服务的免费条件和获取 Key 的指引。

保存后，模型显示“接入就绪”，表示它满足进入路由的本地条件，不代表上游推理或工具调用已经验证过。用一次真实请求确认实际效果。

<figure>
  <a href="/guide-assets/models-add-zh.webp"><img src="/guide-assets/models-add-zh.webp" alt="模型页面的添加模型对话框，提供订阅、扫描本机和 API 等入口" width="1200" height="900" loading="lazy" /></a>
  <figcaption>已有订阅选“连接订阅”；复用本机配置选“扫描本机”；有 Key 和地址选“新建 API 接入”。</figcaption>
</figure>

## 接入后，选择用途

已有 Qoder 或 WorkBuddy 账号？可以通过 [Magpie headless](/zh/docs/magpie-model-sources/) 把可用模型接入 HiRoute。[按 Agent 接入](/zh/docs/agent-integrations/) 也列出了其他来源与各自的配置步骤。

- **给日常 Agent 降低模型成本**：去 [智能模型路由](/zh/docs/model-routing/) 创建路由，再映射到你的 Agent。
- **先试一个新模型**：去首页的“新模型”，按 [试用流程](/zh/docs/model-trial/) 完成配置。
- **给另一种 Agent 执行任务**：去 [Agent 智能路由](/zh/docs/task-routing/) 把执行模型与执行 Agent 组合成可委派的路由。

完成标志是模型已保存、出现在路由候选中；真实请求成功后，再确认上游推理和工具调用是否正常。

## 订阅和 API 一起接入

接入 Codex 或 Claude 订阅并完成检查后，可以在同一个表单里填写对应的 OpenAI 或 Anthropic API Key，也可以选择一个已有的 API 接入。留空就只使用订阅。选好模型、保存一次，订阅和 API 两边可用的模型会一起导入。

部分订阅附带按账单周期发放的 API 赠金。这时填写领取赠金的那个组织的 Key，再按下文给这个 Key 设一份花费额度，就能把赠金用在 HiRoute 的请求上。

## 用同名模型组成一条路由

打开一个模型的详情，选择“创建智能路由”。如果其他接入也提供这个模型，HiRoute 会提示“其他接入点也有这个模型，是否一起加入候选？”：

- **一起加入候选**：当前模型排在第一位，其他接入作为备用。
- **仅用当前模型**：路由里只放这一个模型。

新路由以“固定模型”方式创建，按候选顺序尝试，暂时不可用的候选会被跳过。也可以把模型加入已有路由，已有候选的顺序保持不变，新模型排在后面。保存或发布前，路由不会生效。

这样，订阅和 API 里的同名模型就组成了一条路由：Agent 只用一个模型名，HiRoute 先用订阅，订阅暂时用不了时接着用 API。

## 限制 API Key 的花费

在 API 接入的详情中打开“API Key 与额度”，为某个 Key 开启“限制 API 花费”，再选择额度方式：

- **固定金额，用完即止**：适合一次性的预算。
- **定期补充**：按天、周、月或年补充，可以设置补充日期、时间和时区。余额规则可选“每期恢复到设定金额，余额不结转”，或“累加，并限制余额上限”。
- **共用已有额度**：多个 Key 共用同一份额度。

“首次可用金额”可以留空，表示从完整金额开始；如果赠金已经用掉一部分，可以填写剩余金额。

额度用于估算经过 HiRoute 的花费。单价已知时，HiRoute 在转发前预留额度，请求完成后结算。复用同一 API Key 的决策模型调用也计入额度。请在模型详情的“调整价格”中设置同币种的适用输入、输出与缓存单价。价格未知的请求仍可执行，保留为未计价记录，不计入已用花费。额度不足以支付已知费用时，HiRoute 会尝试其他 Key 或下一个候选。

## 设置代理

每个 API 接入都有自己的“接入网络设置”，在“连接方式”中选择：

- **直连（默认）**
- **服务环境代理**：沿用 HiRoute 后台服务启动时的代理环境。
- **手动代理**：填写 HTTP / HTTPS 代理地址，以及不经过代理的地址（可选）。localhost 和回环地址总是直连，代理地址不能包含账号或密码。

保存后，新的请求立即使用新设置，进行中的请求保持原来的路径；保存不代表连通性已经验证。复用这个接入的决策模型也遵循同样的设置。代理地址在 HiRoute 服务所在的机器上解析：通过远程 Web UI 配置时，`127.0.0.1` 指的是服务器本身，不是浏览器所在的电脑。

订阅连接的代理在“模型”页面的“订阅连接代理”中单独设置，可选自动（使用启动时的代理）、直接连接或手动代理。保存后需要重启 HiRoute。

## 模型资料与新模型推荐

上下文窗口、输出上限等官方模型资料，以及首页“新模型”中的推荐，都由 hiroute.ai 单独维护和发布，本机服务会自动获取经过校验的内容，不需要等应用升级。已发布的路由保留发布时的能力信息；修改模型或重新发布后才会采用新的资料。

供应商可能在同一个模型名下更新模型，资料里的日期不代表接入实际使用的版本。
