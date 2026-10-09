# HiRoute Linux headless CLI

[English](standalone-cli.md)

Standalone 是不安装 Desktop 时的单用户运行方式。正式 `hiroute` CLI、`hirouted`
role-all daemon、Local Control、Gateway、业务存储和观测共同组成控制与运行闭环。它沿用同 UID
本机信任边界，不需要额外的“CLI 管理 token”，也不提供第二套 Agent 管理 API。

Standalone 与 HiRoute Desktop 不应同时管理同一用户的安装，不提供系统级多用户安装、Windows 安装、
运行时下载或自动更新。macOS 可使用相同候选包机制，但本页验证与 Quickstart 以 Linux 为准。

## 安装

从 [hiroute.ai](https://hiroute.ai/download/) 安装适用于 `x86_64` 或 ARM64（`aarch64`）的
最新稳定 Linux 版本：

```sh
curl -fsSL https://hiroute.ai/install.sh | sh
hiroute service start --output json
hiroute system status --output json
```

安装器会校验安装包并安装到当前用户目录，不会自动启动服务。主机要求和服务管理见
[Linux 安装指南](https://hiroute.ai/docs/install-linux/)。

## 构建候选包

发行或集成方从精确 committed candidate 准备 `hiroute`、
`hirouted` 和固定版本 CPA，再生成可复现归档及伴随 manifest：

```sh
python3 scripts/collect-third-party-licenses.py \
  --cargo-target x86_64-unknown-linux-gnu \
  --cpa-source-repo /path/to/CLIProxyAPI \
  --output /absolute/output/notices

python3 scripts/package-standalone.py build \
  --version 0.2.0 --revision FULL_COMMIT_SHA \
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

集成后安装器也接受 `--manifest-url https://.../PACKAGE.tar.gz.json`。重定向和归档必须保持
HTTPS；安装器会在任何写入前校验平台、归档 SHA-256、闭合集合和逐文件摘要。不要根据本页
拼造未发布的下载地址。

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
hiroute schema show --command-id compute.connection.test --output json
hiroute compute connection test --help
```

不要用 `schema list` 是否返回来判断 Host 管理命令是否可用。Application/Local Control 只使用
`schema list` 返回的命令；CLI 和 standalone daemon 会同时拒绝 Planned 业务命令，处理函数或开发
构建中存在命令路径不代表它已公开。所有机器响应都是一个 `hiroute.machine-envelope/v2`：自动化
读取 `status`、`data`、稳定错误 `error.code`、`warnings[].code` 和
`next_actions[].command_id`，不要解析展示文案。

以下示例中的 JSON 都是不含凭据的普通文件。`jq` 仅用于演示提取字段，不是 HiRoute 依赖。

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

从 `data.candidates[].binding_id` 选择来源后构造完整编辑器。固定单模型是合法的最小计划：

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
        "reselect_on_user_message": false,
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

## 接入、检查和恢复 Codex

```sh
hiroute agents scan --output json > agents.json
hiroute agents check agent_codex_default \
  --scope native-authentication --output json
```

`configuration`、`native-authentication` 和 `collaboration` 是同 UID 本地有界检查，不使用第二个
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

## 会话、实际选择与用量事实

一次 Agent 或 Worker 请求完成后：

```sh
hiroute sessions list --include-unlinked --limit 50 --output json
hiroute sessions show SESSION_ID --output json
hiroute sessions receipt RECEIPT_ID --output json
hiroute sessions status --output json
hiroute value show --routing PLAN_ID --session SESSION_ID --output json
```

`sessions show` 默认只返回事实和 timeline，不返回对话正文。RoutingReceipt 的
`route_decision`、`attempt_started`、`usage_and_cache` 等有序事实给出实际计划、模型选择、Attempt
和上游报告的已知 token。`value show` 只返回已有账价值；没有可信价格证据时金额保持 `null`，
且不得把未知金额或尚未形成账值行的 token 伪造为零成本。正文、搜索、catalog 与 ancestry 仍需
各自精确的受保护 capability。

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

## Gateway 监听器

```sh
hiroute gateway show --output json
hiroute gateway set --address 127.0.0.1 --port auto --output json
hiroute gateway set --address 192.0.2.10 --port 8317 \
  --accept-remote-risk --output json
hiroute gateway recover --output json
```

`set` 保存 desired 后重启，并只在真实 ready 后推进 applied。自动端口首次选定后持久化；
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

计划继续使用 `routing preview/apply`，编辑器 schema 为 `hiroute.plan-editor/v2`：

| 模式 | 编辑字段 |
| --- | --- |
| `smart_saving` | `smart.classifier`、`smart.economy`、`smart.primary` 与 `smart.judgment` |
| `custom_branches` | `branch_routing.classifier`、`branch_routing.branches`、`branch_routing.default_branch_id`、`branch_routing.judgment` 与 `branch_routing.reselect_on_user_message` |

决策模型和自定义扩展都使用 `classifier: {kind: "decision_service", service: <完整已保存连接版本>}`。
只有智能省钱可改用 `{kind: "local_rules"}`。发布按连接 ID、revision 和完整内容核对
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
