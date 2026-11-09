# Omarchy 安装与首次启动

HiRoute 提供适用于 Omarchy 的签名 Arch 包，支持 x86_64 和 ARM64。安装包包含桌面应用、启动器入口和配套的 CLI 与服务。应用和 CLI 共用当前用户的模型、路由、Agent 与会话。

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>

复制以下指引到 Agent 对话中，再说明你想完成的操作。内容包含文档链接、内置技能入口和 CLI 线索。

<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
请先阅读以下文档，结合我在对话中提出的目标，用 HiRoute CLI 完成“Omarchy 安装与首次启动”相关操作。目标不明确时，只询问必要信息。

本页文档：https://hiroute.ai/zh/docs/install-omarchy/
CLI 参考：https://hiroute.ai/zh/docs/cli/

先按文档选择架构，核对 OpenPGP 主密钥指纹和安装包签名，再使用 pacman 安装。从 Apps 打开 HiRoute，通过启动页安装或更新当前用户的后台服务。安装后，引导我在“Agent → HiRoute 管理 → 启用”中开启产品内置管理技能，后续配置交给该技能：https://hiroute.ai/zh/docs/#management-skill

操作线索（按本次目标选用；下面均为 hiroute 子命令）：
- 检查 CLI 和服务是否可用：--help · system status
- 查看当前接入和下一步：quickstart
- 开启产品内置管理技能：agents connect preview --management enable → agents connect apply
- 卸载前恢复 Agent 接入：agents restore preview → agents restore apply

界面与 CLI 连接同一个 HiRoute 服务，共用配置和执行记录。先读取现状并复用已有配置；参数以本机命令 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。按内置技能完成所需预览、应用与核验，最后说明结果。
```

</details>
</aside>

## 1. 选择架构

在终端运行 `uname -m`。结果为 `x86_64` 时，在 [Omarchy 安装页](/zh/download/#omarchy) 选择 **x86_64**；结果为 `aarch64` 时选择 **ARM64**。

将安装包、`.sig` 签名和 `.sha256` 校验文件保存到同一文件夹。安装页提供与所选安装包对应的完整命令。保留下载时的文件名，包括 `.pkg.tar.xz` 或 `.pkg.tar.zst` 后缀。

## 2. 核对安装包和发布密钥

在下载文件夹中，将 `package` 替换为实际下载的安装包文件名，再检查摘要。

```sh
package='PASTE_EXACT_PACKAGE_FILENAME'
sha256sum --check "$package.sha256"
```

下载 [HiRoute 发布公钥](https://raw.githubusercontent.com/higress-group/HiRoute/main/assets/packaging/omarchy-release-key.asc)，保存为 `omarchy-release-key.asc`，然后查看指纹。

```sh
gpg --show-keys --with-fingerprint omarchy-release-key.asc
```

主密钥指纹应为 `CB5F8ADEDC272ED8879D206C5642FA57EA8DF169`。摘要或指纹不一致时，请停止安装并重新核对来源与文件。

## 3. 安装签名包

确认指纹后，将发布密钥加入 pacman 密钥环，并在本机信任该密钥。保留安装包旁边的签名文件后执行安装。

```sh
sudo pacman-key --add omarchy-release-key.asc &&
sudo pacman-key --lsign-key CB5F8ADEDC272ED8879D206C5642FA57EA8DF169 &&
sudo pacman -U "$package"
```

pacman 按正常签名策略验证安装包。出现校验失败时，先解决文件或签名问题，再继续。

## 4. 从 Apps 打开 HiRoute

在 Omarchy 应用启动器中打开 **HiRoute**。首次运行选择 **安装后台服务**，为当前用户配置 CLI、服务与本机控制台，也可以选择登录时启动服务。已有的兼容安装会保留监听和服务设置。

进入控制台后，[接入第一个模型](/zh/docs/models/)，再按 [快速开始](/zh/docs/) 继续配置。关闭桌面窗口后，后台服务会继续运行。

## 更新 HiRoute

下载并验证新版本安装包，再使用 `pacman -U` 安装。如果启动页提供 **检查服务更新…**，先查看版本和重启影响，再选择 **更新并重启**。更新会保留已有配置和数据。

需要稍后更新服务时，可以选择 **在浏览器中打开** 继续使用现有服务。若本机控制台未启用，请按启动页给出的终端步骤处理。

## 移除服务或应用

移除当前用户的运行环境时，打开 **设置 → 桌面应用 → 移除服务…**，并查看受影响的 Agent。需要处理的 Agent 设置应先在 Agent 页面解决，再重试。保存的模型、方案和历史记录可在重新安装后继续使用。

系统包和启动器入口使用 `sudo pacman -R hiroute-desktop` 单独移除。仅移除系统包会保留当前用户的运行环境和数据。
