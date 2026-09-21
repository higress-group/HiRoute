# macOS 安装候选

[English](macos-installation.md)

安装目标为 macOS 15+，分别提供 Apple Silicon（arm64）和 Intel（x86_64）单架构包。
最低版本取自随包官方 CPA release 的实际构建信息；旧系统、Intel 真机及 Windows 的验证结果须单列，不能由 arm64 测试推断。
仓库提供候选构建入口；构建成功不等于 Finder、账号、Worker 或正式分发验收通过。

## 安装与 CLI

打开候选 DMG，将 `HiRoute.app` 拖到镜像中的 `Applications` 入口，拷贝完成后推出镜像，
从 `/Applications/HiRoute.app` 打开。ZIP 仍可用于直接解压安装。
首次安装自签名包可能被系统拦截；它没有 Developer ID 公证。普通工程候选不自动具备公开分发资格；
只有完成完整性、安装、组件声明和发布审核并进入官网发布清单的精确产物才作为公开下载。
请使用可信来源的包并按 macOS 的应用打开确认流程处理，不关闭系统安全检查。

App 只能从 `/Applications/HiRoute.app` 或当前用户的 `~/Applications/HiRoute.app` 运行后安装
用户级终端入口。在“设置 → CLI”中显式选择安装，HiRoute 会创建
`~/.local/bin/hiroute` 到包内 CLI 的符号链接；不会复制二进制、请求 sudo 或修改 shell 文件。
如果该路径已有非 HiRoute 条目会报告冲突并保持原样。App 位于 DMG、下载临时目录或其他位置时
不会安装入口。

确保 `~/.local/bin` 已在 PATH 中；zsh 可由用户自行加入 `~/.zprofile`：

```sh
export PATH="$HOME/.local/bin:$PATH"
hiroute worker list --output json
hiroute --help
```

设置页会分别显示链接 missing/valid/broken/conflict、PATH 是否包含入口目录、是否被更早的同名
命令遮蔽，以及 daemon 是否可用；链接有效不代表服务已启动。repair/remove 只处理指向受支持
HiRoute.app 的自有链接，不删除 `~/.local` 或 `~/.local/bin`。受管 Agent 启动时也只有在该链接
重新校验为当前可信 App CLI 后才把 `~/.local/bin` 前置到子进程 PATH。

Agent 的受管技能和启动器继续使用 `hiroute` 命令名及既有接入预览/确认流程。
应用运行不需要 Rust、Go、Docker、npm 或源码。Worker 自己的 CLI、ACP adapter 及
必要 runtime 仍按设置中的依赖引导安装，它们不包含在 HiRoute 安装包内。

CLI 默认连接当前用户的 `~/Library/Application Support/ai.hiroute.desktop/run`。
`HIROUTE_RUNTIME_DIR` 优先于 `XDG_RUNTIME_DIR`，两者都是明确的开发/测试覆盖；
空值或相对路径返回不可用，不会回落到另一实例。未运行时先打开 HiRoute；
服务不可用时恢复 Desktop 后按原操作/任务身份查询，不重复提交未知结果的请求。

CPA 随包提供，只在获准订阅路径使用。缺失、替换或不能执行时订阅检查失败，
控制和其他可用 API 来源保持可用。用完整候选替换损坏 App；不要手动安装另一 CPA。

## 手动升级与卸载

1. 在 HiRoute 中明确退出，确认本次服务和任务完成停止；关闭窗口只隐藏应用。
   生命周期完整收尾的组合验收跟踪 #93，不能仅凭窗口消失判断已停止。
2. 替换 `/Applications/HiRoute.app`，保留应用数据，再启动。固定位置让用户级符号链接目标不变。
3. 若移动了 App 或接入检查报告路径失效，使用设置中的接入预览和确认重新配置；
   若出现用户配置漂移，先解决冲突，不能直接覆盖。登录项在设置中检查，必要时关闭再启用。

卸载前先通过设置预览/恢复受管模型及 Agent 接入，再关闭“登录后后台启动”，明确退出，
然后删除 App。删除 App 默认保留数据。若选择彻底清理，确认无任务运行、已恢复接入且
不再需要会话/日志/回执后，只删除当前用户 `~/Library/Application Support/ai.hiroute.desktop`
中的 HiRoute 数据。不要删除 `.codex`、`.claude`、用户项目或原有凭据。
若 App 已损坏，先恢复相同安装位置的完整候选以执行接入恢复；不要手工猜测配置覆盖。

## 构建与检查

在 macOS 的准确、干净 committed checkout 执行（默认宿主架构）：

```sh
RUSTC_WRAPPER=sccache python3 scripts/package-desktop.py build \
  --arch arm64 --cpa-source-repo /path/to/CLIProxyAPI
# 使用构建结果中的实际路径：
python3 scripts/package-desktop.py verify /absolute/output/HiRoute.app
python3 scripts/package-desktop.py verify-dmg /absolute/output/HiRoute-VERSION-SHA-macos-arm64-trial.dmg
```

构建复用本机现有 Rust 验证共享锁，输出保留在该 checkout 的 `target/`，使用 sccache
时也不能外置 Cargo target。它执行锁定依赖的 release 编译和前端构建；一般开发验证仍使用 Debug。
从显式指定的 CPA checkout 提取 `vendor/cpa/source.json` 固定的提交，核对并应用同目录的启动管道补丁，
使用 Go 构建，再对 CPA 签名并将最终摘要清单编入 Desktop；不进行 deep 重签。
补丁只让父进程通过继承管道传入已有实例管理凭据，保留 loopback 与远程管理禁用规则；不改变模型协议。
构建机须已安装 Go，并能取得锁定的 Go 依赖；不接受任意预编译 CPA，不在用户启动时下载。
上游 LICENSE 自动放入 `Licenses/CLIProxyAPI-LICENSE`，源码、补丁、编译器和产物摘要记录在构建证据中。
打包器还会从锁定的 Cargo/npm 依赖和同一 CPA commit 自动生成
`THIRD-PARTY-LICENSES.txt` 与机器可核对的许可证清单，并连同 HiRoute 的 Apache-2.0
许可证一起放入 `Contents/Resources/Licenses`；缺少可确认的许可证材料会直接中止构建。

Intel 包使用 `--arch x86_64`，需要已安装的 `x86_64-apple-darwin` Rust target；
Apple Silicon 使用 `--arch arm64` 和 `aarch64-apple-darwin` target。
交叉构建时必须能执行目标架构 CPA 的版本探针（Apple Silicon 上需要已有 Rosetta），不会自动安装 Rosetta。
两个架构独立输出到 `target/desktop-package/CANDIDATE_SHA/ARCH/ROUND/`，不会混用组件或合成 universal 包。
重复执行同一命令即可再构建；UTC 时间与随机后缀区分每轮，不覆盖先前产物，也不清理 target。
DMG/ZIP 文件名包含版本、12 位提交号、架构及 `trial`（正式包为 `developer-id`）。
每轮 `result.json` 保留完整提交号、DMG/ZIP SHA256、组件摘要和镜像验证结果；`build.log` 保留命令输出。
DMG 使用系统 `hdiutil` 生成压缩只读镜像，自动验证完整性、只读挂载入口及 App 组件，然后卸载挂载。
如果卸载失败，构建失败且保留日志中的挂载路径；关闭访问该路径的窗口后用 `hdiutil detach PATH` 重试，勿强制卸载其他镜像。

人工 review：先核对 `shasum -a 256 FILE.dmg` 与本轮 `result.json`，按上述步骤安装。
替换前按“手动升级”退出旧 App，避免覆盖运行中的实例；首次隔离验证可使用独立 macOS 测试账号，
不要把开发覆盖环境下的成功当作普通 Finder 启动已通过。
反馈记录完整提交号（见 App 内 `Contents/Resources/installation.json`）、DMG SHA256、架构、macOS 版本、
复现步骤与实际结果；系统拦截时记录提示并在“系统设置 → 隐私与安全性”核对可信 App 后按系统允许的流程打开。
不移除隔离属性、不关闭 Gatekeeper/SIP；ad-hoc 包可能仍需人工授权，构建成功不保证可启动。

原生宿主、daemon、CLI、CPA 位于 `Contents/MacOS`；当前模型目录编入 daemon，前端编入宿主。
`Contents/Resources/installation.json` 记录候选 SHA、版本、最终摘要和系统动态依赖。
`dependency-inventory.json` 是归属核对清单（含可能未链接的 Cargo 依赖），不替代许可证原文。

需要 Developer ID 分发的候选使用已有公证钥匙串 profile；组件许可证仍由同一锁定依赖收集器生成：

```sh
python3 scripts/package-desktop.py build \
  --cpa-source-repo /path/to/CLIProxyAPI \
  --identity 'Developer ID Application: YOUR ORGANIZATION (TEAMID)' \
  --notary-profile YOUR_PROFILE
```

凭据只来自本机钥匙串，不放进仓库或命令输出。工具签名嵌套二进制和 App，提交公证，
staple 并验证后重建 zip，再封装、签名和公证 DMG；两次公证均须 Accepted。自签名构建在包内明确标记
`controlled-trial`，不声称 Developer ID 签名或公证；该技术标记本身也不代表已经完成组件声明、安装和发布审核。
短期公开自签名版本仍须显式提供并核对适用组件材料，通过上述审核后才可进入官网发布清单。

安装场景需另行记录：安装位置/Finder 无覆盖启动、CLI 同服务和未运行错误、实际委派/结果、
获准订阅调用、CPA 缺失/替换/失败时健康 API/控制、登录启动、退出、手动升级及恢复卸载。
必须注明候选 SHA、安装包摘要和实际执行结果；组件/fixture 测试不能替代这些证据。
