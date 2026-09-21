# HiRoute Desktop

[English](README.md)

Desktop 提供模型管理、路由配置、Agent 接入、任务结果及会话记录入口，通过共享 Client Core 调用真实本机服务。公开使用入口见[官网文档](https://hiroute.ai/docs/)。

## 开发入口

在本仓库 checkout 构建 UI，然后构建同版本的原生应用和 daemon：

```sh
cd apps/desktop
npm ci
npm run build
cd ../..
cargo build --locked -p hiroute-desktop --features desktop-runtime -p hiroute-daemon -p hiroute-cli --bin hiroute-desktop --bin hirouted --bin hiroute
target/debug/hiroute-desktop
```

macOS 平台构建和实际窗口交互需要在 macOS 本机验证；通用开发检查见仓库根目录
`CONTRIBUTING.md`。所有 Cargo 输出使用该 checkout 默认 `target/`。

这是开发二进制入口。公开安装包和平台说明见[下载页](https://hiroute.ai/download/)。
原生应用从同目录启动 `hirouted`，从内置正式签名资源初始化 ReleaseFacts。WebView
仅加载本地构建产物；不提供任意命令、路径、HTTP 或授权的 IPC。

关闭窗口会隐藏窗口，后台由 native 宿主持有；Dock 激活可重开。退出 native 应用会结束本次启动的 daemon，下次启动从真实后端恢复。外部 daemon 仅作为只读连接，不受 Desktop 的关闭/退出管理。正式安装与平台交付范围见官网各版本说明。

可信启动后，Desktop 锁文件记录子进程 PID、私有端点目录与 socket 身份。持有锁、原 PID 已退出、身份未替换且端点不再监听时，重开交由 daemon 自身恢复残留端点。没有可信记录、PID 仍存活或路径身份不明时保留只读状态，不删除或接管该端点。

## 恢复与授权边界

确认上下文由 native backend 持有，只绑定这次 Preview 的对象、输入、digest/revisions 和窗口生命期，60 秒后失效。业务确认统一由本地 WebView 展示；WebView 只能针对 backend 发出的当前临时确认 ID 回传一次接受或拒绝，不能携带上下文、跳过确认或取得能力。接受后才通过继承通道登记 Desktop 主体的一次性能力；匹配的成功 ACK 之后才能发送 Apply。登记通道出现不确定结果后停止使用该写入关系，重新启动 native 应用才能建立新关系。

Desktop 只在当前交互内存中保留提交 key、摘要和 Operation 引用，用于响应丢失后的查询与显式重试。重开直接读取服务端实际配置；事务恢复属于 daemon 的既有 Journal。客户端不读写 `pending-intent.json`，历史文件不会触发告警或锁定编辑。切换编辑不取消后台操作，新动作独立经过服务端版本与幂等检查。

当前交互还保留版本化的 Plan ID + 目标名称摘要。没有查到受理时，同一意图可在 revision 改变后重新 Preview/重新确认，仍用原 key。查到原 Operation 时优先观察它，digest 不同会明确提示“最新编辑尚未应用”，并保留编辑草稿。查询失败或路由结果尚未同步时不会持续禁用编辑器；用户继续修改后，迟到的旧结果不得覆盖新输入，下一次保存仍由服务端 revision 检查。

“恢复先前名称”使用当前 desired 的新 Preview/确认，只替换名称，不回写旧 revision。先前名称保存在本次 native 会话中；非敏感编辑草稿与语言/文字大小偏好保存在本机 WebView。重启后仍可查询持久 Operation，先前名称按钮的会话历史不持久化。

## 验证入口

- Core/CLI/API/Application/daemon/Desktop：`cargo test --locked -p hiroute-client-core -p hiroute-cli -p hiroute-application-api -p hiroute-application -p hiroute-daemon -p hiroute-desktop -- --test-threads=1`。Desktop 的生产 bootstrap 测试使用同一 checkout 的 `target/debug/hirouted`（先构建该二进制，或随 daemon 的集成测试一起构建）。
- 前端：`npm run build`。
- 确定性 CLI 生成器：连续两次执行 `target/debug/generate-cli-contract`，第二次生成内容必须不变。
- macOS 隔离 Plan 准备：`python3 apps/desktop/tests/prepare_plan.py /tmp/hr02-new-empty-root`。依赖本机真实 Claude CLI、正式注册模型事实和已构建的生产 CLI/daemon；通过生产 Preview/受保护 Apply 准备数据，不直接写业务库。
- 实际 GUI 验收：debug 构建用 `HIROUTE_DESKTOP_TEST_ROOT` 指向上述隔离根，在其 `agent-input/` 工作目录启动原生程序。核对 WebView 确认取消零写、确认 ID 重放/窗口关闭失败关闭、名称 A→B→A、两个独立成功 Operation、Plan/alias/其余 desired 与实际发布身份；该 debug 路径开关不进入 release 构建。

组件替身测试不能替代实际 Tauri → Client Core → 生产 daemon 的可逆切片。执行退出码、各场景结论、平台与准确 revision 分别记录。


真实 Tauri ACL 探针仅是开发 example：构建 `cargo build --locked -p hiroute-desktop --features desktop-runtime --example acl_probe`，再以 `HIROUTE_DESKTOP_TEST_ROOT` 指定独立测试根运行 `target/debug/examples/acl_probe`。它使用生产 handler/权限清单，创建未许可的本地窗口与 remote-origin 主窗口，检查十次拒绝并写入该根的 `acl-probe.json`；不加入产品 bridge。退出 0 为十次权限拒绝，1 为断言失败，2 为超时。
