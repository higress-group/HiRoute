# 快速开始

HiRoute 帮你做两个选择：**这一阶段用哪个模型，这项任务交给哪个 Agent。** 你可以先用其中一条，也可以把它们组合起来。

| 你想解决的问题 | 从哪里开始 | 完成后能做什么 |
| --- | --- | --- |
| 日常编码成本高，想试更经济的新模型 | **模型智能路由**：接入模型 → 创建路由 → 连接常用 Agent | 简单工作用常规模型，难点交给主力；查看实际用量和表现 |
| 想在熟悉的 Agent 中使用另一套模型和执行工具 | **Agent 智能路由**：配置执行 Agent → 开启任务路由 → 发起委派 | 从 Codex 调用 DeepSeek Harness，或从 Hermes 把编码任务交给 Codex |

截图使用夜晚模式与示例数据。点击图片可查看大图。


<details class="agent-prompt" id="agent-cli">
<summary>交给 Agent：完成模型路由接入（CLI）</summary>

复制到能在目标主机运行命令的 Agent 输入框，替换方括号中的内容后发送。

```text
请用 HiRoute CLI 为我完成模型路由接入。
目标 Agent：[名称；若有多个配置，指定目录或 profile]
模型来源：[已有订阅 / 已保存 API 接入 / 待添加的供应商和地址]
常规模型：[名称]；主力模型：[名称]
路由名称：[日常开发]；用途：[我的典型任务]
决策方式：[已保存决策连接 / 本地启发式规则]
验证任务：[选填；留空只配置，不调用模型]

1. 运行 hiroute quickstart、hiroute compute list、hiroute routing list，复用已有资源，解析准确的目标 context 和模型 binding。Hermes 使用明确的 --hermes-root。
2. 缺少来源时，按 compute connection 的 options、prepare 或受保护授权流程接入；先 preview --apply-file，再 apply --request-file。
3. 读取 routing options。用 routing preview 创建 smart_saving 路由，分别传 --economy、--primary，以及 --decision local_rules 或 --decision-service ID@REVISION，生成新的 Apply 文件后发布。
4. 用 agents connect preview --context CONTEXT_ID --plan PLAN_ID 准备模型接入。Codex 优先独立的按需 CLI profile；明确原生模型保留和默认策略。其他 Agent 按当前帮助选择协议或预设，只改模型路由。
5. 应用后查询 agents connect status --effective。按返回指引加载配置；只有填写了验证任务才执行，并用会话记录核对实际路由、模型和结果。不能把配置保存成功说成已生效。

执行约定：先读取当前配置、命令 --help 和所需 schema，使用实际返回的 ID、版本与协议。先 Preview，核对改动符合以上目标后 Apply；只在缺少必要选择或超出目标时问我。保留私有 Apply 文件和提交标识，响应不确定先查询原操作，不重新生成提交。凭证只走受保护输入，由我在自己的终端录入，不进入聊天、命令参数或普通文件。最后分别报告：已保存的配置、需要加载的原生设置、真实执行证据或待验证项。
```

命令格式、凭证接入和恢复细节见 [HiRoute CLI](/zh/docs/cli/)。

</details>

<details class="agent-prompt">
<summary>交给 Agent：完成跨 Agent 委派配置（CLI）</summary>

复制到能在目标主机运行命令的 Agent 输入框，替换方括号中的内容后发送。

```text
请用 HiRoute CLI 配置跨 Agent 委派。
主 Agent：[Codex / Hermes / 其他；明确配置目录或 profile]
执行 Agent：[DeepSeek Harness / Codex CLI / 其他]
执行模型：[名称和来源]；任务路由名称：[名称]
用途：[适合委派的任务与交付物]
我现在只配置，暂不启动任务；主 Agent 保持原来的模型接入。

1. 运行 hiroute quickstart、hiroute worker executors、hiroute worker plans，核对主 Agent 的 context、执行器可用性和已有路由；缺模型时先按 compute connection 流程接入。
2. 新建或调整 routing preview，选好执行模型，设置 --delegation enable，并同时填写 --worker-harness 与 --worker-protocol；填写用途。Codex CLI 的 harness 为 codex_cli，DeepSeek Harness 为 deepseek_harness，协议按当前要求选择。
3. 预览并发布后，使用 agents connect preview --context CONTEXT_ID --collaboration explicit --apply-file NEW_FILE，仅开启主 Agent 的明确请求委派。此步不传 --plan，不修改主 Agent 模型或管理 Skill；按提示完成本机技能检查再应用。
4. 确认 worker plans 能发现已发布路由，执行器就绪，任务路由已配置。给我一条引用准确路由名称的委派提示词，并列出尚缺的环境条件。

执行约定：先读取当前配置、命令 --help 和所需 schema，使用实际返回的 ID、版本与协议。先 Preview，核对改动符合以上目标后 Apply；只在缺少必要选择或超出目标时问我。保留私有 Apply 文件和提交标识，响应不确定先查询原操作，不重新生成提交。凭证只走受保护输入，由我在自己的终端录入，不进入聊天、命令参数或普通文件。最后分别报告：已保存的配置、需要加载的原生设置、真实执行证据或待验证项。
```

命令格式、凭证接入和恢复细节见 [HiRoute CLI](/zh/docs/cli/)。

</details>

## 先安装并启动 HiRoute

- **macOS**：要求 macOS 15.0 或更高版本。公开安装包为自签名构建，核对 SHA256 后，按 [macOS 安装与首次启动](/zh/docs/install-macos/) 打开应用。
- **Windows**：原生 x64，按当前用户安装，自带服务与 CLI，不需要 Python 或管理员权限。参见 [Windows 安装](/zh/docs/install-windows/)。
- **Linux**：支持 x86_64 与 ARM64，无需 `sudo`。开启 Web UI 后，在浏览器中使用相同界面；也可只用 CLI。参见 [Linux 安装](/zh/docs/install-linux/)。

[下载页](/zh/download/)列出已发布的安装包。打开后先确认服务就绪；浏览器管理的是服务所在主机的模型、Agent 和文件。使用路由或运行任务时，请保持 HiRoute 服务运行。

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

## 用 CLI 或让 Agent 帮你配置

运行 `hiroute quickstart` 查看服务状态、准确的配置目标和已有来源/路由；选定后加 `--context CONTEXT_ID --plan PLAN_ID`。这个入口只读状态，并告诉你还缺什么。指定 Hermes 配置时可加 `--hermes-root /ABSOLUTE/EFFECTIVE/ROOT`。

[HiRoute CLI](/zh/docs/cli/) 提供配置预览、应用、任务执行与恢复步骤。同一管理 Skill 可以从受支持的 Agent 中管理另一个目标，调用方自己的模型可保持原生接入。需要录入凭证时，在自己的终端使用不回显提示，不要把密钥发到聊天。
