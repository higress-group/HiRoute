# HiRoute CLI

HiRoute CLI 是 HiRoute 本机服务的正式终端入口，可连接桌面应用，也可以作为 Linux 无界面版的完整管理界面。模型来源、智能路由、Agent 接入、会话观测和 Worker 任务都复用与桌面应用相同的生产路径，不存在第二套无界面控制面。

## 安装与检查

- macOS 桌面应用：打开“设置” → “CLI” → “终端入口”，选择“安装”。
- Linux 无界面版：按 [Linux 无界面版安装](/docs/install-linux/) 使用官网一行命令安装，再显式启动服务。

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

CLI 入口与本机服务状态相互独立。如果命令提示服务不可用，桌面应用用户先打开应用；独立服务版（standalone）用户运行 `hiroute service status`，并在需要时运行 `hiroute service start`。CLI 不会替你自动启动服务或重放失败请求。

## 两层命令合同

Host 管理命令以根帮助和 family 帮助为准：

```sh
hiroute --help
hiroute service --help
hiroute gateway --help
hiroute protected-input --help
```

Application / Local Control 业务命令以 Released schema 和完整 leaf 帮助为准：

```sh
hiroute schema list --output json
hiroute schema show --command-id routing.apply --output json
hiroute routing apply --help
```

不要期待 `schema list` 收录 Host 命令，也不要从网页示例推断当前安装的请求字段。

## 管理模型、路由与 Agent

当前已发布的 CLI 支持完整无界面闭环：

- `compute scan/list/show` 发现和读取模型来源；`compute connection options/test/preview/apply/authorize` 检查并保存连接。
- `models show` 查看候选模型能力。
- `routing options/list/show/preview/apply` 创建、更新并发布智能路由。
- `agents scan/list/check` 发现本机 Agent；`agents connect preview/apply/status` 接入，`agents restore preview/apply` 恢复。
- `operations find/get` 在写响应丢失或状态不确定时，按原幂等域查询已发生的操作。

写操作都先 `preview` 再 `apply`，并保留同一份 change、digest、revision 与 idempotency key。密码或 API key 通过 `protected-input` 传入，不放进普通 JSON、参数或日志。具体字段始终从对应 `schema show`、`options` 和 leaf `--help` 获取。

## 查询会话与运行表现

```sh
hiroute sessions list --include-unlinked --limit 50 --output json
hiroute sessions show <SESSION_ID> --output json
hiroute sessions receipt <RECEIPT_ID> --output json
hiroute sessions status --output json
hiroute value show --routing <PLAN_ID> --session <SESSION_ID> --output json
```

默认会话查询返回事实和 timeline，不返回对话正文。receipt 展示实际路由、模型和上游已报告的 token；没有可信价格证据时，价值金额保持未知，不会伪造为零。

## 发现执行器和计划

```sh
hiroute worker executors
hiroute worker plans
```

列表只包含当前本机实际可用的执行器，以及当前 Agent 获准使用的已发布计划。不要根据显示名称猜测计划 ID，应使用命令返回的 ID。

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

`--submission-key` 是调用者选择的幂等键。若网络或进程中断后不确定任务是否被接受，请保留同一个 key 查询，不要创建一个新 key 自动重试：

```sh
hiroute worker status --submission my-check-001 --operation start
```

## 查看进度与结果

使用启动结果中的 run ID：

```sh
hiroute worker status --run <RUN_ID>
hiroute worker wait --run <RUN_ID>
hiroute worker result --run <RUN_ID>
```

`wait` 是有界等待，不会取消仍在运行的任务。`result` 支持按 offset 和最大字节数分页读取较大的结果。

## 继续或取消

继续任务时必须同时提供 task ID 和它的精确最新 run ID：

```sh
hiroute worker continue \
  --task <TASK_ID> \
  --expected-latest-run <RUN_ID> \
  --submission-key my-check-002 \
  -- "根据测试结果完成修复"
```

取消一个精确 run：

```sh
hiroute worker cancel --run <RUN_ID> --reason user-requested
```

取消不会撤销已经写入的文件或已经发生的外部操作。

## 机器可读输出

公开命令支持 `--output text|json|quiet`。交互使用默认的 `text`；脚本和主 Agent 使用 `json` 并按 schema 处理；只关心成功或失败时使用 `quiet`。可以用 `hiroute schema list` 和 `hiroute schema show` 在运行时发现当前版本的机器合同。

CLI 当前公开 45 个 Released Application 命令；其余 Planned 命令会继续被 CLI 和 daemon 拒绝。自动化应在运行时读取 schema，而不是把命令总数或尚未发布的能力写死。
