# HiRoute Linux headless CLI

[English](standalone-cli.md)

Standalone 是不安装 Desktop 时的单用户运行方式。正式 `hiroute` CLI、`hirouted`
role-all daemon、Local Control、Gateway、业务存储和观测共同组成控制与运行闭环。本机控制入口使用
操作系统对 socket 的访问权限，不需要额外的“CLI 管理 token”，也不提供第二套 Agent 管理 API。

Standalone 与 HiRoute Desktop 不应同时管理同一用户的安装，不提供系统级多用户安装、Windows 安装、
运行时下载或自动更新。macOS 可使用相同候选包机制，但本页验证与 Quickstart 以 Linux 为准。

## 安装

安装的 daemon 同时提供 [Web 控制台](web-console.md)。Linux 默认关闭；可在安装时通过
`--console-mode`、`--console-listen`、`--console-port` 配置，或安装后运行
`hiroute console init`，再启动/重启服务。`hiroute console open` 只为实际已监听的
本地或远程端点生成一次性登录 URL，不会启动监听。远程 HTTP 会持续提示明文传输风险。

从 [hiroute.ai](https://hiroute.ai/download/) 安装适用于 `x86_64` 或 ARM64（`aarch64`）的
最新稳定 Linux 版本：

```sh
curl -fsSL https://hiroute.ai/install.sh | sh
hiroute service start --output json
hiroute system status --output json
```

安装器会校验安装包并安装到当前用户目录，不会自动启动服务。主机要求和服务管理见
[Linux 安装指南](https://hiroute.ai/docs/install-linux/)。

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

## 构建候选包

发行或集成方从精确 committed candidate 准备 `hiroute`、
`hirouted` 和固定版本 CPA，再生成可复现归档及伴随 manifest：

```sh
python3 scripts/collect-third-party-licenses.py \
  --cargo-target x86_64-unknown-linux-gnu \
  --cpa-source-repo /path/to/CLIProxyAPI \
  --output /absolute/output/notices

python3 scripts/package-standalone.py build \
  --version RELEASE_VERSION --revision FULL_COMMIT_SHA \
  --target x86_64-unknown-linux-gnu \
  --hiroute /absolute/path/hiroute \
  --hirouted /absolute/path/hirouted \
  --cpa-binary /absolute/path/cliproxyapi \
  --cpa-version PINNED_VERSION \
  --cpa-license /absolute/path/CLIProxyAPI-LICENSE \
  --notices /absolute/output/notices \
  --output /absolute/output

python3 scripts/install-standalone.py install \
  --manifest /absolute/output/PACKAGE.tar.gz.json \
  --archive /absolute/output/PACKAGE.tar.gz
```

`RELEASE_VERSION` 和 `FULL_COMMIT_SHA` 应分别取同一 committed candidate 的组件版本与
完整源码 revision。

集成后安装器也接受 `--manifest-url https://.../PACKAGE.tar.gz.json`。重定向和归档必须保持
HTTPS；安装器会在变更安装前校验平台、归档 SHA-256、闭合集合和逐文件摘要。不要根据本页
拼造未发布的下载地址。

HTTPS 归档下载会使用有大小边界的请求，并对提前结束的传输进行有限次重试。
合并分段数据时，安装器要求服务端对象身份保持不变；不支持分段的服务端仍可整包下载。
所有大小和摘要检查通过前，安装器不会更改现有安装。

收集器只从锁定的 Rust 依赖和 `vendor/cpa/source.json` 指定的 CPA commit 生成材料；
`--cpa-source-repo` 只是该固定 commit 的本地对象来源，不会采用 checkout 当前分支。
打包器要求收集器的闭合输出，并将 HiRoute、CPA 及其传递依赖的许可证和确定性清单随归档
安装到 `licenses/`；材料缺失时打包失败。

安装只写当前用户目录：

- 稳定入口：`$HOME/.local/bin/hiroute`、`hirouted`；无需 sudo。
- 版本二进制：`$HOME/.local/lib/hiroute/<version>`。
- 只读资源和 marker：`$HOME/.local/share/hiroute`。
- 管理 Skill：`$HOME/.agents/skills/hiroute-management` 和
  `$HOME/.claude/skills/hiroute-management`。
- Linux 状态：`${XDG_STATE_HOME:-$HOME/.local/state}/hiroute`。

安装器会用私有、当前用户拥有的目录安装 Skill；已有符号链接、非目录、非当前用户所有或
group/world 可写的 Skill 父目录会导致安装在写入前失败。同一安装命令用于首次安装、修复和升级。
更新后显式运行 `hiroute service restart --output json` 才会切换正在运行的 daemon。

若存在 `/Applications/HiRoute.app`、`~/Applications/HiRoute.app` 或活动 Desktop Local
Control，安装会失败。反向切换时也应先停止并卸载 standalone。

## 存储升级与恢复

已校验的新程序在替换前先做只读预检。确实需要历史格式切换时，安装器才在私有服务状态目录
保留完整原版程序、资源与安装记录；daemon 直接读取交接记录，systemd 启动也适用，
不依赖终端继承变量。同格式修复不复制历史程序包，也不要求这份副本的额外预留空间。
重装同版本、同一 daemon 镜像时也保留原交接记录，不要求旧包支持新增预检协议；
服务启动仍执行原有存储校验，不因此获得旧版兼容或降级权限。
准备失败只清理本次未发布的副本；交接记录可能已被读取后，即使后续安装回滚，也保留材料。请为恢复材料预留空间，不要删除 upgrade-packages、
upgrade-package.json、封存历史或 storage.upgrade-backups 来绕过空间检查。

服务无法启动时，也可运行 `hiroute service recovery --output json` 查看存储方确定的
原因、恢复动作与诊断位置。缺包不允许单独重装旧程序或原地降级；请保留数据、密钥、
完整程序和记录，必要材料缺失时按诊断与支持指引处理。可继续的原批次使用当前版本恢复。必须整组还原时，
按对应批次的《恢复说明.md》、RECOVERY.md 及可用的 PROGRAM-LOCATIONS.md，停止服务
和自动重启，先另存新版数据及配套程序，再还原全部数据库、密钥、Observation 和记录的
辅助文件，最后还原原版程序、资源、安装记录与服务注册。重新加载管理器后确认实际
Control/Gateway 就绪及一次真实请求；不能只看安装命令退出成功。

显式无界面运维仍支持在首次封存启动时用 HIROUTE_RECOVERY_PACKAGE 提供完整旧包的
绝对目录。变量必须传给真正的前台 daemon，或写入服务管理器支持的环境配置；在终端
export 后调用 systemctl 不代表后台进程收到了变量。正常安装器使用受保护交接记录。
缺少密钥、原材料不可校验或阶段不明时应保留材料并诊断/求助，反复重启不能重建缺失数据。
修复同一份未变化的程序不要求新增预检协议；程序字节发生变化时，需要候选支持预检。

格式切换保留配置、凭据和路由，封存旧 HiRoute 历史，当前历史和统计从切换后开始；
原生 Agent 历史不变，当前格式的新记录在正常重启后继续保留。


使用 systemd 手工补充旧恢复包时，先运行 `hiroute service stop --output json`
停止失败服务，再运行 `systemctl --user edit ai.hiroute.cli.service`，添加临时配置：

```ini
[Service]
Environment="HIROUTE_RECOVERY_PACKAGE=/absolute/path/to/verified-complete-old-package"
```

将路径替换为实际校验过的完整旧包目录。运行 `systemctl --user daemon-reload`，
再运行 `hiroute service start --output json`。仍被拒绝时，运行
`hiroute service recovery --output text` 查看原因、下一步与诊断目录。
升级成功后，只移除此临时环境项，重新加载并重启服务，确认实际就绪和一次请求。
保留其它用户配置，不要用 `systemctl revert` 一并删除它们。仅在终端导出变量，
不会把该变量交给 systemd 管理的进程。

## 启动并确认真实就绪

```sh
hiroute service start --output json
hiroute service status --output json
hiroute system status --output json
hiroute gateway show --output json
```

只有以下事实同时成立才算业务就绪：

- `service status` 的 `data.local_control_ready` 为 `true`；
- `system status` 的 `data.daemon` 为 `role_all`、`data.gateway` 为 `ready`；
- `gateway show` 的 `data.ready` 为 `true`，并给出实际 `connect_address`。

进程管理器返回成功不等于服务已就绪。systemd user unit 不可用时，可在受管理的前台终端运行：

```sh
hiroute service run
```

其他生命周期入口：

```sh
hiroute service doctor --output json
hiroute service logs --output json
hiroute service restart --output json
hiroute service stop --output json
hiroute service autostart enable --output json
hiroute service autostart disable --output json
```

登录自启与当前会话启动是两个动作。`SIGTERM`/`SIGINT` 会触发有序停机。

### 订阅网络代理

先在终端开启代理，再执行 `hiroute service start`、`restart` 或 `run`。
这些命令读取 `HTTP_PROXY`、`HTTPS_PROXY`、`NO_PROXY` 及其小写形式，供订阅目录发现
和推理使用。HTTPS 目标使用 `HTTPS_PROXY`，`NO_PROXY` 控制绕过目标。不读取
`ALL_PROXY`、PAC 或操作系统网络设置，也不执行 shell 启动文件。

CLI 将这六个变量保存到仅当前用户可读的
`~/.local/share/hiroute/service/proxy-environment.json`，使单独启动的 systemd/launchd
服务能使用终端配置，不修改其他服务的环境。只有 CPA 消费此配置，本地管理通信保持直连；
登录自启复用已保存配置。修改代理后执行 `hiroute service restart`；对已运行服务执行
`start` 不更新配置。清除代理时，先 unset 全部六个变量再重启。代理失败不会回退直连。
代理地址可能包含密码，不要分享此私有文件。这不改变原生 Codex 的登录和 token 刷新归属。

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

## 先发现当前公开合同

CLI 有两层公开合同：

- Host 管理命令以根帮助和 family 帮助为准；`service`、`gateway`、`protected-input` 有意不进入
  Application release manifest。
- Application/Local Control 命令以 `schema list/show` 返回的 Released descriptor 和完整 leaf
  `--help` 为准。

```sh
hiroute --help
hiroute service --help
hiroute gateway --help
hiroute protected-input --help
hiroute schema list --output json
hiroute schema show --command-id compute.connection.test --include-schema --output json
hiroute compute connection test --help
```

不要用 `schema list` 是否返回来判断 Host 管理命令是否可用。Application/Local Control 只使用
`schema list` 返回的命令；CLI 和 standalone daemon 会同时拒绝 Planned 业务命令，处理函数或开发
构建中存在命令路径不代表它已公开。所有机器响应都是一个 `hiroute.machine-envelope/v2`：自动化
读取 `status`、`data`、稳定错误 `error.code`、`warnings[].code` 和
`next_actions[].command_id`，不要解析展示文案。

以下示例中的 JSON 都是不含凭据的普通文件。`jq` 仅用于演示提取字段，不是 HiRoute 依赖。

## 测试目录中的 API 连接

`compute connection test` 需要 JSON 输入来源和嵌套的 `{kind, request}` 请求体。
OpenRouter 等当前目录提供的选项使用 `registered`；自定义 API 使用 `native`，其配置放在
`request.draft`。`--non-interactive` 不能代替请求输入。完整的已安装 schema 在
`data.schemas.examples` 中提供两种模板，`data.schemas.example_guidance` 说明前置条件。
完整 schema/模板发现需要 `--include-schema`；v0.3.1 只返回命令描述。

确认服务已就绪、本机已安装 Python 3，并准备好只包含 OpenRouter API Key 的私密文件
（权限 0600），然后替换下方的 `KEY_FILE`。脚本从运行中的服务取得准确目录并构造
registered 模板。当前内置 OpenRouter 选项没有远端目录探测路径；
`inference_model_id=null` 时不会向 OpenRouter 发请求，只根据内置目录准备候选，
不验证连通性或 API Key，也不保存来源。
退出时会尝试释放本次受保护输入，并保留独立的结果和错误文件供排查。
普通 JSON 中只有引用，Key 通过 FD 3 单独传递。

```sh
(
set -eu
umask 077
KEY_FILE='/absolute/private/openrouter-key'
work=$(mktemp -d "${TMPDIR:-/tmp}/hiroute-connection.XXXXXXXX")
candidate="candidate/native/openrouter-$(python3 -c 'import uuid; print(uuid.uuid4().hex)')"
hiroute schema show --command-id compute.connection.test --include-schema --output json > "$work/contract.json"
hiroute compute connection options --output json > "$work/options.json"
hiroute protected-input register --candidate "$candidate" --secret-fd 3 --output json \
  3< "$KEY_FILE" > "$work/input.json"
trap 'hiroute protected-input release --candidate "$candidate" --output json >&2 || echo "Retry releasing this candidate if cleanup failed: $candidate" >&2' EXIT
python3 - "$work" "$candidate" > "$work/registered-request.json" <<'PY'
import json, pathlib, sys
work, candidate = pathlib.Path(sys.argv[1]), sys.argv[2]
contract = json.loads((work / 'contract.json').read_text())
options = json.loads((work / 'options.json').read_text())
registration = json.loads((work / 'input.json').read_text())
if any(x.get('status') != 'succeeded' for x in (contract, options, registration)):
    raise SystemExit('Read contract.json, options.json and input.json for the failed prerequisite')
if registration.get('data', {}).get('registered') is not True:
    raise SystemExit('Protected input was not registered')
option_id = 'openrouter.free.global.v1'
option = next((x for x in options['data']['options']
               if x['connection_option_id'] == option_id), None)
if option is None or not option.get('registered_check_available', False):
    raise SystemExit('The current catalog does not offer this registered check')
body = next(x for x in contract['data']['schemas']['examples'] if x['kind'] == 'registered')
request = body['request']
request.update(connection_option_id=option_id, expected_catalog=options['data']['catalog'],
               candidate_ref=candidate, lineage_ref='lineage/native/' + candidate.rsplit('/', 1)[1],
               check_id='check/native/' + candidate.rsplit('/', 1)[1], edit_revision=1,
               input_candidate={'candidate_ref': candidate, 'candidate_revision': 1},
               models=[], inference_model_id=None)
print(json.dumps(body, indent=2))
PY
set +e
hiroute compute connection test --request-stdin --output json \
  < "$work/registered-request.json" > "$work/result.json" 2> "$work/stderr.txt"
rc=$?
set -e
cat "$work/result.json"
cat "$work/stderr.txt" >&2
if [ "$rc" -eq 0 ]; then
python3 - "$work/result.json" <<'PY'
import json, sys
check = json.load(open(sys.argv[1]))['data']
fields = ('reachability', 'authentication', 'directory', 'inference')
print('Remote check results: ' + ', '.join(name + '=' + check[name] for name in fields))
if check['reachability'] == 'not_run' and check['authentication'] == 'unknown':
    print('Remote connectivity and API key were not verified.')
PY
fi
printf 'connection-test exit code: %s; result files: %s\n' "$rc" "$work"
exit "$rc"
)
```

请查看结果中的 `data.reachability`、`data.authentication`、`data.directory` 和
`data.inference`。当前内置 OpenRouter 选项执行此流程会返回
`reachability=not_run`、`authentication=unknown`、`directory=not_run` 和
`inference=not_run`。退出码 0 / `status=succeeded` 仅表示命令完成，不能证明服务已验证；
目录中有模型或候选可选择也不能证明这一点。远端验证需要显式选择受支持的探测方式；
选择推理模型可能消耗配额。

受保护输入登记协议对新登记凭据使用 `candidate_revision=1`，它与模型检查完成后返回的候选
版本不同。`expected_catalog` 必须完整复制当前的 `data.catalog`；目录不匹配时，重新获取
options 并构造检查，模板中的摘要仅为占位。相同请求也可通过
`--request-file "$work/registered-request.json"` 传入，替代 stdin 重定向。
自定义 API 可将 `native` 模板保存为 `native-request.json`，填写真实 endpoint、协议事实
和受保护输入引用；下一节给出了手工声明模型的完整请求。

## 接入 Native API 模型来源

凭据只能通过受保护 FD 登记。不要把 Key 放进 argv、环境变量、普通 JSON、日志或 Agent 对话：

```sh
chmod 600 /absolute/private/provider-key
exec 3</absolute/private/provider-key
hiroute protected-input register \
  --candidate candidate/native/my-provider \
  --secret-fd 3 --output json
exec 3<&-
```

按 `compute.connection.test` schema 构造一次有界检查。下面是 Responses API、Bearer 认证、无
`/models` 目录时手工声明一个文本模型的完整形状；替换 endpoint、模型和能力事实，但不要添加
secret 字段：

```json
{
  "kind": "native",
  "request": {
    "draft": {
      "inference_model_id": null,
      "candidate_ref": "candidate/native/my-provider",
      "lineage_ref": "lineage/native/my-provider",
      "display_name": "My Responses API",
      "existing_source_id": null,
      "edit_revision": 1,
      "check_id": "check/native/my-provider-1",
      "base_url": "https://provider.example/v1",
      "base_kind": "api_root",
      "request_path_override": null,
      "inventory_path_override": "/v1/models",
      "protocol": "responses",
      "protocol_profile_id": "profile/custom/responses",
      "protocol_profile_revision": 1,
      "authentication": {"kind": "bearer"},
      "configuration_revision": 1,
      "models": [{
        "upstream_model_id": "provider-model-id",
        "display_name": "Provider model",
        "catalog_configuration_id": null,
        "membership": "user_declared",
        "capabilities": {
          "tool": {"value": true, "basis": "user_declared"},
          "vision": {"value": false, "basis": "user_declared"},
          "streaming": {"value": true, "basis": "user_declared"},
          "context_tokens": {"value": 32768, "basis": "user_declared"},
          "max_output_tokens": {"value": 4096, "basis": "user_declared"},
          "native_reasoning": {
            "value": {"kind": "fixed", "profile": "provider-default"},
            "basis": "user_declared"
          }
        }
      }]
    },
    "input_candidate": {
      "candidate_ref": "candidate/native/my-provider",
      "candidate_revision": 1
    }
  }
}
```

```sh
hiroute compute connection test --request-stdin --output json \
  < native-test.json > native-checked.json
hiroute compute list --output json > compute-before-save.json
```

从 `native-checked.json.data.candidate.models[]` 选择 `selectable=true` 的 `model_ref`，从
`compute-before-save.json.data.revisions` 复制当前版本，构造保存变更：

```json
{
  "change": {
    "schema": "hiroute.compute-management-change/v2",
    "subject": {"kind": "candidate", "candidate": {
      "candidate_ref": "CANDIDATE_REF_FROM_CHECK",
      "candidate_revision": 1
    }},
    "expected_revisions": {"target": 0, "dependencies": {}},
    "selected_model_refs": ["SELECTABLE_MODEL_REF"],
    "intent": "save_ready",
    "key_edits": []
  }
}
```

先 preview，再原样携带返回的 `spec`、`accept_digest`、`expected_revisions` 和新的幂等键 apply：

```sh
hiroute compute connection preview --request-stdin --output json \
  < compute-preview-request.json > compute-preview.json

jq '{spec:.data.spec,accept_digest:.data.accept_digest,
     expected_revisions:.data.expected_revisions,
     idempotency_key:"save-my-provider-1"}' \
  compute-preview.json \
  | hiroute compute connection apply --request-stdin --output json \
  > compute-apply.json

hiroute operations get "$(jq -r '.operation.operation_id' compute-apply.json)" --output json
hiroute compute list --output json
hiroute compute show "SOURCE_ID" --output json
hiroute protected-input release \
  --candidate candidate/native/my-provider --output json
```

不要在 apply 结果不确定时换幂等键。保留原请求，并用公开恢复入口按相同作用域查询：

```json
{
  "principal_kind": "interactive_user",
  "operation_kind": "ApplyComputeSave",
  "idempotency_key": "save-my-provider-1",
  "accepted_digest": "ACCEPT_DIGEST_FROM_PREVIEW"
}
```

```sh
hiroute operations find --request-stdin --output json < operation-find.json
```

同一 key 与同一内容会返回原 Operation；同一 key 与不同内容会稳定拒绝，不会重复写入。

## 发现并保存订阅来源

```sh
hiroute compute connection options --output json > connection-options.json
```

`data.subscriptions` 会如实给出发现状态和候选。选择一个 `connector_owned` 候选后，将候选对象放入
`{"candidate": ...}`，再使用同一 preview/apply 规则开始检查：

```sh
jq '{candidate:([.data.subscriptions.candidates[] |
    select(.provenance=="connector_owned") | .candidate] | first)}' \
  connection-options.json \
  | hiroute compute connection preview --request-stdin --output json \
  > subscription-preview.json

jq '{spec:.data.spec,accept_digest:.data.accept_digest,
     expected_revisions:.data.expected_revisions,
     idempotency_key:"check-subscription-1"}' \
  subscription-preview.json \
  | hiroute compute connection apply --request-stdin --output json \
  > subscription-operation.json

jq '{action:"result",operation:.operation}' subscription-operation.json \
  | hiroute compute connection authorize --request-stdin --output json \
  > subscription-checked.json
```

只有 `data.status=verified` 才表示该检查通过。将
`data.checked_candidate` 与 `data.validation` 经上节相同的 compute save preview/apply 保存。
发现、授权检查、保存和真实模型调用是不同事实；不要把发现成功描述成来源已保存或上游已调用。

## 创建、调整并发布路由

先查询真实候选，不手写内部 binding：

```sh
printf '{}\n' | hiroute routing options --request-stdin --output json > routing-options.json
```

日常使用优先采用上面的 Quickstart 参数和共享默认值。下面的高级 JSON 示例展示一种显式选择的固定策略；其中的限制不是产品默认值，也不是启用按难度分流路由前必须创建的临时计划：

```json
{
  "change": {
    "schema": "hiroute.plan-content-change/v2",
    "target": {"intent": "create", "creation_key": "my-first-route"},
    "editor": {
      "schema": "hiroute.plan-editor/v2",
      "display_name": "Daily coding",
      "purpose": "Coding requests from my local Agents",
      "mode": "fixed_model",
      "candidates": [{"binding_id": "BINDING_FROM_ROUTING_OPTIONS"}],
      "smart": {
        "economy": [], "primary": [],
        "judgment": {
          "degree": {
            "simple_threshold_millis": 800,
            "instructions": "判断当前任务所需的推理与不确定性。",
            "simple": "要求明确、范围有限，可以沿用已有模式的任务。",
            "complex": "需要调查未知原因或在明显不同的设计间选择的任务。"
          },
          "competence": {
            "floor_millis": 500,
            "instructions": "评价 assessment target 指向的已完成阶段是否取得有用进展。",
            "criteria": ["未取得有用进展，或需要大量纠正。", "取得有用但不完整或不稳定的进展。", "可靠推进，且无需重大纠正。"]
          }
        },
        "classifier": {"kind": "local_rules"}, "complex_keywords": []
      },
      "free": {"candidates": [], "primary": [], "primary_fallback": false},
      "delegation_enabled": false,
      "requirements": {},
      "limits": {
        "maximum_attempts": 1,
        "request_timeout_ms": 30000,
        "attempt_timeout_ms": 30000
      }
    },
    "consumed_draft": null
  }
}
```

```sh
hiroute routing preview --request-stdin --output json \
  < route-change.json > route-preview.json

jq --slurpfile change route-change.json \
  '{change:$change[0].change,accept_digest:.data.change_digest,
    expected_revisions:.data.expected_revisions,
    idempotency_key:"publish-my-first-route-1"}' \
  route-preview.json \
  | hiroute routing apply --request-stdin --output json > route-apply.json

hiroute routing list --output json
hiroute routing show PLAN_ID --output json
```

更新时把 `target` 改为
`{"intent":"update","plan_id":"PLAN_ID","expected_head_revision":CURRENT_HEAD}` 并提交完整
editor。过期 revision/digest 会返回 conflict，旧发布继续服务；重新读取、编辑、preview 后再 apply。
普通模型请求只执行路由，不会因为计划启用了 Worker 而隐式启动任务。

## 配置 Agent 协作

协作可独立启用、修改触发方式和关闭，不需要手工选择 context。从 `hiroute agents scan`
取得精确 Agent ID，然后运行：

```sh
hiroute agents collaboration configure agent_codex_default --trigger explicit --preview --output json
hiroute agents collaboration configure agent_codex_default --trigger explicit --idempotency-key codex-collab-1 --output json
hiroute agents collaboration configure agent_codex_default --trigger delegate-by-default --idempotency-key codex-collab-2 --output json
hiroute agents collaboration status agent_codex_default --output json
hiroute agents collaboration disable agent_codex_default --preview --output json
hiroute agents collaboration disable agent_codex_default --yes --idempotency-key codex-collab-off-1 --output json
```

`explicit` 只在明确要求时委派；`delegate-by-default` 由 Agent 判断是否委派。入口也接受扫描
得到的其他受支持 Agent ID。它与 Desktop 一样按协作意图选择位置：Codex 使用
`codex_access.root_context_id`，模型 profile 已配置时也使用原始 HOME 的 root context；
其他 Agent 使用自己的 context。模型和访问 token 保持原设置。

升级 HiRoute 后，以相同触发方式和新的配置提交标识重新保存协作配置，才能更新 HiRoute
管理的已安装 Skill；仅启动不会替换它。用户修改或自行维护的 Skill 仍遵守原有冲突规则。

`--preview` 不执行本地检查、写入提交记录或保存配置。实际保存仅在预览报告能力证明缺失时
执行既有的本地有界检查，然后重新预览，不产生上游模型调用。检查失败、文件冲突或仍有
blocker 时停止保存。关闭使用协作自己的 restore point，并要求 `--yes` 确认。

保存前会在上文 CLI 持久状态目录的 `cli-collaboration/` 写入私有、不含凭据的原始提交记录，
按 control endpoint 摘要隔离实例；清空运行目录或重启不会丢失记录。提交标识为 1–200 个
ASCII 字母、数字或 `. _ : -`，不能以 `-` 开头。
可省略提交标识，由 CLI 生成并在 `data.cli_submission.idempotency_key` 返回；自动化应明确
传入 `--idempotency-key`。相同意图用同一标识重试时沿用已冻结请求；不同意图需要新标识。
响应丢失时，读取返回的恢复 action 或用相同标识重试。`accepted` 表示 Operation 已接收，
须用 `hiroute operations get OPERATION_ID --output json` 和协作 status 确认完成。
记录损坏或版本不符会停止重试；不要通过删除记录掩盖未确定的提交状态。

下方原始 JSON 接口仍采用显式传入的 context。协作请求误用 Codex model profile 时，保留
`CAPABILITY_DENIED` 并返回 `COLLABORATION_ROOT_CONTEXT_REQUIRED` 提示，不会自动重定向。

## 接入、检查和恢复 Codex

```sh
hiroute agents scan --output json > agents.json
hiroute agents check agent_codex_default \
  --scope native-authentication --output json
```

`configuration`、`native-authentication` 和 `collaboration` 是本地有界检查，不使用第二个
授权 token，也不产生上游模型调用。`live` 会产生真实模型请求，仍要求命令 help 声明的明确同意
与受保护 probe grant。

从扫描结果复制 Codex `context_id`，用已发布 `PLAN_ID` 连接：

```json
{
  "spec": {
    "schema_version": {"major": 2, "minor": 0},
    "context_id": "CONTEXT_ID_FROM_SCAN",
    "model": {
      "intent": "configure",
      "settings": {
        "mode": "codex_default",
        "native_model_mode": "hiroute_only",
        "fixed_models": [],
        "allowed_plan_ids": ["PLAN_ID"],
        "default_selection": {"kind": "plan", "plan_id": "PLAN_ID"}
      }
    }
  }
}
```

```sh
hiroute agents connect preview --request-stdin --output json \
  < agent-connect.json > agent-preview.json

jq '{spec:.data.spec,accept_digest:.data.accept_digest,
     dependency_digest:.data.dependency_digest,
     expected_revisions:.data.expected_revisions,
     idempotency_key:"connect-codex-1"}' \
  agent-preview.json \
  | hiroute agents connect apply --request-stdin --output json > agent-apply.json

hiroute agents connect status CONTEXT_ID --output json > agent-status.json
```

Standalone daemon 已是 resident service，因此 preview 不要求 Desktop login item。HiRoute 只修改其
拥有的 Codex 字段；完成后用户继续运行原来的 `codex` 入口，模型请求会进入本机 Gateway。保存
`agent-status.json.data.restore_point_ref`。恢复时：

```json
{
  "spec": {
    "schema_version": {"major": 2, "minor": 0},
    "context_id": "CONTEXT_ID",
    "model": {"intent": "restore", "restore_point_ref": "RESTORE_POINT_REF"}
  }
}
```

对该 spec 依次调用 `agents restore preview`、复制 preview 返回字段并调用
`agents restore apply`，最后运行 `agents connect status`。恢复只撤销仍由 HiRoute 拥有的字段；
并发用户修改会导致冲突而不是被覆盖。Claude Code 使用其公开 profile/launcher 合同；不要把
Codex 的 Responses 计划直接配置给只支持 Messages 的 surface。

## JSON 请求输入

所有接受 `--request-stdin` 的命令也接受 `--request-file PATH`：

```sh
hiroute routing preview --request-file route-preview.json --output json
# 原有管道和 shell 重定向保持可用：
hiroute routing preview --request-stdin --output json < route-preview.json
```

输入来源只能选一个。两种方式共用有大小上限的 UTF-8 JSON 解析器和命令类型校验；
未知字段、重复 JSON 成员、多段 JSON、重复或混合来源、与人工选项混用均在连接
Local Control 前返回 `INVALID_ARGUMENTS`。文件必须是普通文件，相对路径以调用者
工作目录为准；以 `-` 开头的文件名加 `./` 前缀，带空格的路径需要引号。
保留现有大小限制（通常为 1 MiB，Operation 查找和 Worker 依赖选择为 64 KiB）。
请求文件不替代独立的 protected-input 或继承的 capability 通道。
Worker 原有 `--file` 继续读取提示词文本。

## 会话、实际选择与用量事实

一次 Agent 或 Worker 请求完成后：

```sh
hiroute sessions list --include-unlinked --limit 50 --output json
hiroute sessions show SESSION_ID --output json
hiroute sessions receipt RECEIPT_ID --output json
hiroute sessions status --output json
hiroute value show --routing PLAN_ID --session SESSION_ID --output json
```

位置参数 `sessions receipt RECEIPT_ID` 保留 v1 RoutingReceipt 查询。
查询 v2 Facts 时，使用 v2 session timeline 返回的准确逻辑 `request_id`；
Session ID、v1 Receipt ID 与此定位符不同：

```json
{
  "schema": "hiroute.observation.query/v2",
  "intent": {
    "view": "facts",
    "query": {"request_id": "REQUEST_ID_FROM_TIMELINE", "limit": 100, "cursor": null}
  }
}
```

保存为 `facts-request.json` 后运行
`hiroute sessions receipt --request-file facts-request.json --output json`。
`--request-stdin` 接受相同请求，返回 v2 Facts 分页；后续页沿用同一 request ID
并传入返回的 cursor。


`sessions show` 默认只返回事实和 timeline，不返回对话正文。RoutingReceipt 的
`route_decision`、`attempt_started`、`usage_and_cache` 等有序事实给出实际计划、模型选择、Attempt
和上游报告的已知 token。`value show` 只返回已有账价值；没有可信价格证据时金额保持 `null`，
且不得把未知金额或尚未形成账值行的 token 伪造为零成本。同一用户通过 Local Control 显式读取正文、
搜索、catalog 与 ancestry 无需另取读取票据；显式传入的授权仍绑定准确操作和范围，运行范围受限的调用方仍保留原有边界。

## 设置本地观测存储容量

默认仍保留七天明细。可以额外设置容量阈值，单位为字节，也可以关闭容量提前回收：

```sh
hiroute observation storage show --output json
hiroute observation storage set --max-active-bytes 20000000000 --output json
hiroute observation storage set --disable --output json
```

这些命令通过操作系统认证的本地用户通道访问；显式传入的委派或只读授权不能修改策略。

示例为十进制 20 GB。超出后，最早的、已完成且允许回收的请求明细可能在七天前到期。
正在处理的请求、仍被引用的内容、防重放和删除屏障、非正文金额汇总以及恢复备份受到保护。
关闭后停止尚未提交的提前回收，不会恢复已经到期的历史。

这是软阈值：后台按间隔、分批回收，繁忙或无进展时延后重试，每轮工作有时间预算。
首次盘点、繁忙、待完成清理或受保护的数据均可能延迟磁盘释放。
`show` 返回有效配置、实测活动占用、测量时间及等待或阻塞原因；
数据库内部可复用页不计为已释放磁盘。旧源目录和恢复程序包单独保留，升级时需预留空间。
恢复必须遵循[完整存储组恢复规则](release-contracts.md#upgrade-and-manual-recovery)。

## 配置 Worker 并委派任务

Worker 复用现有命令。发现不会安装或选择软件：

```sh
hiroute worker dependencies discover --harness codex_cli --output json \
  > worker-discovery.json
```

从同一 Harness 的 `selection_revisions[].revision` 复制并选择完整的 `found` 绝对路径：

```json
{
  "harness": "codex_cli",
  "adapter_path": "/absolute/path/to/codex-acp",
  "cli_path": "/absolute/path/to/codex",
  "node_path": "/absolute/path/to/node",
  "expected_selection_revision": 0
}
```

```sh
hiroute worker dependencies select --request-stdin --output json \
  < worker-selection.json
hiroute worker executors --output json
hiroute worker plans --output json
```

Codex Worker 要求所选 `codex-acp` adapter 在初始化时声明 AIR `sessionFailure`
能力。已核对的 1.1.5 缺少该能力，2.1.1 声明了该能力；HiRoute 检查能力，不使用版本
白名单。如果任务以 `CapabilityUnavailable` 失败，可通过
`hiroute worker read --run RUN_ID` 查看已保存的进展。缺少此能力时，`[HiRoute]`
提示会说明原因和处理办法：更新所选 adapter，在 Worker 依赖配置中重新选择路径，再重新
提交任务。新运行使用新的 submission key；重放旧 key 只会返回原来的失败运行。
发现和选择不是兼容性预检查，也不会安装或更新 adapter。

将路由 editor 的 `delegation_enabled` 设为 `true`，并加入
`"work":{"harness":"codex_cli","protocol":"responses"}` 后重新发布。再提交、定位、等待和读取：

```sh
hiroute worker exec --plan PLAN_ID --cwd /absolute/project \
  --no-wait --submission-key task-20260921-1 --file task.txt --output json

hiroute worker status \
  --submission task-20260921-1 --operation start --output json
hiroute worker status --run RUN_ID --output json
hiroute worker wait --run RUN_ID --wait-timeout 30 --output json
hiroute worker result --run RUN_ID --output json
hiroute worker list --output json
```

等待超时只表示仍在运行，不会取消任务。响应不确定时用原 submission key 查询，或原样重放同一
`worker exec`；不要换 key。重复提交返回同一个 run，不会执行两次。`selection_revision` 是 Worker
选择配置的并发版本，不是 Codex、Claude 或 HiRoute 软件版本。

CLI 不写本地提交收据，无需可写的收据目录，也不再使用 `HIROUTE_WORKER_RECEIPT_DIR`；
已有收据文件保留原样。每次新提交优先预先选定唯一编号，将其字面值传入 `--submission-key`，
并在对话或工具调用记录里保留编号和原请求。未指定编号时 CLI 仍会生成并返回，但在响应到达前
崩溃可能丢失编号。编号丢失时可用 `hiroute worker list --title TITLE --output json` 辅助找回：
遍历分页并核对时间、Plan、目录和任务详情。标题可能重名，随首次输入内容保留七天或在提前
删除内容后隐藏，完成任务也遵守同一规则。不能明确匹配时请求确认，不因列表为空自动新建任务。

## Gateway 监听器

```sh
hiroute gateway show --output json
hiroute gateway set --address 127.0.0.1 --port auto --output json
hiroute gateway set --address 192.0.2.10 --port 8317 \
  --accept-remote-risk --output json
hiroute gateway recover --output json
```

`set` 保存 desired 后重启，并只在真实 ready 后推进 applied。自动端口首次选定后持久化。
重复 `--port auto`、固定转自动或只修改地址时，优先保留已保存的 desired 端口；
desired 尚未选定端口时复用最近 applied 的端口，两者都没有才首次选择。
端口冲突会明确失败，不另选端口。服务管理器重启失败后 desired 仍保留，操作仍为 failed；
随后通过 `hiroute service run` 启动时，真实 ready 后记录该 desired 为 applied。
`recover` 使用当前 applied 地址，不会恢复任意历史端口。

`gateway show` 通过公开 Agent settings status 核对配置与当前运行地址。
`agent_update.required` 在配置偏移、变更待完成或无法完成检查时提示核查/重新配置，
不依赖最近监听器操作是否成功；`existing_agent_count` 为 null 表示发现不可用。
配置已一致和仅启用协作的目标无需更新模型。未配置/已恢复模型还会检查已保存的 V1 接入：
没有旧记录时无需更新；旧配置已断开时提示核查。当前已配置状态优先于已被取代的历史接入。
地址实际变化后，保留所需模型选择，重新执行 `agents connect preview` / `agents connect apply`；
监听器重启不会自动改写原生 Agent 配置。

`0.0.0.0` 对本机管理客户端显示为 `127.0.0.1:<port>`。非 loopback 配置要求显式风险确认。
HiRoute 不自动修改防火墙、TLS 或远端 Agent。暂不支持 IPv6、多监听器和防火墙管理。

## 常见错误与恢复

- `DAEMON_UNAVAILABLE`：Standalone 先运行 `hiroute service status`，需要时运行
  `hiroute service start`，再用 `service doctor`、`service logs` 诊断；Desktop 启动或恢复应用。
  隔离实例确认使用同一个 `HOME`、`XDG_STATE_HOME` 和 `XDG_RUNTIME_DIR`。CLI 不会自动启动服务
  或重放请求。
- `UNKNOWN_COMMAND`：Host 管理路径先读根帮助和 family `--help`；Application 命令不在当前
  release manifest 时，不要尝试内部 operation 名或 staged 路径，先升级或改用 `schema list`
  中的入口。
- `INVALID_ARGUMENTS`：读取 leaf `--help` 和 `schema show`；严格 schema 会拒绝未知字段。
- `REVISION_CONFLICT` / `CHANGE_PREVIEW_STALE`：重新读取当前资源和 options，重新 preview；不要
  修改旧 preview 后强行 apply。
- `IDEMPOTENCY_KEY_REUSED`：同一 key 已用于另一摘要；用 `operations find` 查看胜出 Operation，
  不要把新内容说成已应用。
- apply 响应丢失：保留原 spec/digest/revisions/key，先 `operations find`；同一内容的显式重试
  继续用原 key。
- Agent restore 冲突：用户或 Agent 已修改受管字段；停止自动覆盖，读取 status 并重新确认。
- 模型请求成功但金额未知：查看 RoutingReceipt 的实际 model/usage；没有价格证据时 `null` 是
  正确结果，不等于免费。

## 管理 Skill

安装包把同一份 `hiroute-management` Skill 安装到通用 Agent 和 Claude Code Skill 目录。它只
组织本页公开命令：检查服务、配置来源与路由、接入/恢复 Agent、查询观测和操作 Worker。它不
实现业务校验、不直接编辑数据库或 Agent 配置、不接触明文凭据，也不会自行安装、发布、变更
监听器、取消任务或启用自启动。Agent 对 Host 管理命令先读 `hiroute --help` 和对应 family
`--help`，对 Application/Local Control 命令先读 `schema list/show` 和完整 leaf `--help`，并遵循
用户明确授权的外部副作用范围。

## 卸载

先停止服务，再执行：

```sh
python3 scripts/install-standalone.py uninstall
```

卸载只删除 marker 记录且仍归 HiRoute 所有的稳定入口、当前版本程序、服务定义和两处 Skill；
任何条目被外部替换都会中止而不是覆盖。业务存储、诊断和会话默认保留。删除保留数据必须另行
确认确切目录及其内容已不再需要。

## 决策服务与分支路由

Desktop 在“模型 → 决策模型”管理这些连接；已发布 CLI 沿用 `decision services list/apply/test`。
决策模型和自定义扩展独立于实际执行任务的通用模型来源。先发现当前安装的合同：

```sh
hiroute decision services list --output json
hiroute schema show --command-id decision.services.apply --output json
hiroute decision services apply --help
hiroute decision services test --help
```

没有单独的 `decision services preview` 命令。第一次 `apply` 只预览，不保存。
把下面完整请求保存为 `decision-preview-request.json`。外层请求包含 `schema_version` 和
`spec`；内层 ChangeSpec 也有自己的 `schema_version`：

```json
{
  "schema_version": {"major": 1, "minor": 0},
  "spec": {
    "schema_version": {"major": 1, "minor": 0},
    "command_id": "decision.services.apply",
    "resource_id": "decision-main",
    "desired_state": {
      "id": "decision-main",
      "expected_revision": 0,
      "service": {
        "id": "decision-main",
        "revision": 1,
        "name": "百炼决策模型",
        "connection": {
          "kind": "system_one",
          "provider": "bailian-token-plan",
          "model": "decision-model-preview",
          "endpoint": "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/systemone",
          "timeout_ms": 10000,
          "auth_header": {"name": "Authorization", "value_secret_ref": "decision/main/r1"}
        }
      },
      "input_slot": "candidate/decision-main"
    }
  }
}
```

预览前，通过受保护输入注册 `candidate/decision-main`。本例私有文件应包含完整
`Bearer ...` 请求头值，而非裸 API Key：

```sh
chmod 600 /absolute/private/decision-header
exec 3</absolute/private/decision-header
hiroute protected-input register \
  --candidate candidate/decision-main --secret-fd 3 --output json
exec 3<&-

hiroute decision services apply --request-stdin --output json \
  < decision-preview-request.json > decision-preview.json
```

确认预览没有 blocker 后，用返回的完整规范、摘要和精确 revision 构造保存请求，再提交：

```sh
jq '{schema_version:{major:1,minor:0},spec:.data.normalized_spec,
     accept_digest:.data.change_digest,expected_revisions:.data.expected_revisions,
     idempotency_key:"save-decision-main-1"}' \
  decision-preview.json > decision-apply-request.json

hiroute decision services apply --request-stdin --output json \
  < decision-apply-request.json > decision-apply.json

hiroute decision services list --output json > decision-saved.json
hiroute protected-input release --candidate candidate/decision-main --output json
```

检查 `decision-apply.json` 中的 Operation 状态。提交结果不确定时，按原幂等域使用
`operations find/get` 查询，并保留完整原请求，不要换 key。编辑期望当前版本并保存下一版本，
如 `expected_revision: 1`、服务 `revision: 2`；删除使用 `service: null` 和当前期望版本。
路由、草稿或保留历史版本仍引用的连接不能删除。替换认证使用新的受保护引用，
已发布旧版本继续持有自己的凭证。Desktop 输入 API Key 时自动处理 Bearer 包装，
无需用户额外操作 Secret 管理页。

从保存结果中取准确的 r1 测试，不要测试未保存草稿或无条件使用最新版本：

```sh
jq -e --arg id decision-main --argjson revision 1 \
  '[.data.services[] | select(.id==$id and .revision==$revision)] |
   if length == 1 then
     {schema:"hiroute.classifier-decision-test/v1",
      classifier:{kind:"decision_service",service:.[0]}}
   else error("Expected saved connection revision is not in this list") end' \
  decision-saved.json > decision-test-request.json

hiroute decision services test --request-stdin --output json < decision-test-request.json
```

测试发送固定合成输入，可能消耗供应商额度。检查 `data.outcome` 和 `data.failure_code`，
不能仅凭 CLI 退出码判断通过。测试验证已保存连接的传输与必要响应字段，
不读取真实会话、不运行业务模型、不产生胜任样本；通过不保证任务质量或持续健康。
保存连接与测试是独立动作，两者都不会发布路由。

历史问题适用性是内置 System One 路由新发布时的默认行为。背景固定最多 8 轮；普通 Preview/Apply 会冻结实际提示词、标准、阈值及内容摘要。已有冻结路由继续原样执行，到下次发布才切换。本地规则和自定义扩展保持原行为。

评分区分默认和自定义。`judgment.competence.standard:"default"` 在下次编辑、发布时解析为当前默认；`"custom"` 逐字保留 instructions 和全部 criteria。旧输入缺少意图时，仅文字及三条标准精确匹配旧内置默认才归为默认，阈值变化不影响判断。“恢复默认”恢复当前默认并保留阈值。引用不可用上下文字段的自定义文字会在 Preview 指出具体范围和字段；旧发布继续有效。

连接页保留原有单一“测试连接”及 `hiroute.classifier-decision-test/v1` 契约，固定示例采用当前默认请求格式。独立的 `hiroute.historical-applicability-test/v1` 仅供 CLI 和验收诊断，输入包含已保存的 service_id、service_revision、Preview 返回的完整冻结 protocol，以及 standard 或 max_branches 形状。最大 16 个双组类别使用一次 19 问请求。应检查结果状态及精确机器标识；较小形状通过不证明最大容量。显式测试可能消耗配额；打开、保存和发布不会探测。路由编辑器没有独立测试或额外确认步骤。

观测查询保留 `hiroute.observation.query/v2`。facts 和 plan_quality 请求可设置 `include_routing_observations:true`；省略或 false 保持旧严格字段、授权编码和游标绑定。协议标识、摘要及形状计数仍保留在机器输出中。

计划继续使用 `routing preview/apply`，编辑器 schema 为 `hiroute.plan-editor/v2`：

| 模式 | 编辑字段 |
| --- | --- |
| `smart_saving` | `smart.classifier`、`smart.economy`、`smart.primary` 与 `smart.judgment` |
| `custom_branches` | `branch_routing.classifier`、`branch_routing.branches`、`branch_routing.default_branch_id` 与 `branch_routing.judgment` |

按难度分流和自定义分支会在每条新用户消息到来时重新判断模型组，然后只在选中的组内
优先沿用仍有效的上一轮模型。这个行为自动生效。可选字段 `reselect_on_user_message`
已废弃；为兼容受支持的旧版调用方，仍接受该字段，但它不影响路由，新请求可以省略。
读取结果中可能仍包含旧值，以保持密封 Plan 和发布数据的一致性。

决策模型和自定义扩展都使用 `classifier: {kind: "decision_service", service: <完整已保存连接版本>}`。
只有按难度分流可改用 `{kind: "local_rules"}`。发布按连接 ID、revision 和完整内容核对
已保存的不可变版本，不接受 `rest` 直配。

判断设置包含 `degree: {simple_threshold_millis, instructions, simple, complex}` 和
`competence: {floor_millis, instructions, criteria}`。阈值是 0–1000 的整数，
800 表示 0.8、500 表示 0.5；`criteria` 恰好包含 0、0.5、1 的三个非空评分标准。
前面固定路由示例中的简短提示词是可编辑示例，不代表完整内置默认提示词。

自定义分支包含 `id`、`name`、`condition`、常规 `candidates` 和 `primary_candidates`，
单组分支将后者设为空数组。分支可选 `judgment` 必须是完整覆盖；省略或设为 `null`
表示跟随计划默认。单独调整时复制整套有效设置再修改；恢复默认时清除整套覆盖。
任务 `condition` 与程度、胜任提示词分开配置。没有主力时不判断程度，仍可观察胜任度。

发布冻结连接版本、任务条件、判断标准、候选顺序及思考配置。保存 r2 不会改变已固定 r1
的路由，需要选中后重新发布；即使列表已返回 r2，已保存的历史 r1 仍可用于发布。
每个新用户轮次重新判断。工具续接和重放只有在同轮可识别、历史连续且决策可复用时才继承
冻结选择；压缩或历史重建导致消息不连续、决策无法复用时会重新判断。只有本次完整、兼容、同类别的
有效低分才能触发主力保护；缺失或部分评分不是零分。常规组故障接力只进入同分支主力，
直接选中主力则只在主力组内接力。

自定义连接使用 `connection.kind: "custom"`，遵守 HiRoute 的
[通用 Decision API](../decision-extensions/api/README.zh-CN.md)；内置连接使用供应商
[System One 映射](../decision-extensions/api/system-one-design.md)。选择与失败规则见
[决策协议](../decision-extensions/api/decision-design.md)。本期没有工具精选的运行入口。

### 为订阅保存代理

`hiroute subscription-proxy show --output json` 查看订阅代理设置与应用状态。
`hiroute subscription-proxy set --mode manual --url http://127.0.0.1:1187 --output json`
保存 HTTP/HTTPS 代理；可加 `--no-proxy example.com,192.0.2.0/24`。本机地址自动绕过。
使用 `--mode direct` 直接连接，或 `--mode inherit` 恢复既有服务启动环境策略。
保存后运行 `hiroute service restart --output json` 应用；重启可能中断正在执行的任务和请求。
手动代理作用于全部受管订阅及 Claude 订阅检查，不作用于 Worker 工具或 Native API。
不支持 URL 用户名密码、PAC 或 ALL_PROXY。配置已应用不等于订阅可用；仍需检查订阅和真实推理。


## 接入改名、追加与删除

继续使用 `compute connection preview` 和 `compute connection apply`，在
`change` 中增加可选 `edit`；不填写时保持原有保存语义。先读取 `compute list`
的最新 revisions，再预览，并将返回的 spec、digest、revisions 原样提交。

| `edit` | subject | selected_model_refs |
| --- | --- | --- |
| `{"action":"rename","display_name":"团队 API"}` | saved_source | 空数组 |
| `{"action":"append_models"}` | 对已有接入重新检查得到的 candidate | 要追加的模型 |
| `{"action":"remove_models"}` | saved_source | 要移除的模型 |
| `{"action":"delete"}` | saved_source | 空数组 |

saved_source 的完整形式为 `{"kind":"saved_source","source_id":"SOURCE_ID"}`。
上述已有接入操作和追加操作的 `key_edits` 为空。名称需与其他接入区分，长度为
1–60 个字符。新建 candidate 保存也可带 rename edit 指定名称，并按原有流程
提交受保护凭据。同一供应商模板创建的不同接入分别拥有端点、凭据和模型。

追加保留已有模型、绑定、凭据和启停状态。移除最后一个模型需明确使用 delete；
删除接入会清理其在 HiRoute 中拥有的凭据，保留订阅登录和历史记录。
预览的 `affected_plan_refs` 返回阻止删除的引用，包括停用路由、保存的草稿以及
仍保留的执行或恢复版本。应用阶段会重新检查，预览后新增引用也会阻止删除。

删除路由或调整其模型后，上一份发布可能仍保留引用。可通过正常路由发布流程更新：

```json
{"change":{"schema":"hiroute.publication-checkpoint-change/v1"}}
```

将此请求传给 `routing preview --request-stdin --output json`，再将相同 `change`、
返回的 `change_digest`（作为 `accept_digest`）、`expected_revisions` 和新的
`idempotency_key` 传给 `routing apply --request-stdin --output json`。等待 Operation
达到 `succeeded` 后，重新预览接入删除；响应不确定时应观察或重放原 Operation/key。
此操作仅将已安装的路由配置原样发布到下一版本，支持空配置，不创建路由或新增授权。
停用路由、草稿和仍保留的执行版本继续阻止删除。Desktop 在删除受阻窗口提供
“更新引用状态”，完成后再次打开删除窗口检查。

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

该显式视图用**当前来源下的 API 等价单价**计算已记录用量，计入重试，列出基准、价格来源/时间、路由估算成本、基准估算成本和带正负号的估算差额。它不会改写历史回执，也不证明回答质量相同、订阅账单或现金节省。路由侧的全范围金额可独立计算时，即使基准侧缺少数据也会显示；基准金额及差额仍需全部请求可成对比较，否则保持 null。可比较子集的小计会同时报告完整请求数，覆盖率描述的是成对比较。负差额不会截成零。过期、缺少用量或单价均降低覆盖率；查询范围过大时可缩短区间；该视图暂不支持按日分组。普通 `value show` 保留历史估值含义。

缺单价时，读取已安装的 `prices.effective` / `prices.override.preview` schema，以实际 Binding、币种和 `valuation_kind: "api_equivalent"` 查询，使用返回的 `edit_context` 修订号生成价格 Preview。手动 rates 单位是每百万 token 的整数货币微单位，输入/输出/缓存分别设置；使用有依据的价格或让用户提供这些非敏感数值，不猜测单价。保留原设置，提前给出填好参数的价格恢复 Preview/Apply 命令；原先没有手动覆盖时可用 `follow_catalog` 恢复目录价格。使用 Preview 返回的完整 spec/digest/revisions 和同一个提交 key 应用。订阅也能设置 API 等价参考价，不改变权益或普通计费分类；免费、订阅、付费和未知保持区别。

若参考计价中缓存创建按普通输入单价收费，可将参考缓存写入价设为输入价。此时金额与未上报的普通输入/缓存创建拆分无关，HiRoute 可合并同价计算桶，但不会补造或改写用量；两者单价不同时仍需要该细分计数。

新请求的历史估值也可在该请求冻结的输入价与缓存写入价相同、包含及互斥关系已确认时合并计算。金额可以完整，而缓存写入 token 仍为未知。对尚留存的旧记录，会话时间线可用匹配的冻结证据推导金额，不修改原有结算；旧结算汇总和已过期记录不会自动重算。

当路由侧全范围金额可计算、输入和缓存命中已知且缓存桶的包含与互斥关系已确认时，缺少缓存写入量的参考侧可以给出区间：假设参考模型使用相同输入与缓存命中量，未上报的写入量介于零和输入减缓存命中之间。区间按所选参考价计算，是明确假设下的估算，不证明参考模型实际会产生的缓存、输出或账单；它不会将成对比较覆盖标记为完整。缺少其他必要用量、单价或范围证据时不提供区间。
