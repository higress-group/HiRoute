# Windows 桌面版安装

HiRoute 桌面版提供按当前用户安装的 NSIS 安装包，只支持原生 Windows x64。安装包自带命令行、后台服务、控制台文件和 WebView2 引导安装器，不要求 Python 或管理员权限。经过验证的版本发布后，可从[下载页](/download/)选择 Windows 桌面安装包。

## 校验下载

文件名为 `HiRoute-<版本>-<源码修订>-windows-x86_64-setup.exe`。从 `hiroute.ai/releases/<版本>/` 下载，在 PowerShell 中计算摘要，与下载页该文件旁的完整 SHA-256 比较：

```powershell
Get-FileHash -Algorithm SHA256 -LiteralPath "$env:USERPROFILE\Downloads\YOUR-INSTALLER.exe"
```

[Get-FileHash](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/get-filehash) 用于计算文件摘要。不一致时请删除文件并重新下载。

## 打开未签名安装包

本期版本没有代码签名，Windows 可能显示“未知发布者”或 SmartScreen 信誉提示。确认下载来源和摘要后，在系统提供该选项时选择“更多信息 → 仍要运行”。组织策略或 Smart App Control 可能禁止继续运行，此时请走组织的应用批准流程，不要关闭 Windows 安全功能。微软的[应用和浏览器控制说明](https://support.microsoft.com/en-us/windows/security/windows-security/app-browser-control-in-the-windows-security-app)介绍了这些保护。

## 安装与启动

1. 在普通用户会话打开安装包。安装目录固定为 `%LOCALAPPDATA%\Programs\HiRoute`；后台服务的数据单独存放在 `%LOCALAPPDATA%\HiRoute\cli`。
2. 登录自启和托盘默认开启，安装时可以分别取消。关闭托盘后，关闭窗口会退出壳，后台服务继续运行。
3. 打开 HiRoute。壳检查当前用户的 Task，启动服务，等待 Gateway 就绪后打开本机控制台。不需要口令或解锁，受保护操作仍在页面内逐次确认。

同一用户已有的 Task 安装会原地升级。本期不提供 SCM 安装；已有 SCM 托管的安装会在产品写入前被拒绝，并保持原样。此桌面安装包不支持 ARM64 或 x86 主机。

安装器把固定的 `.local\bin` 加入当前用户 PATH，保留其它条目；新开终端后生效。设置页与托盘的“Start at login”同时管理壳和后台 Task。托盘“Quit”只退出壳，服务保留运行。

无人值守安装可在校验安装包后，使用 `/S` 运行并等待进程退出码。安装器采用默认的登录自启和托盘选项，全程不弹提示，完成后启动服务，失败时返回非零退出码。仅使用 CLI 和 daemon 的用户也安装同一包。本期不发布 Windows standalone 归档或 ARM64 包。

## 更新与恢复

打开“设置 → 版本升级”。壳从官方 HTTPS 发布目录下载较新的稳定版，并在服务继续运行时检查大小和 SHA-256。安装前等待现有调用与 Worker 任务结束；“取消升级”恢复接收新工作，不取消或重放现有任务。随后安装器替换应用并重新打开 HiRoute。

手动下载的新安装包使用同一套排空与事务流程。旧 daemon 不支持排空时，只有交互式安装经明确确认后才能停止它，静默更新会拒绝。失败后，启动页或升级页显示错误和恢复状态，请保留日志与备份。安装被中断后再次打开壳，会尝试 journal 恢复并恢复被暂停的接收入口；遇到变更或外来文件时保留现场并要求修复。

## 卸载

在“已安装的应用 → HiRoute → 卸载”中开始。移除服务前，HiRoute 会恢复受支持的 Agent 配置；需要人工决策时停止卸载并列出 Agent ID，先在 HiRoute 中处理后再试。卸载只删除已登记的安装文件、Task、壳登录项及它自行加入的 CLI PATH 条目。数据默认保留，只有明确勾选才删除。独立 headless 状态目录不会成为 NSIS 的安装目录。

如果服务不可用，交互式恢复或卸载会另行确认，默认选择“否”。继续操作可能中断正在执行的工作，已配置的 Agent 也可能需要手动恢复；所有 HiRoute 数据都会保留。无人值守模式不会绕过这项确认，已发现的恢复冲突也仍会阻止卸载。首次安装失败且没有 Agent 数据时，可以不启动服务直接回滚。

升级提交后，会清理已验证的旧版本程序文件；卸载也会移除当前版本。被修改或外来的文件会保留。
