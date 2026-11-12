# Agent 智能路由

留在你熟悉的 Agent 里，把任务交给更合适的**模型 × Harness**。Harness 是 Agent 的执行工具，影响它如何使用工具、管理上下文和完成工作。HiRoute 的 Agent 路由在界面中叫“任务路由”：它选择执行 Agent，而模型路由决定执行时用哪些模型。

| 你日常使用的主 Agent | 想使用的能力 | 可发布的任务路由示例 |
| --- | --- | --- |
| Codex | 用 DeepSeek Harness（DSH）运行 DeepSeek 模型 | “DSH 开发”：接入可用的 DeepSeek 模型，执行 Agent 选 DeepSeek Harness |
| Codex CLI 或 Claude Code | 让经济模型先检查日志和测试 | [“Pi 快速检查”](#lightweight-pi)，选经济模型和 Pi 执行 Agent |
| Hermes | 用 Codex 完成编码与测试 | “Codex 编码”：选好执行模型，执行 Agent 选 Codex CLI |

主 Agent 可以保留原来的模型与登录，按需启用任务委派。下面以桌面应用说明，Linux Web UI 操作相同。

**推荐通过产品内置委派技能使用任务路由。** 按下方步骤在“Agent → 任务路由”中启用，之后只需向主 Agent 说明任务目标，技能负责发现路由、委派和取回结果。配置路由也可以交给已[开启管理技能](/zh/docs/#management-skill)的 Agent；界面与 CLI 共用同一份配置和任务记录。

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>

复制以下指引到 Agent 对话中，再说明你想完成的操作。内容包含文档链接、内置技能入口和 CLI 线索。

<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
请先阅读以下文档，结合我在对话中提出的目标，用 HiRoute CLI 完成“Agent 智能路由”相关操作。目标不明确时，只询问必要信息。

本页文档：https://hiroute.ai/zh/docs/task-routing/
CLI 参考：https://hiroute.ai/zh/docs/cli/

请使用产品内置的 HiRoute 管理技能；尚未启用时，按图示引导我在“Agent → HiRoute 管理 → 启用”中开启：https://hiroute.ai/zh/docs/#management-skill
涉及任务委派时，请使用产品内置委派技能；尚未启用时，引导我在“Agent → 任务路由 → 启用”中开启，按已安装技能处理委派：https://hiroute.ai/zh/docs/task-routing/#delegation-skill

操作线索（按本次目标选用；下面均为 hiroute 子命令）：
- 选择执行 Agent、开放可委派路由：worker executors · worker plans · routing preview + --delegation enable / --worker-harness / --worker-protocol → routing apply
- 启用内置委派技能：agents connect preview --context CONTEXT_ID --collaboration explicit → agents connect apply
- 由内置技能发起并跟进任务：worker exec · worker status · worker wait · worker result
- 查看任务记录、继续或取消：worker list · worker read · worker continue · worker cancel

只开启委派技能时不传 --plan，保留主 Agent 的模型接入。执行器与协议一起选择；任务执行、跟进和恢复遵循产品内置委派技能。

界面与 CLI 连接同一个 HiRoute 服务，共用配置和执行记录。先读取现状并复用已有配置；参数以本机命令 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。按内置技能完成所需预览、应用与核验，最后说明结果。
```

</details>
</aside>

## 开始前

[按 Agent 接入](/zh/docs/agent-integrations/) 有按来源组织的组合示例，包括用 Qoder 模型让 Pi 执行，以及让 Hermes 委派给 Codex。

在 HiRoute 服务所在主机安装目标执行 Agent，并按 [接入模型](/zh/docs/models/) 准备执行模型。先选一条固定模型路由即可，之后也能组合 [按难度分流](/zh/docs/model-routing/)。

## 适合委派什么

委派适合目标、工作目录和交付物相对清楚、可以独立执行的工作，例如实现一个局部功能、检查一组测试失败或整理一份研究结果。主 Agent 负责发现已开放的计划、选择合适场景并取回结果。

协调本身、计划发现、状态查询、等待和结果总结不应再次委派。执行 Agent 也不能递归发起另一层委派。

<h3 id="lightweight-pi">用轻量 Pi 降低委派任务的 token 成本</h3>

用 Pi 做日志初筛、找调用位置或整理测试缺口时，把明确的目标和相关文件交给独立执行任务。Pi 是轻量 Harness，可以用较短的系统提示开始工作。给它配支持工具调用的经济模型，大段输入和工具往返由低单价模型处理，主 Agent 只读所需的简短结果。对重复的日志和代码检查，这样能明显减少主力模型的 token 开销。[Pi 官方说明](https://pi.dev/)

在 HiRoute 服务所在主机准备 Pi 和 Node.js。新建“Pi 快速检查”固定模型路由，选一个经济模型，再到“更多设置 → 任务委派”开启“允许委派任务给执行 Agent”，选 Pi 并检测安装。用途可填“读取指定文件，列出关键发现和来源位置”。启用路由后，[在主 Agent 开启任务路由](#delegation-skill)。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-pi-worker-zh.webp"><img src="/guide-assets/agent-guide-pi-worker-zh.webp" alt="HiRoute 智能路由中开启任务委派并选择 Pi 执行 Agent" width="1200" height="900" loading="lazy" /></a>
  <figcaption>开启任务委派，选择 Pi 执行，然后检测本机的 Pi 和 Node.js。</figcaption>
</figure>

> 使用 HiRoute，让“Pi 快速检查”读这次失败的测试日志。返回第一个错误、相关文件和复现命令。

在“Agent → 任务记录”核对执行方，再到“会话 → 路由时间线”看主会话和委派任务各自使用的模型与 token。对比相似工作时，合计两边的用量和按量费用，找出最适合交给 Pi 的任务。

## 1. 配置一条可委派的路由

1. 打开“智能路由”，新建或编辑一份计划。
2. 展开“更多设置”，在“任务委派”中打开“允许委派任务给执行 Agent”。
3. 填写使用场景。开启任务委派后，使用场景必须填写：说明适合交给这条路由的任务和预期交付，例如“实现范围明确的代码修改，并运行相关测试”。主 Agent 根据名称和使用场景选择路由。
4. 选择一个执行 Agent：Codex CLI、Claude Code、Qoder CLI、Pi、DeepSeek Harness 或 OpenCode。可用性取决于本机安装和依赖检查。每份计划只选一个，执行失败时不会自动改用另一种 Agent。
5. 按页面提示检查所需执行环境，然后发布计划。

安装缺失不会阻止保存计划，但真正启动任务前必须让所选执行 Agent 通过依赖检查。

<figure>
  <a href="/guide-assets/routing-worker-zh.webp"><img src="/guide-assets/routing-worker-zh.webp" alt="路由编辑器展开任务委派，选择执行任务的 Agent" width="1200" height="900" loading="lazy" /></a>
  <figcaption>在路由中选执行方。示例界面使用 Codex CLI；DSH 路由在同一区域选择 DeepSeek Harness。</figcaption>
</figure>

<h2 id="delegation-skill">2. 开启主 Agent 的内置委派技能</h2>

打开“Agent”，选择日常使用的主 Agent，在“任务路由”中点击“启用”。Codex CLI 对应“Codex”，DSH Web 和 CLI 对应“DeepSeek Harness Web”。Claude Code、Qoder、Pi、Hermes 和 OpenCode 也可以发起委派。使用 Hermes 时，先选中它实际使用的配置目录。

1. 选择“委派时机”：“仅在明确要求时”，只有你要求执行、委派或交给 Worker 时才启动任务；“默认由 Agent 判断”，Agent 可以主动委派适合执行的工作，你仍可明确要求只回答、不执行。
2. 选择“启用任务路由”，并按提示完成任务委派技能检查。

任务路由只影响委派，不改变主 Agent 自己使用的模型。主 Agent 也可以不启用模型路由，继续使用原有模型和登录，只通过任务路由使用 HiRoute 计划。

可委派的任务路由在“智能路由”中管理；主 Agent 会发现已发布并允许任务委派的计划。

<figure>
  <a href="/guide-assets/agent-task-zh.webp"><img src="/guide-assets/agent-task-zh.webp" alt="Codex 任务路由对话框中选择委派时机" width="1200" height="900" loading="lazy" /></a>
  <figcaption>首次可选“仅在明确要求时”。已启用时按钮显示“保存配置”。</figcaption>
</figure>

## 3. 在日常对话中发起委派

先确保路由已发布、允许委派，且执行 Agent 的依赖检查通过。把下面的路由名和绝对路径替换成你自己的配置：

> 在 Codex 中：使用 HiRoute，把 `/absolute/path/to/project` 中这个解析器的回归修复委派给“DSH 开发”。定位失败、完成修复并运行相关测试，最后汇总修改与测试结果。

> 在 Hermes 中：使用 HiRoute，把 `/absolute/path/to/project` 的登录表单实现委派给“Codex 编码”。按现有组件实现并运行相关测试，最后汇总结果。

也可以用通用说法：

- 指定路由：“使用 HiRoute，将当前任务委派给【路由名称】执行。”将占位内容替换为“智能路由”中允许任务委派的路由名称。
- 由 Agent 选择：“使用 HiRoute 委派当前任务，选择合适的路由。”

若配置为默认由 Agent 判断，主 Agent 也可以对适合独立执行的工作主动选择计划。任务记录显示在“Agent”页面的“任务记录”中。

产品内置委派技能使用下文的 `worker` 命令完成这些操作；日常使用时直接向主 Agent 描述任务即可。

## 4. 查看进度并取回结果

打开“Agent → 任务记录”，选择任务，核对使用的路由、执行 Agent 和运行状态。让主 Agent 读取执行结果后，再检查实际改动和测试输出；配置已保存或任务已提交，都不等于任务已完成。

<figure>
  <a href="/guide-assets/tasks-zh.webp"><img src="/guide-assets/tasks-zh.webp" alt="Agent 任务记录展示任务列表、执行 Agent 和运行状态" width="1200" height="900" loading="lazy" /></a>
  <figcaption>任务记录示例：在这里跟进执行状态；任务结束后由主 Agent 取回并汇总结果。</figcaption>
</figure>

需要追加要求时，可以在同一任务上继续。失败时先查看原因再决定继续或重新发起；连接中断时按下方 CLI 说明查询原提交，避免重复启动。

## 用 CLI 执行与恢复

<details>
<summary>展开命令示例、提交标识与断线恢复</summary>

安装 [HiRoute CLI](/zh/docs/cli/) 后，可以先发现执行器和计划：

```sh
hiroute worker executors
hiroute worker plans
```

再启动任务：

```sh
hiroute worker exec \
  --plan <PLAN_ID> \
  --cwd /absolute/path/to/project \
  --title "修复解析器回归" \
  --submission-key parser-fix-001 \
  -- "定位失败、实现最小修复并运行相关测试"
```

`worker exec` 提交后会等待任务的下一次进展，最多 30 秒，然后返回当前状态和 run ID；任务已经成功结束时，会直接带上结果。任务继续在后台运行，不受这次等待影响。用 `--wait-timeout` 调整这段等待（1–30 秒），用 `--no-wait` 提交后立即返回；`--run-timeout` 设置任务本身的最长运行时间，默认 1 小时，最长 24 小时。

`--submission-key` 用来标识这次提交，新的任务应使用新的值。提交前保留它；任务被接受后，再保存返回的 task ID 和 run ID，用于查询进度、继续或取消。

如果连接中断，无法确定任务是否已被接受，先用原来的提交标识查询：

```sh
hiroute worker status --submission parser-fix-001 --operation start
```

不要换一个提交标识直接重试，以免启动重复任务。

任务进行中时，用 `hiroute worker status --run <RUN_ID>` 查看状态。任务结束后用 `hiroute worker result --run <RUN_ID>` 读取结果，需要补充要求时用 `hiroute worker continue` 在同一任务上继续。完整命令见 [HiRoute CLI](/zh/docs/cli/)。

</details>

## 权限和取消边界

委派任务使用 `approve-all` 权限策略运行，不逐项审批工具调用：所选执行 Agent 可以按自身能力读写文件、执行命令和访问网络。它不提供操作系统级沙箱，`--cwd` 只是任务的工作目录，不是目录授权。请只在可以接受这些操作的环境中委派任务，需要隔离时自行准备容器或独立账号等环境。

取消只停止指定的这次运行（run），不会撤销已经写入的文件或已完成的外部操作。多个任务使用同一工作目录时仍可能同时执行，请自行避免互相覆盖文件。
