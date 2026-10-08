# 在 Linux 上以无界面模式运行 HiRoute

Linux 无界面版将 HiRoute 安装到当前用户目录，适合服务器、远程工作站和无图形界面的自动化环境。安装内容包括本机服务、负责转发模型请求的 Gateway、CLI，以及供 Agent 使用的管理 Skill。你可以在终端连接模型、配置路由、接入 Agent、查看会话和委派任务。

Linux 安装包支持 `x86_64` 和 ARM64（`aarch64`）。不需要 `sudo`；需要 `curl`、Python 3，以及能够运行 HiRoute 二进制的 glibc Linux 环境。

## 1. 安装

运行以下命令安装最新稳定版：

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

## 2. 启动服务

安装不会自动启动服务或重放请求。安装完成后运行：

```sh
hiroute service start --output json
hiroute system status --output json
```

`service start` 通过 systemd 的用户服务管理器启动后台服务，需要主机已建立相应的用户会话。精简容器或部分 SSH 环境可能没有用户服务管理器；此时可在终端以前台方式运行：

```sh
hiroute service run
```

保持这个进程运行，再从另一个终端执行 `hiroute system status --output json`。如果需要登出后长期运行，请按所在发行版的安全策略启用 systemd 用户会话或 linger；这属于主机管理设置，安装器不会替你修改。

`system status` 应显示本机服务和 Gateway 已就绪。若服务不可用，先查看：

```sh
hiroute service status --output json
hiroute service doctor --output json
hiroute service logs --output json
```

服务启停等本机管理命令通过 `--help` 查看。例如，下面两条命令分别列出顶层命令和 `service` 的子命令：

```sh
hiroute --help
hiroute service --help
```

## 3. 发现业务命令

模型、路由、Agent、会话和委派任务的命令还提供 schema，用来说明当前版本接受的请求字段和返回结果。写自动化脚本前，先查看对应命令的 schema 和帮助：

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

它们分别展示可发现的来源、已有路由、可接入 Agent 和观测状态。需要修改配置时，使用查询结果中的实际 ID，再查看对应 `options`、`schema show` 和具体子命令的 `--help` 来准备请求。

保存配置通常分两步：先用 `preview` 查看将发生的变更，再用 `apply` 提交同一份变更，并带上预览返回的摘要和版本号。连接提供 `test` 时，可先检查连接。决策连接的预览与保存都使用 `decision services apply`，具体格式见 [CLI 指南](/docs/cli/)。密码或 API Key 通过 `protected-input` 传入，不放进普通 JSON、命令参数或日志。

## 4. 完成无界面配置

按下面顺序建立第一条路由：

1. 用 `compute scan/list/show` 查看可用来源；自定义 API 使用 `compute connection options/test/preview/apply`，需要浏览器授权的来源再使用 `authorize`。
2. 用 `models show` 核对候选模型能力。
3. 用 `routing options` 取得当前选项和版本号，再用 `routing preview/apply` 创建并发布路由；用 `routing list/show` 检查结果。
4. 用 `agents scan/list/check` 发现本机 Agent，再用 `agents connect preview/apply/status` 接入。恢复时使用 `agents restore preview/apply`，HiRoute 只撤销仍由自己拥有的配置字段。
5. 从 Agent 发起真实请求后，用 `sessions list/show/receipt/status` 查看实际路由、模型和用量，用 `value show` 查看已有价格证据下的价值记录。

CLI 与桌面应用使用同一份配置和执行记录。两者都需要发布路由后才生效，也遵循相同的配置恢复规则。

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

卸载会按安装记录删除对应的程序版本目录、命令入口、服务配置和管理 Skill，保留运行数据。如果命令入口、服务配置或 Skill 已被其他程序替换，安装器会停止删除，避免误删。版本目录会整体移除，请将自己的文件保存在其他位置。

下一步阅读 [HiRoute CLI](/docs/cli/) 了解输出、幂等恢复和命令边界，或阅读 [智能模型路由](/docs/model-routing/) 与 [智能任务路由](/docs/task-routing/) 了解产品机制。
