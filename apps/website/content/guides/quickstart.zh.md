# 快速开始

HiRoute 帮你做两个选择：**这一阶段用哪个模型，这项任务交给哪个 Agent。** 你可以先用其中一条，也可以把它们组合起来。

**HiRoute 界面里的各项管理与任务操作，都可以通过 CLI 完成。** 两者连接同一个 HiRoute 服务，共用底层配置、任务和执行记录：在界面里配置后，CLI 可以接着操作；让 Agent 通过 CLI 完成的改动，也能在界面里查看。**推荐先开启内置管理技能，让 Agent 操作 CLI，更快上手 HiRoute。**

| 你想解决的问题 | 从哪里开始 | 完成后能做什么 |
| --- | --- | --- |
| 日常编码成本高，想试更经济的新模型 | **模型智能路由**：接入模型 → 创建路由 → 连接常用 Agent | 简单工作用常规模型，难点交给主力；查看实际用量和表现 |
| 想在熟悉的 Agent 中使用另一套模型和执行工具 | **Agent 智能路由**：配置执行 Agent → 开启任务路由 → 发起委派 | 从 Codex 调用 DeepSeek Harness，或从 Hermes 把编码任务交给 Codex |

截图使用夜晚模式与示例数据。点击图片可查看大图。

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>

复制以下指引到 Agent 对话中，再说明你想完成的操作。内容包含文档链接、内置技能入口和 CLI 线索。

<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
请先阅读以下文档，结合我在对话中提出的目标，用 HiRoute CLI 完成“快速开始”相关操作。目标不明确时，只询问必要信息。

本页文档：https://hiroute.ai/zh/docs/
CLI 参考：https://hiroute.ai/zh/docs/cli/

请使用产品内置的 HiRoute 管理技能；尚未启用时，按图示引导我在“Agent → HiRoute 管理 → 启用”中开启：https://hiroute.ai/zh/docs/#management-skill
涉及任务委派时，请使用产品内置委派技能；尚未启用时，引导我在“Agent → 任务路由 → 启用”中开启，按已安装技能处理委派：https://hiroute.ai/zh/docs/task-routing/#delegation-skill

操作线索（按本次目标选用；下面均为 hiroute 子命令）：
- 查看当前配置与下一步：quickstart
- 接入模型：compute list → compute connection options → compute connection preview → compute connection apply
- 创建、调整和发布模型路由：routing options → routing preview → routing apply
- 连接 Agent 的模型路由：agents connect preview --context CONTEXT_ID --plan PLAN_ID → agents connect apply
- 开启内置管理 / 委派技能：agents connect preview + --management enable / --collaboration explicit → agents connect apply
- 核对实际运行：agents connect status --effective · sessions list · sessions show · worker status · worker result

界面与 CLI 连接同一个 HiRoute 服务，共用配置和执行记录。先读取现状并复用已有配置；参数以本机命令 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。按内置技能完成所需预览、应用与核验，最后说明结果。
```

</details>
</aside>

## 先安装并启动 HiRoute

- **macOS**：要求 macOS 15.0 或更高版本。公开安装包为自签名构建，核对 SHA256 后，按 [macOS 安装与首次启动](/zh/docs/install-macos/) 打开应用。
- **Windows**：原生 x64，按当前用户安装，自带服务与 CLI，不需要 Python 或管理员权限。参见 [Windows 安装](/zh/docs/install-windows/)。
- **Linux**：支持 x86_64 与 ARM64，无需 `sudo`。开启 Web UI 后，在浏览器中使用相同界面；也可只用 CLI。参见 [Linux 安装](/zh/docs/install-linux/)。

[下载页](/zh/download/)列出已发布的安装包。打开后先确认服务就绪；浏览器管理的是服务所在主机的模型、Agent 和文件。使用路由或运行任务时，请保持 HiRoute 服务运行。

<h2 id="management-skill">推荐：开启管理技能，让 Agent 帮你上手</h2>

1. 打开“Agent”，选择你常用的 Agent，在“HiRoute 管理”中选择“启用”，打开配置。
2. 如果页面提示命令行入口尚未准备，先选择“准备命令行入口”，确保 Agent 能运行 `hiroute`。再选择“启用 HiRoute 管理”，按提示重新加载技能或开启新会话。
3. 直接告诉 Agent 你想完成什么，例如接入一个模型、创建适合日常编码的路由，或查看最近的模型表现。产品内置管理技能会引导它读取现状、通过 CLI 配置并核对结果；你可以随时回到界面查看。

管理技能可以独立启用，调用它的 Agent 仍可使用原来的模型。下面两条路径既可在界面中操作，也可以把对应文档交给已启用管理技能的 Agent。

<figure>
  <a href="/guide-assets/agent-management-zh.webp"><img src="/guide-assets/agent-management-zh.webp" alt="夜晚模式的 HiRoute 管理技能启用对话框，提供命令行入口准备与启用按钮" width="1200" height="900" loading="lazy" /></a>
  <figcaption>“Agent → HiRoute 管理 → 启用”准备产品内置技能；模型路由与任务路由分别设置。</figcaption>
</figure>

## 路径一：让常用 Agent 用上模型路由

适合先解决成本问题，或在真实工作中试一个新模型。

1. **接入模型。** 打开“模型 → 通用模型 → 添加模型”，连接已有订阅、扫描本机配置或添加 API。检查连接、选择模型并保存。详见 [接入模型](/zh/docs/models/)。
2. **创建路由。** 打开“智能路由 → 新建智能路由”。想先跑通链路，选“固定模型”；想把简单工作交给更便宜的模型，选“按难度分流”，配置常规组、主力组和决策方式，再选择“启用”。
3. **连接常用 Agent。** 打开“Agent”，选择 Codex 等目标，在“模型路由”中选择“启用”，勾选刚发布的路由并保存。Codex 可选“按需使用 · 仅 CLI”，再复制启动命令；其他接入方式按页面提示重新启动或加载配置。
4. **做一个小任务并核对。** 在 Agent 中选择这条路由，发起可明确验收的小任务。回到“会话”，确认实际使用的路由、模型和用量。保存配置不代表运行中的会话已经切换。

**想直接试新模型？** 从首页“新模型”的“试用”或“接入并试用”进入，一次完成选模型、发布路由与映射 Agent。新模型先承担简单轮次，熟悉的主力模型兜底。

<figure>
  <a href="/guide-assets/trial-entry-zh.webp"><img src="/guide-assets/trial-entry-zh.webp" alt="夜晚模式的新模型试用入口，模型卡片提供试用操作" width="1200" height="900" loading="lazy" /></a>
  <figcaption>从首页“新模型”卡片进入试用；是否需要接入，取决于你已有的模型连接。</figcaption>
</figure>

继续阅读：[试用新模型](/zh/docs/model-trial/) · [配置智能模型路由](/zh/docs/model-routing/)。实际节省取决于模型价格、任务分布和兜底频率，以你的执行记录为准。

## 路径二：把任务交给另一种 Agent

模型与 Harness（Agent 的执行工具）不必绑定你的日常工作入口。例如，你可以继续在 Codex 对话，把一个独立任务交给使用 DeepSeek 模型的 DeepSeek Harness；也可以在 Hermes 中把编码任务交给 Codex。

1. **准备执行方。** 在 HiRoute 服务所在主机安装目标执行 Agent，并接入它要使用的模型。
2. **配置可委派的路由。** 在“智能路由”中新建或编辑路由，选好模型，展开“更多设置 → 任务委派”。打开“允许委派任务给执行 Agent”，填写使用场景、选择执行 Agent，检查环境后发布。
3. **开启主 Agent 的任务路由。** 在“Agent”中选择日常使用的主 Agent，在“任务路由”中选择“启用”。初次可选“仅在明确要求时”，完成页面提示的技能检查并保存。主 Agent 可以继续使用原来的模型接入。
4. **明确委派并查看结果。** 对主 Agent 说：“使用 HiRoute，将这个任务委派给【路由名称】执行，完成后汇总修改和测试结果。”把占位内容换成已发布的路由名称，并说明任务目标和工作目录。执行进度在“Agent → 任务记录”中查看。

<figure>
  <a href="/guide-assets/agent-task-zh.webp"><img src="/guide-assets/agent-task-zh.webp" alt="主 Agent 的任务路由设置，可以选择仅在明确要求时委派" width="1200" height="900" loading="lazy" /></a>
  <figcaption>模型路由和任务路由分别配置。这里控制主 Agent 何时发起委派。</figcaption>
</figure>

委派以 `approve-all` 运行：执行 Agent 可以读写文件、运行命令和访问网络，工作目录不构成沙箱。首次先选范围明确、允许执行这些操作的小任务。

继续阅读：[Agent 智能路由：配置、示例与结果](/zh/docs/task-routing/)。

更多操作可把每页的“给 Agent 的指引”复制给 Agent，具体命令与恢复方式见 [HiRoute CLI](/zh/docs/cli/)。
