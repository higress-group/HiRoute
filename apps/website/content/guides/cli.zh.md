# HiRoute CLI

HiRoute CLI 让你在终端连接模型、配置路由、接入 Agent、查看会话和委派任务。它连接本机的 HiRoute 服务，与 macOS / Windows 桌面应用、Linux Web UI 使用相同的配置和执行记录，也可以独立用于终端和自动化环境。

本文中的命令名、参数和 JSON 字段均保留原文。例如 `worker` 管理委派任务，`schema` 查看命令接受和返回的数据格式。

## 安装与检查

- macOS 桌面应用：打开“设置” → “CLI” → “终端入口”，选择“安装”。
- Windows 桌面应用（0.5.0+）：当前用户安装包自带 CLI，可选择将入口加入 PATH。按 [Windows 安装指南](/docs/install-windows/)安装后打开新终端。
- Linux：按 [Linux Web UI 与 CLI 安装](/docs/install-linux/)使用官网一行命令安装，再手动启动服务。

如果 PATH 不包含入口目录，把下面一行加入 shell 配置后重新打开终端：

```sh
export PATH="$HOME/.local/bin:$PATH"
```

检查入口和可用命令：

```sh
hiroute --help
hiroute schema list --output json
hiroute schema show --command-id worker.exec --output json
```

安装 CLI 后，还需要让本机服务保持运行。如果命令提示服务不可用，桌面应用用户先打开应用；独立服务版（standalone）用户运行 `hiroute service status`，并在需要时运行 `hiroute service start`。CLI 不会替你自动启动服务或重放失败请求。

## CLI Quickstart：接入、确认和调整

终端和任意已安装 HiRoute 管理 Skill 的 Agent 都使用同一条路径。调用方可以管理另一个 Agent，自己的模型不必接入 HiRoute。模型路由、管理 Skill 和任务委派保持独立。

```sh
hiroute quickstart
hiroute quickstart --context CONTEXT_ID --plan PLAN_ID
```

Quickstart 只读当前服务、准确的配置目标、可复用的模型来源和路由，并列出待选择项及下一步命令。它不会启动服务、准备凭据、保存配置、重启 Agent 或调用模型。指定 Hermes 配置时加 `--hermes-root /ABSOLUTE/EFFECTIVE/ROOT`；所选目录不可用时，应保留该目标排查，不能自动换到另一个配置。

1. 先用 `compute list` 复用已保存来源；没有合适来源时，扫描接入点并显式调用 `compute connection prepare` 准备凭据。当前接入点扫描使用 `compute scan --request-stdin`，输入 `{"schema":"hiroute.compute-scan-request/v2"}`，Hermes 加上选定的 `native_target`。准备结果包含候选引用、版本和可选择模型。
2. 预览所选模型，再提交生成的请求文件：

   ```sh
   hiroute compute connection preview --candidate CANDIDATE_REF --candidate-revision REV --all-selectable --apply-file source-apply.json
   hiroute compute connection apply --request-file source-apply.json
   ```

   只选部分模型时重复使用 `--model-ref REF`。`--all-selectable` 会固定为本次候选中的具体模型列表，不会自动扩展到后来发现的模型；没有可选模型时不能保存。准备凭据、保存、供应商检查和原生 Agent 的真实请求是不同效果。
3. 用 `routing options --request-stdin` 输入 `{}`，获取当前绑定 ID。可选择 `--mode fixed_model --model BINDING_ID`，也可直接创建按难度分流组合：

   ```sh
   hiroute routing preview --name '日常省钱' --purpose '处理日常和复杂开发任务' --mode smart_saving --economy ECONOMY_BINDING --primary PRIMARY_BINDING --decision local_rules --apply-file route-apply.json
   hiroute routing apply --request-file route-apply.json
   ```

   将示例 ID 替换为当前返回的值；重复模型组参数可以指定顺序。明确选择 `--decision local_rules` 或 `--decision-service ID@REVISION`，普通 Chat 执行模型不等于兼容的决策服务。其余编辑器设置由共享后端默认值补全。调整已有路由时使用 `routing preview --plan PLAN_ID`，只传要改的选项，并生成新文件；其他保存设置会保留。
4. 使用 `agents connect preview --context CONTEXT_ID --plan PLAN_ID`，按 `--help` 补充原生选择，并加 `--apply-file connect-apply.json`。Hermes 明确选择协议；Codex 明确选择原生模型保留及默认模型策略；Claude 明确选择预设；Qoder/Pi/DSH 添加模型入口，不会声称已修改全局默认。`--management enable` 和 `--collaboration explicit|delegate_by_default` 分别配置独立 Skill，不要求调用方模型接入。检查预览后运行 `agents connect apply --request-file connect-apply.json`。
5. 查看 Operation 和 `agents connect status --effective --context CONTEXT_ID --plan PLAN_ID`，按返回的指引加载原生配置、选择模型。文件保存不等于运行中 Agent 已加载，供应商直连检查也不等于 Agent 使用了该路由。未经模型调用授权，验证保持待完成。匹配的请求记录仅说明该请求和会话的路由、实际模型、结果及已报告用量；价格未知不等于零成本。

Apply 文件为新建的私有文件，不会覆盖已有文件，包含准确请求和稳定幂等键。保留 `submission.operation_lookup`，响应不确定时用 `operations find` 查找，或原样重放同一个文件；不要重新生成 Preview 和新键来重试可能已接受的工作。再次运行 Quickstart 可复用已完成步骤。只恢复一项能力时，用 `agents restore preview --context CONTEXT_ID --facet management_skill --apply-file restore-apply.json`，再执行 `agents restore apply --request-file restore-apply.json`；`model`、`collaboration` 分别选择其他能力。恢复保留无关数据，运行中的原生会话可能仍需重新加载或选择模型。

通过命令族和子命令的 `--help` 查看分支，用 `schema show --command-id routing.preview --include-schema` 获取完整的已安装请求/响应定义；不加该参数仍返回命令描述。JSON 输入与简化参数二选一，受支持的 Preview 两种形式均可加 `--apply-file`。Desktop 的驻留服务前置条件需由 Desktop 完成，不能在 CLI 中伪造。

业务命令在 stdout 输出一个完整 JSON 文档。应单独保存 stderr 和实际退出码；Operation 被接受不代表执行完成。若 Agent 终端工具合并输出：

```sh
(
  hiroute quickstart > quickstart.stdout.json 2> quickstart.stderr.txt
  hiroute_exit=$?
  printf '%s\n' "$hiroute_exit" > quickstart.exit.txt
  exit "$hiroute_exit"
)
```

分别读取完整 JSON、诊断文本和保存的退出码，不能只解析第一行，也不能把后续命令的状态当作 HiRoute 退出码。退出码 2 表示输入无效；`--error-details` 提供有限的字段诊断。个人经验笔记应保存在托管 Skill 之外。

### 通过 CLI 接入决策模型

先运行 `hiroute decision services options`，优先复用明确选定的已保存 `ID@REVISION`。
“已保存”不代表当前可用。安装包预设包括百炼 Token Plan/业务空间、OpenRouter、TypeSafe，
以及 System One 兼容和自定义扩展；预设不保证供应商当前可用、账号权限或价格。
按返回的必填项补充工作空间/兼容接入的地址和模型。普通 Chat 执行模型不是决策服务，
本地规则仍是可以明确选择的另一条路径。

凭证由用户在聊天之外录入。在 HiRoute 所在机器上，使用自己的终端（例如独立 SSH 会话）执行，
将候选引用换成新的非敏感标识：

```sh
hiroute protected-input register --candidate CANDIDATE_REF --prompt --format bearer
```

在不回显的提示中粘贴裸 API Key，HiRoute 自动加上 `Bearer`，只输出登记结果。
不要把密钥发给 Agent、输入其终端工具，也不要放进参数、环境变量或普通 JSON/文件。
自定义完整请求头值使用 `--format raw`。取消或没有控制终端时不会保存连接。
仅将登记是否成功告诉 Agent，它可用同一引用继续；Desktop 用户也可使用已有密码输入框。

```sh
hiroute decision services apply --id daily-decision --name '日常决策' --provider typesafe --candidate CANDIDATE_REF --apply-file decision-apply.json
hiroute decision services apply --request-file decision-apply.json
hiroute decision services test --service daily-decision@SAVED_REVISION
hiroute routing preview --plan PLAN_ID --decision-service daily-decision@SAVED_REVISION --apply-file decision-route-apply.json
hiroute routing apply --request-file decision-route-apply.json
```

选择实际供应商，并按需补充 `--endpoint`、`--model`。自定义扩展使用 `--provider custom --endpoint URL`，
选择受保护候选及可选 `--header NAME`，或明确 `--no-auth`。新连接须指定 ID 和名称；
修改时用 `--id` 加要改的字段，保留其他内容。更换地址须明确提供新凭证，或选择自定义免认证。
逐项检查 Preview 和 Operation，并用实际保存版本替换示例版本。保留私有 Apply 文件及查找标识，
响应不确定时原样重试；复用已完成步骤，释放未用的候选引用。

测试是可能计费的显式探测，要查看 `data.outcome` 和 `failure_code`，不能只看 CLI 退出码。
通过只证明本次连接探测，不代表语义判断质量、执行模型可用或 Agent 已加载。
发布后用匹配请求核对实际决策来源、执行组、模型和结果；手动把主力模型放进经济组不能证明复杂分流。
空的自定义关键词不代表内置规则失效，自然复杂任务未命中时应如实报告；取消和传输失败也要计入结果。

恢复或重接管理者自己的模型路由前，先准备独立原生接入或外部操作者接续。
恢复成功可能撤销运行中进程仍在使用的授权；文件恢复不等于已重新加载，应在执行前安排接续。

## 查找命令与请求格式

服务启停、Gateway 管理和受保护输入等本机管理命令，用 `--help` 查看。顶层帮助列出命令分类，子命令帮助说明具体用法：

```sh
hiroute --help
hiroute service --help
hiroute gateway --help
hiroute protected-input --help
```

模型、路由、Agent、会话和委派任务等命令，还提供 schema，说明当前版本的请求字段和返回结构。例如，先查找路由保存命令，再查看其数据格式和用法：

```sh
hiroute schema list --output json
hiroute schema show --command-id routing.apply --include-schema --output json
hiroute routing apply --help
```

`schema list` 不包含前一类本机管理命令。准备 JSON 请求时，以当前安装返回的 schema 为准，避免直接套用其他版本的字段。

## 管理模型、路由与 Agent

常用命令如下。斜杠表示多个子命令，例如 `compute scan/list/show` 分别指 `hiroute compute scan`、`hiroute compute list` 和 `hiroute compute show`，不是要输入斜杠：

- `compute scan/list/show` 发现和读取模型来源；`compute connection options/test/preview/apply/authorize` 检查并保存连接。
- `models show` 查看候选模型能力。
- `decision services list/apply/test` 管理决策模型或自定义扩展的已保存连接版本。
- `routing options/list/show/preview/apply` 创建、更新并发布智能路由。
- `agents scan/list/check` 发现本机 Agent；`agents connect preview/apply/status` 接入，`agents restore preview/apply` 恢复。
- `operations find/get` 在保存请求已发出、但结果未收到时，查询原操作是否已经完成。

配置保存分为预览和提交：

1. 先查看该命令的 `schema show`、`options` 和具体子命令的 `--help`，准备请求。
2. 用 `preview` 查看变更，保留返回的摘要和版本号（revision）。
3. 用 `apply` 提交同一份变更，带上这些值和固定的幂等键（idempotency key，用来识别同一次提交，避免重复执行）。

模型、路由和 Agent 分别有 `preview`、`apply` 命令。决策连接是例外：两个步骤都使用 `decision services apply`，见下一节。密码或 API Key 通过 `protected-input` 传入，不放进普通 JSON、命令参数或日志。

## 管理决策连接

CLI 保留 `decision services` 命令名；桌面应用将这些连接放在“模型 → 决策模型”中。内置决策模型和自定义扩展独立于实际执行任务的通用模型：

```sh
hiroute decision services list --output json
hiroute schema show --command-id decision.services.apply --output json
hiroute decision services apply --help
hiroute decision services apply --request-stdin --output json < decision-preview-request.json
hiroute decision services apply --request-stdin --output json < decision-apply-request.json
hiroute decision services test --request-stdin --output json < decision-test-request.json
```

以上三个 JSON 文件需要先按当前 schema 准备；命令示例不会自动创建它们。没有单独的 `decision services preview` 命令，`apply` 根据请求内容完成两步操作：

| 步骤 | 请求中需要提供的内容 |
| --- | --- |
| 预览 | `{schema_version, spec}`；不保存配置 |
| 保存 | 同一 `schema_version`；以预览返回的 `data.normalized_spec` 作为 `spec`；以 `data.change_digest` 作为 `accept_digest`；带上 `data.expected_revisions` 和固定幂等键 |

`spec.desired_state` 包含连接 ID、`expected_revision` 和完整的 `service`。设置新凭证时，还需提供受保护输入的 `input_slot`。`service: null` 表示删除连接；仍被路由等记录引用的连接不能删除。

列表返回各连接的最新版本，路由使用的是你选中的完整已保存版本。保存时会核对 ID、revision 和内容，旧版本仍可用于发布。路由编辑数据中的相关字段为：

| 配置 | 字段 |
| --- | --- |
| 按难度分流的连接与判断设置 | `smart.classifier`、`smart.judgment` |
| 自定义分支的连接与计划默认判断设置 | `branch_routing.classifier`、`branch_routing.judgment` |
| 分支的常规与主力候选 | `candidates`、`primary_candidates`；没有主力组时后者为空数组 |
| 分支独立判断设置 | 可选的完整 `judgment`；省略时跟随计划，设置后独立，移除后恢复跟随计划 |

保存 r2 不会改变已固定 r1 的路由；需要选择新版本并通过 `routing preview/apply` 发布。测试请求的 schema 为 `hiroute.classifier-decision-test/v1`，`classifier` 为 `{kind: "decision_service", service: <完整已保存版本>}`。它只发送固定示例，检查指定的连接版本，不读取真实会话，也不产生实际任务的评分。检查 `data.outcome` 与 `data.failure_code`，不能仅凭命令退出成功判断测试通过。

自定义扩展遵守 [自定义扩展 API](/docs/decision-api/)；内置供应商配置和接口映射见 [决策模型接入指南](/docs/decision-extensions/)。可选的 [Jev 自托管参考扩展](/docs/jev-decider/) 提供自行部署的示例。完整桌面配置步骤见 [使用智能模型路由](/docs/model-routing/)。

## 查询会话与运行表现

```sh
hiroute sessions list --include-unlinked --limit 50 --output json
hiroute sessions show <SESSION_ID> --output json
hiroute sessions receipt <RECEIPT_ID> --output json
hiroute sessions status --output json
hiroute value show --routing <PLAN_ID> --session <SESSION_ID> --output json
hiroute observation plan-quality samples --plan-id <PLAN_ID> --output json
hiroute observation plan-quality samples --session-id <SESSION_ID> --limit 50 --output json
```

默认会话查询返回执行记录和时间线，不返回对话正文。`receipt` 展示实际路由、模型和供应商报告的 token 用量；缺少可靠价格时，金额显示为未知，不会当成零费用。

`observation plan-quality samples` 至少需要 `--plan-id` 或 `--session-id` 之一，用来查询指定计划或会话的执行阶段与评分。`branch_execution` 记录实际任务分支、模型组、组内候选和当时判断标准；本轮选择原因与后续阶段评分分别保存。`--competence below-floor|meets-floor` 按阶段冻结的胜任下限筛选，`--unrated` 查看缺失或部分评分；未评分不等于零。返回游标用于继续分页，此查询不会调用模型或返回受保护的对话正文。

## 发现执行器和计划

```sh
hiroute worker executors
hiroute worker plans
```

`worker executors` 列出本机执行 Agent 的可用状态；`worker plans` 列出当前 Agent 获准使用的已发布计划。选择通过依赖检查的执行 Agent，并使用命令返回的计划 ID，不要用显示名称代替。

## 启动任务

```sh
hiroute worker exec \
  --plan <PLAN_ID> \
  --cwd /absolute/path/to/project \
  --title "检查失败测试" \
  --submission-key my-check-001 \
  -- "分析失败原因，提出最小修复并运行相关检查"
```

输入必须来自 `--` 后的文本、`--file` 文件或标准输入三者之一。`--cwd` 会转换为绝对路径，但它不是目录授权或并发锁。

`--submission-key` 是调用者选择的幂等键。新任务应使用新的值。若连接或进程中断后无法确定这次任务是否被接受，使用原来的键查询，不要换一个新键直接重试：

```sh
hiroute worker status --submission my-check-001 --operation start
```

## 查看进度与结果

使用启动结果中的 run ID（这次运行的标识）：

```sh
hiroute worker status --run <RUN_ID>
hiroute worker wait --run <RUN_ID>
hiroute worker result --run <RUN_ID>
```

`wait` 到达等待时限就返回，任务仍可在后台运行；这不代表任务被取消。`result` 支持指定偏移量（offset）和最大字节数，分次读取较大的结果。

## 继续或取消

继续任务时，同时提供 task ID（任务标识）和该任务最新的 run ID，避免基于过时的运行结果继续：

```sh
hiroute worker continue \
  --task <TASK_ID> \
  --expected-latest-run <RUN_ID> \
  --submission-key my-check-002 \
  -- "根据测试结果完成修复"
```

取消指定的这次运行：

```sh
hiroute worker cancel --run <RUN_ID> --reason user-requested
```

取消不会撤销已经写入的文件或已经发生的外部操作。

## 机器可读输出

管理业务命令默认输出 JSON，并接受 JSON 输出模式。Worker 命令按各自帮助支持 `text|json|quiet`，通常默认文本；自动化应显式使用 `--output json`。具体模式以已安装子命令的 `--help` 为准。

`schema show --include-schema` 为受支持的管理命令返回完整定义；不加参数时只返回描述。参数值保持原文，不翻译成中文。

自动化只应调用当前安装已提供的命令。规划中的功能不代表已经可以调用，也不要把命令数量写死在脚本里。

## 恢复完整计划配置

修改已有计划前，先把完整成功响应保存到新的私有文件：

```sh
(umask 077; set -C; hiroute routing show PLAN_ID --output json > /ABS/plan-before.json)
```

检查实际退出码和完整响应。在 Apply 前，把已填入 CLI 绝对路径、真实计划 ID 和独一无二绝对文件
路径的恢复命令交给用户；管理 Agent 无法继续时，用户也能在独立终端执行：

```sh
hiroute routing preview --plan PLAN_ID --restore-from /ABS/plan-before.json --apply-file /ABS/restore-apply.json
hiroute routing apply --request-file /ABS/restore-apply.json
hiroute routing show PLAN_ID --output json
```

第一条只预览：读取已保存的公开响应，使用当前 head revision 和既有完整编辑器投影，再生成普通
精确 Apply 文件。`--restore-from` 不能混用单字段编辑参数或请求 JSON。失败、不完整、格式错误或
不同目标的响应会被拒绝；缺失依赖和版本冲突不会被掩盖。不确定是否已提交时查询或重放同一 Apply；
确定未接受且已过期的请求才生成新的 Preview 和文件，不跳过版本检查。

恢复后比较全部 `data.desired` 字段：候选及顺序、推理档、决策服务和判断设置、上下文/限制、能力要求、
模式/委派/work 等，并核对别名；版本和发布编号会正常前进。更换候选可能采用角色默认值，仅换回
模型名称并不恢复原来的推理档。来源、凭证和 Agent 原生能力保留各自恢复所有权；计划响应不是整套
安装数据的备份。

原生接入应给出已填好实际参数的 `agents restore preview --context ... --facet ... --apply-file ...`
和 `agents restore apply --request-file ...` 命令、原生目录/选择参数及已有恢复点。可能撤销调用方授权
的恢复或重接入前，先安排独立接续；原生加载/当前会话选择单独报告。


## 分析历史会话和 API 等价省钱效果

本机 CLI 复用同一系统用户的 Local Control 权限。显式读取消息、工具内容和搜索不再需要另一个 Desktop 授权票据；显式传入的票据仍会校验。模型网关凭证不能用于管理。先查询：

```sh
hiroute sessions list --include-unlinked --limit 50 --output json
hiroute sessions show SESSION_ID --content messages-and-tools --output json
hiroute sessions receipt RECEIPT_ID --output json
hiroute observation plan-quality samples --plan-id PLAN_ID --output json
```

使用实际返回的 ID。`schema show --command-id sessions.show --include-schema` 给出 V2 时间线、请求内容目录、上下文来源和内容分页查询。`sessions list --request-file QUERY_JSON` 支持 search/session_search；`sessions receipt --request-file QUERY_JSON` 支持请求事实。沿用相同时间和筛选条件跟进 `next_cursor`。将任务内容、工具结果和回答与实际决策来源、简单概率/阈值、执行组、胜任度、模型/推理档和发布版本关联分析。缺失采集、过期内容或索引尚未完成只是证据缺口。调整策略前先说明依据，保留前述完整恢复命令，再审阅和应用。

Coding Plan 的订阅支出与 API 等价估算分开。选定实际基准 Binding（通常为计划主力模型）和币种：

```sh
hiroute value show --routing PLAN_ID --period 7d --basis api-equivalent --reference-binding PRIMARY_BINDING --currency CNY --output json
```

该显式视图用**当前来源下的 API 等价单价**计算已记录用量，计入重试，列出基准、价格来源/时间、路由估算成本、基准估算成本和带正负号的估算差额。它不会改写历史回执，也不证明回答质量相同、订阅账单或现金节省。不完整的总额和差额保持 null；可比较子集的小计会同时报告完整请求数。负差额不会截成零。过期、缺少用量或单价均降低覆盖率；查询范围过大时可缩短区间；该视图暂不支持按日分组。普通 `value show` 保留历史估值含义。

缺单价时，读取已安装的 `prices.effective` / `prices.override.preview` schema，以实际 Binding、币种和 `valuation_kind: "api_equivalent"` 查询，使用返回的 `edit_context` 修订号生成价格 Preview。手动 rates 单位是每百万 token 的整数货币微单位，输入/输出/缓存分别设置；使用有依据的价格或让用户提供这些非敏感数值，不猜测单价。保留原设置，提前给出填好参数的价格恢复 Preview/Apply 命令；原先没有手动覆盖时可用 `follow_catalog` 恢复目录价格。使用 Preview 返回的完整 spec/digest/revisions 和同一个提交 key 应用。订阅也能设置 API 等价参考价，不改变权益或普通计费分类；免费、订阅、付费和未知保持区别。

若参考计价中缓存创建按普通输入单价收费，可将参考缓存写入价设为输入价。此时金额与未上报的普通输入/缓存创建拆分无关，HiRoute 可合并同价计算桶，但不会补造或改写用量；两者单价不同时仍需要该细分计数。

自定义扩展直接返回类别和模型组；路由不使用 HiRoute 阈值或评分标准，草稿保留切回内置连接时需要的设置。可选阶段评分只用于观察。
