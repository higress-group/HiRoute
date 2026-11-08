# macOS 安装与首次启动

HiRoute 桌面应用当前支持 macOS 15.0 或更高版本。短期提供的是自签名安装包，没有 Apple Developer ID 签名和公证；因此 macOS 会要求你明确确认首次启动。

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>

复制以下指引到 Agent 对话中，再说明你想完成的操作。内容包含文档链接、内置技能入口和 CLI 线索。

<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
请先阅读以下文档，结合我在对话中提出的目标，用 HiRoute CLI 完成“macOS 安装与首次启动”相关操作。目标不明确时，只询问必要信息。

本页文档：https://hiroute.ai/zh/docs/install-macos/
CLI 参考：https://hiroute.ai/zh/docs/cli/

先按文档完成本次安装或检查目标。安装后，引导我在“Agent → HiRoute 管理 → 启用”中开启产品内置管理技能，后续配置交给该技能：https://hiroute.ai/zh/docs/#management-skill

操作线索（按本次目标选用；下面均为 hiroute 子命令）：
- 检查 CLI 和服务是否可用：--help · system status
- 查看当前接入和下一步：quickstart
- 开启产品内置管理技能：agents connect preview --management enable → agents connect apply
- 卸载前恢复 Agent 接入：agents restore preview → agents restore apply

界面与 CLI 连接同一个 HiRoute 服务，共用配置和执行记录。先读取现状并复用已有配置；参数以本机命令 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。按内置技能完成所需预览、应用与核验，最后说明结果。
```

</details>
</aside>

## 下载正确的安装包

在 [下载页](/zh/download/) 按 Mac 的架构选择 DMG：

- Apple 芯片（M1、M2、M3、M4 等）选择 `arm64`。
- Intel Mac 选择 `x86_64`。

从 `hiroute.ai/releases/<版本>/` 下载，并用下载页给出的完整 SHA-256 核对文件。新版本的简短文件名为 `HiRoute-<版本>-macos-<架构>.dmg`：

```sh
shasum -a 256 ~/Downloads/HiRoute-*.dmg
```

摘要不一致时不要打开文件，请删除并重新下载。

## 安装

1. 打开 DMG。
2. 将 HiRoute 拖入“应用程序”。
3. 从“应用程序”中打开 HiRoute，而不是一直从 DMG 运行。

## 首次启动自签名应用

先正常双击 HiRoute。如果 macOS 阻止启动：

1. 打开“系统设置” → “隐私与安全性”。
2. 找到刚刚被阻止的 HiRoute，选择“仍要打开”。
3. 再次确认打开。

Apple 的 [安全打开 Mac App 说明](https://support.apple.com/zh-cn/102445) 会随系统版本更新。HiRoute 不要求关闭 Gatekeeper、降低系统安全设置或关闭 SIP。

如果系统提示文件已损坏，先重新核对下载域名和 SHA256，并重新下载；不要把完整性错误当作普通的首次启动拦截绕过。

## 安装终端入口（推荐）

启动桌面应用后，打开“设置” → “CLI” → “终端入口”，选择“安装”。该操作会把与当前应用配套的入口安装到：

```text
$HOME/.local/bin/hiroute
```

如果页面提示 PATH 尚未包含这个目录，在 `~/.zprofile` 中加入：

```sh
export PATH="$HOME/.local/bin:$PATH"
```

重新打开终端，然后运行：

```sh
hiroute --help
```

这个终端入口连接 HiRoute 的本机服务；桌面应用应已安装并让本机服务保持可用。更多命令见 [HiRoute CLI](/zh/docs/cli/)。

## 更新与恢复

打开“设置 → 版本升级”，选择“检查更新”。HiRoute 从官网获取稳定版本，先校验完整的 DMG 和 App，再检查升级前提。安装前会等待现有调用和任务结束，等待期间可以选择“取消升级”；安装期间 Agent 路由会暂时不可用。HiRoute 会保留完整的旧版 App，升级失败时尝试恢复旧版本。

如果升级需要切换存储格式，会保留配置、凭据和路由，并封存旧的 HiRoute 历史；当前历史和统计从切换后开始，原生 Agent 的历史不受影响。

尽量通过“版本升级”更新。必须手动替换 App 时，先完全退出 HiRoute，确认它的服务和任务已经停止，并保留完整的原 App 和数据目录。

更新后，先在首页确认服务状态，再打开已保存的路由，并从接入的 Agent 发起一个小请求，确认保存的配置和实际路由都可用。

启动失败时，启动页会显示具体原因和恢复说明，服务不可用时也能看到。按页面指出的批次阅读 `RECOVERY.md` 或 `恢复说明.md` 后再恢复；也可以在终端运行 `hiroute service recovery --output json`，查看已验证的恢复批次和来源目录。不要单独替换数据库文件，也不要通过删除数据或反复重试来解决。

## 卸载

1. 在“Agent”页面停用各 Agent 的模型路由、任务路由和 HiRoute 管理，等待恢复完成，让 Agent 回到接入前的设置。
2. 在“设置”中关闭“登录时启动”。
3. 完全退出 HiRoute，再将 App 移到废纸篓。

删除 App 默认保留数据。确认没有正在运行的任务、接入已经恢复、会话记录和日志都不再需要后，才删除当前用户的 HiRoute 数据目录 `~/Library/Application Support/ai.hiroute.desktop`。不要删除 `.codex`、`.claude`、你的项目或已有凭据。
