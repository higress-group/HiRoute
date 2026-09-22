# 在 Linux 上以无界面模式运行 HiRoute

Linux 无界面版把 HiRoute 的本机服务、Gateway、CLI 和 Agent 管理 Skill 安装到当前用户目录，适合服务器、远程工作站和无图形界面的自动化环境。它不是精简的只读客户端：模型来源、智能路由、Agent 接入、会话观测和任务委派都可以从 CLI 完成。

当前公开产品验证以 Linux 为准，支持 `x86_64` 和 `aarch64`。不需要 `sudo`；需要 `curl`、Python 3，以及能够运行 HiRoute 二进制的 glibc Linux 环境。

## 1. 安装

首个稳定 Linux 包发布后，运行：

```sh
curl -fsSL https://hiroute.ai/install.sh | sh
```

希望先检查脚本时：

```sh
curl -fsSLo install.sh https://hiroute.ai/install.sh
less install.sh
sh install.sh
```

入口安装到 `~/.local/bin`。如果终端找不到 `hiroute`，把下面一行加入 shell 配置并重新打开终端：

```sh
export PATH="$HOME/.local/bin:$PATH"
```

安装脚本会下载最新稳定的 Linux 版本，并在安装前检查架构与文件完整性。桌面应用与独立服务版（standalone）不应在同一个 `HOME` 中同时管理服务。

## 2. 显式启动服务

安装不会自动启动服务或重放请求。安装完成后运行：

```sh
hiroute service start --output json
hiroute system status --output json
```

`service start` 使用当前用户的 systemd user manager，适合已有正常登录会话的主机。精简容器、某些 SSH 环境或未启用用户会话的系统可能没有这个条件；此时可在受管理的前台终端直接运行：

```sh
hiroute service run
```

保持这个进程运行，再从另一个终端执行 `hiroute system status --output json`。如果需要登出后长期运行，请按所在发行版的安全策略启用 systemd 用户会话或 linger；这属于主机管理设置，安装器不会替你修改。

`system status` 应显示 role-all daemon 和 Gateway 已就绪。若服务不可用，先查看：

```sh
hiroute service status --output json
hiroute service doctor --output json
hiroute service logs --output json
```

`service`、`gateway` 和 `protected-input` 是 Host 管理命令，通过根帮助和 family 帮助发现：

```sh
hiroute --help
hiroute service --help
```

## 3. 发现业务命令

模型、路由、Agent、会话和 Worker 属于 Application / Local Control 命令。以当前安装返回的 Released schema 为准，不要根据网页示例猜请求字段：

```sh
hiroute schema list --output json
hiroute schema show --command-id compute.connection.test --output json
hiroute compute connection test --help
```

可以先复制下面这组只读命令熟悉当前机器，不需要先准备配置 JSON：

```sh
hiroute compute scan --output json
hiroute compute list --output json
hiroute routing list --output json
hiroute agents scan --output json
hiroute agents list --output json
hiroute sessions status --output json
```

它们分别展示可发现的来源、已有路由、可接入 Agent 和观测状态。确认实际 ID 后，再通过相应 `options`、`schema show` 与 leaf `--help` 生成写请求。

写操作遵循同一个安全流程：先读取 schema 和 options，执行有界 test，再把同一份变更送入 `preview`；确认 preview 的摘要、revision 和影响后才执行 `apply`。密码或 API key 通过 `protected-input` 传入，不放进普通 JSON、参数或日志。

## 4. 完成无界面配置

按下面顺序建立第一条路由：

1. 用 `compute scan/list/show` 查看可用来源；自定义 API 使用 `compute connection options/test/preview/apply`，需要浏览器授权的来源再使用 `authorize`。
2. 用 `models show` 核对候选模型能力。
3. 用 `routing options` 取得当前选项和 revision，再用 `routing preview/apply` 创建并发布路由；用 `routing list/show` 检查结果。
4. 用 `agents scan/list/check` 发现本机 Agent，再用 `agents connect preview/apply/status` 接入。恢复时使用 `agents restore preview/apply`，HiRoute 只撤销仍由自己拥有的配置字段。
5. 从 Agent 发起真实请求后，用 `sessions list/show/receipt/status` 查看路由事实和用量，用 `value show` 查看已有价格证据下的价值记录。

这些不是另一套无界面控制面；CLI 与桌面应用调用相同的 Application、Local Control、存储、发布和恢复路径。

## 5. 任务委派

已经发布执行计划后，可以从终端或主 Agent 发现计划并提交任务：

```sh
hiroute worker executors
hiroute worker plans
hiroute worker exec \
  --plan <PLAN_ID> \
  --cwd /absolute/path/to/project \
  --submission-key first-task-001 \
  -- "分析这个任务，完成实现并运行相关检查"
```

网络或进程中断后，不要换一个 key 盲目重试。保留原 `submission-key`，使用 `worker status --submission ... --operation start` 查询是否已接收。

## 6. 卸载

先停止服务，再运行同一份官方安装器的卸载入口：

```sh
hiroute service stop --output json
curl -fsSL https://hiroute.ai/install/standalone.py | python3 - uninstall
```

卸载范围仅限安装 marker 声明的版本目录、稳定入口、服务定义和管理 Skill，运行数据会保留。删除前会拒绝已被替换的稳定入口、服务定义或 Skill；安装器不会逐个审计版本目录内部的自定义改动，因此不要把自己的文件放进这些目录。

下一步阅读 [HiRoute CLI](/docs/cli/) 了解输出、幂等恢复和命令边界，或阅读 [智能模型路由](/docs/model-routing/) 与 [智能任务路由](/docs/task-routing/) 了解产品机制。
