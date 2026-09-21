# macOS 安装与首次启动

HiRoute 桌面应用当前支持 macOS 15.0 或更高版本。短期提供的是自签名安装包，没有 Apple Developer ID 签名和公证；因此 macOS 会要求你明确确认首次启动。

## 下载正确的安装包

在 [下载页](/download/) 按 Mac 的架构选择 DMG：

- Apple 芯片（M1、M2、M3、M4 等）选择 `arm64`。
- Intel Mac 选择 `x86_64`。

下载链接应来自 `hiroute.ai/releases/<版本>/`；文件名包含构建来源提交的短 SHA。用下载页给出的完整 SHA256 摘要核对文件：

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

## 安装终端入口（可选）

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

这个终端入口连接 HiRoute 的本机服务；桌面应用应已安装并让本机服务保持可用。更多命令见 [HiRoute CLI](/docs/cli/)。
