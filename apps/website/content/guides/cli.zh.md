# HiRoute CLI

HiRoute CLI 让你在终端连接模型、配置路由、接入 Agent、查看会话和委派任务。它连接本机的 HiRoute 服务，与桌面应用使用相同的配置和执行记录，也可以独立管理 Linux 无界面版。

本文中的命令名、参数和 JSON 字段均保留原文。例如 `worker` 管理委派任务，`schema` 查看命令接受和返回的数据格式。

## 安装与检查

- macOS 桌面应用：打开“设置” → “CLI” → “终端入口”，选择“安装”。
- Linux 无界面版：按 [Linux 无界面版安装](/docs/install-linux/) 使用官网一行命令安装，再手动启动服务。

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
hiroute schema show --command-id routing.apply --output json
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
| 智能省钱的连接与判断设置 | `smart.classifier`、`smart.judgment` |
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

公开命令支持 `--output text|json|quiet`：

| 值 | 适用场景 |
| --- | --- |
| `text` | 默认值，供人在终端阅读 |
| `json` | 供脚本或 Agent 读取，按当前 schema 解析字段 |
| `quiet` | 精简输出，具体保留内容以该命令的 `--help` 为准 |

参数值应原样输入，不翻译成中文。用 `hiroute schema list` 和 `hiroute schema show` 查询当前版本的数据格式。

自动化只应调用当前安装已提供的命令。规划中的功能不代表已经可以调用，也不要把命令数量写死在脚本里。
