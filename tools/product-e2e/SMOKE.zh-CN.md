# 当前候选生产冒烟

[English](SMOKE.md)

`hiroute-smoke` 是开发验证入口。默认 quick 的三个必需案例分别验证：当前生产 Gateway 的受控 Responses 请求、正式 CLI/control-role daemon 经共享 Client Core 的状态、client_access/幂等只读查询与受控发现、被篡改的当前目录资源的安全拒绝。它们不证明控制面发布成功、真实 Agent、真实账号、Desktop 或安装完成。

## 运行

遵循仓库远端 Rust 约定，先提交并推送独立候选，再运行：

```sh
python3 scripts/remote-rust.py run --ref refs/heads/<branch> --sha <full-commit> -- \
  cargo test --locked -p hiroute-product-e2e --test smoke_cli -- --nocapture
```

测试实际调用开发工具并检查默认三例、control 域和单个 Gateway 案例的新运行结果。远端 `completed` 只说明 Cargo 成功及非零 Rust 测试命中，还需检查日志中的逐例状态和 checkout 内 `target/smoke/<run-id>/report.json`。

默认 `smoke_cli` 只通过 `default_two_domain_smoke` 执行一次三例真实链路，同时核对每例步骤、scope、结果与报告。CLI domain/case 选择器由不启动产品的 `validate` 测试覆盖。不要再把默认三例与三个单例重复运行；需要定位失败时直接按 case 重跑。

同一 checkout 的 `target/smoke/run.lock` 仍保护真实生产子进程与恢复材料。当前构建证明流程保持不变，不能擅自传入未验证的旧二进制来缩短检查。

在该验证 checkout 已构建工具后，可直接选择：

```sh
target/debug/hiroute-smoke list
target/debug/hiroute-smoke validate --domain control
target/debug/hiroute-smoke run
target/debug/hiroute-smoke run --domain control
target/debug/hiroute-smoke run --case gateway.responses.controlled
```

`list/validate` 仅校验目录或集合，不执行产品。domain/case 是精确 ID；同时提供时取交集，未知、重复、零命中或遗漏请求案例均报错。按旧报告的 `expected_cases` 重跑时，将这些 ID 逐项作为 `--case` 传入；所有 ID 在当前登记集合重新解析，生成新的身份和私有资源，不加载旧成功结果或配置。

## 读结果与恢复

报告分别保存工具退出、产品步骤退出/信号、逐例 green/expected_red/red、未执行原因、源码/产物身份、构建与执行时间、清理状态和能力缺口。大体积调试产物的完整性复核仍在每次产品命令启动前后执行，但这段 runner 证据开销不消耗 30 秒产品场景或 10 秒产品步骤等待预算。当前目录资源的 schema 或摘要篡改被正确拒绝时，daemon 非零退出与该负向案例 green 可以同时成立。未交付的 Agent、真实账号、Desktop 和安装适配只能返回 not_executed，不能返回 expected_red。`Report::verify` 是结构一致性检查，不能将外来 JSON 认证为生产证据。

子进程只使用本次隔离目录。控制面发现脚本是合成输入，不会调用日常 Codex/Claude。Agent 正式适配尚未接通；已有组件合同要求先证明全部配置层隔离，变更后经正式恢复并核对，再清理。冲突或恢复失败保留恢复上下文，不用复制备份覆盖用户配置。

日志和恢复定位信息仅存放在私有区域，不自动上传。正常取消先停止新操作并回收本次进程；强制 SIGKILL、主机重启或远端硬期限可能留下 interrupted 资源。发现未结束的 private/run.journal 时应人工检查该次 PID/资源，不能把旧记录转成 green，也不能批量清理仍活动的任务。失败 runtime 使用私有短临时目录并保留，成功后清理；Cargo 始终使用本 checkout 默认 target。

## 领域贡献

领域在 `src/smoke/registry.rs` 登记有限案例，并提供自己的真实操作和业务断言。中央 runner 只负责选择、生命周期和公共报告。17 在 control 案例中消费 CLI → shared Client Core → Application 的只读 `client_access` 与 `FindOperationByIdempotency` 路径；01 拥有控制面发布/恢复业务语义，02 拥有实际 Desktop 窗口、原生确认与改名业务案例，Agent 和安装领域提供正式恢复及平台证据。不要用内部 Application seam、手写 socket 或 fixture 输出替代所声称的产品入口。

若 Gateway 内层在返回耗时分解前失败，报告保留已知 build_ms，其余墙钟耗时记为 unclassified_ms，timing_complete=false，execution_ms 不冒充纯产品耗时。
