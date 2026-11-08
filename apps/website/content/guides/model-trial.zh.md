# 试用新模型

新模型发布后，不必一次让它接管全部工作。HiRoute 的试用让新模型先承担简单的轮次，熟悉的主力模型兜底；新模型某一步没做好、问题又关系到接下来的工作时，主力接手。之后根据你自己任务里的记录，逐步调整新模型承担的工作量。


<details class="agent-prompt" id="agent-cli">
<summary>交给 Agent：用 CLI 配置新模型试用</summary>

复制到能在目标主机运行命令的 Agent 输入框，替换方括号中的内容后发送。

```text
请用 HiRoute CLI 配置“新模型先接简单任务、主力兜底”的试用路由。
试用模型：[名称和来源]；主力模型：[名称和来源]
决策连接：[已保存的名称；没有则协助接入兼容的决策模型]
目标 Agent：[名称和准确目录或 profile]
新路由名称：[名称]；验证任务：[选填；空则不调用模型]

1. 用 quickstart、compute list 和 decision connections options 解析当前目标和可复用资源；缺少连接时先按 CLI 流程补齐。普通 Chat 模型不能直接冒充决策模型，也不要改成启发式规则来替代这次带评分的试用。
2. 读取 hiroute schema show --command-id routing.options --include-schema 和 routing.preview 的 schema。用 routing options 的 authoring 返回完整编辑器，构造 smart_saving 路由：试用模型放常规组，主力放主力组，选择明确保存的决策连接版本。
3. 通过当前结构化请求设置 smart.judgment.degree.simple_threshold_millis 为 900（“试用”档），保留适用的低分保护和其他默认项。不要发明 --trust 参数，也不要直接发布简化参数默认的“熟悉”档。核对并填写适合我任务的胜任评分标准。
4. 用 routing preview --request-file PREVIEW_FILE --apply-file APPLY_FILE 预览完整配置，再 routing apply --request-file APPLY_FILE 发布。预览文件与 Apply 文件分开；发布后核对实际保存的阈值和模型组。
5. 用 agents connect preview/apply 只映射所选目标的模型路由，保留任务路由、管理 Skill 和原生权限。给出加载方式；仅执行上面填写的验证任务，并核对真实记录。
6. 输出计划 ID、发布版本、目标 context、加载指引和验证状态；暂不根据少量样本自动上调信任度。

执行约定：先读取当前配置、命令 --help 和所需 schema，使用实际返回的 ID、版本与协议。先 Preview，核对改动符合以上目标后 Apply；只在缺少必要选择或超出目标时问我。保留私有 Apply 文件和提交标识，响应不确定先查询原操作，不重新生成提交。凭证只走受保护输入，由我在自己的终端录入，不进入聊天、命令参数或普通文件。最后分别报告：已保存的配置、需要加载的原生设置、真实执行证据或待验证项。
```

命令格式、凭证接入和恢复细节见 [HiRoute CLI](/zh/docs/cli/)。

</details>

<details class="agent-prompt">
<summary>交给 Agent：读取表现和回放信任度（CLI）</summary>

复制到能在目标主机运行命令的 Agent 输入框，替换方括号中的内容后发送。

```text
请只读评估 HiRoute 试用路由 [计划名称或 ID]，暂不发布任何改动，也不调用模型。
1. 用 routing list/show 解析计划和当前发布版本，读取 observation plan-trust --help，以及 schema show --command-id observation.plan-trust.replay --include-schema。
2. 查询 hiroute observation plan-trust --plan-id PLAN_ID --revision active --output json，并读取 observation plan-quality samples 的评分覆盖和样本。
3. 用 hiroute observation plan-trust replay --plan-id PLAN_ID --revision active --thresholds-millis 900,800,650,500 --output json 比较几个档位的历史分配；再读 observation plan-trust suggestions。
4. 报告样本量、评分覆盖、胜任表现、低分保护、费用估算及未知项。回放只是历史估算，不能推断未来成功率；无建议或证据不足时如实说明。
5. 给出保持、上调、下调或分工的理由。如我后续要求应用，再读取最新配置、保留完整恢复基线并重新 Preview/Apply。
```

命令格式、凭证接入和恢复细节见 [HiRoute CLI](/zh/docs/cli/)。

</details>

## 从首页开始

首页的“新模型”列出由 hiroute.ai 维护的推荐，每个模型会标明能否直接试用：

- **已连接，可以直接试**：选择“试用”。
- **需要先连接**：选择“接入并试用”，在试用流程里直接完成接入。
- 想试的模型不在推荐里时，选择“试用其他模型”，从已接入的模型中挑选。

试用流程分四步：选择模型、核对并发布、映射 Agent、开始使用。

<figure>
  <a href="/guide-assets/trial-entry-zh.webp"><img src="/guide-assets/trial-entry-zh.webp" alt="首页新模型试用卡片" width="1200" height="900" loading="lazy" /></a>
  <figcaption>试用位是新模型的统一入口：已有连接可直接试，需要连接时在流程内完成。推荐列表会更新，图中模型为示例。</figcaption>
</figure>

## 1. 选择模型

- **要试用的模型**：可以选已接入的模型，也可以在这里复用本机订阅、选择已保存的 API 模型，或连接其他接入点。
- **主力模型**：负责复杂任务和需要保护的轮次。只有当已保存的配置明确指向一个模型时，才会自动填入当前主力。
- **决策连接**：试用需要给每个阶段打胜任分，所以要选择一个决策模型，不提供启发式规则。还没有决策模型时，可以在这里新增，草稿会保留。
- **接入的 Agent**：选择要使用这条路由的 Agent。
- **名称与起始信任度**：起始信任度默认是“试用”，新模型最初只承担最简单的工作，之后可以随时调整。

<figure>
  <a href="/guide-assets/trial-setup-zh.webp"><img src="/guide-assets/trial-setup-zh.webp" alt="试用流程第一步，选择要试用的模型" width="1200" height="900" loading="lazy" /></a>
  <figcaption>先选择试用模型，再向下选择主力、决策连接和 Agent。页面底部的“检查改动”进入下一步。</figcaption>
</figure>

## 2. 核对并发布

发布前会列出全部改动。核对时可以修改胜任评分标准实际使用的提示词。选择“保存并发布路由”后，路由才开始处理新请求。

## 3. 映射 Agent

把刚发布的路由映射到 Agent。这一步只调整所选的模型路由，权限、工具审批和任务协作保持不变。

- **Codex**：推荐“按需使用 · 仅 CLI”，这条路由会成为 HiRoute profile 的默认模型，普通 Codex 和 Desktop 保持原配置。如果已经选择“设为默认”，映射后重新启动对应的 Codex CLI 或 Desktop。
- **Claude Code**：选择要映射到哪些原生预设（Opus、Sonnet、Haiku）。
- 其他 Agent 按页面提示在模型选择中选用这条路由。

离开试用流程后，也可以在“智能路由”页面继续映射。

## 4. 开始使用

按页面给出的启动方式开启一个新会话。Codex 按需使用时，复制启动命令，在安装 Codex 的电脑的终端里运行；直接运行普通 Codex 或 Desktop 不会使用这份试用配置。

先从范围明确的小任务开始，然后在“会话”的路由时间线里观察：每一轮实际用了哪个模型、为什么这样选、评分多少，以及低分保护在哪里触发、在哪里解除。费用按 API 等价价格估算。

## 根据证据调整信任度

信任度就是难度门槛：越往右，越多简单工作交给新模型。除了“试用、熟悉、信任、放手、托付”五个档位，也可以填精确值。即使调到“托付”，适用的低分保护仍然生效。

在路由编辑器的“信任度”中调整时，“用你的记录回放此设置”会用当前版本的历史记录估算，换成新档位后有多少阶段会交给常规模型。这是对历史分配的估算，不是对未来的预测。

<details>
<summary>什么时候会出现信任度调整建议？</summary>

证据足够时，首页“需要你处理的事”会给出建议。建议只看当前发布版本的记录：

- **可以上调**：至少 30 个有完整评分的阶段、来自至少 3 个会话，评分覆盖至少六成，其中至少 85% 达到胜任下限。期间模型有更新公告时，只统计公告之后的记录。
- **建议下调**：至少 30 个完整评分的阶段里，达到胜任下限的不到 70%。已经在最谨慎的档位时，会改为建议回到之前的路由，或建议更换常规模型。
- **可以分工**：信任度已经到“放手”或更右、表现仍然稳定时，可以按任务类别分工，让常规模型接手其中一类任务。

</details>

建议只是建议。打开后会生成一份独立的草稿，保存和发布仍由你确认，Agent 映射也不会自动改变。没有建议时，保持当前档位就是正常结果。

路由的“模型表现”汇总每个模型的阶段数、胜任率、评分覆盖和费用估算，可以对照建议一起看。供应商可能在同一个模型名下更新模型，记录可能混合多个版本。

<figure>
  <a href="/decision-assets/readme-report-zh-CN.png"><img src="/decision-assets/readme-report-zh-CN.png" alt="路由的模型表现报告，展示评分覆盖与费用估算" loading="lazy" /></a>
  <figcaption>在“智能路由 → 模型表现”核对质量和成本。图中数据用于说明界面，不代表模型基准测试或节省承诺。</figcaption>
</figure>

**怎样算跑通了？** 已发布路由、Agent 已加载它，并在“会话”里看到一次真实执行。之后再积累多个任务的证据，决定是否提高信任度。
