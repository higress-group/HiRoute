<p align="right"><a href="README.md">English</a></p>

<p align="center">
  <a href="https://hiroute.ai/">
    <img src="apps/website/public/brand/github-social-preview.png" alt="HiRoute——面向长程 Agent 的智能路由" width="100%">
  </a>
</p>

<p align="center">
  <strong>让 Codex 和 Claude Code 的订阅更耐用。</strong><br>
  在本地运行的模型路由：常规工作交给经济模型，难题留给最强的模型。
</p>

<p align="center">
  <a href="https://hiroute.ai/">官网</a> ·
  <a href="https://hiroute.ai/download/">下载</a> ·
  <a href="https://hiroute.ai/docs/">使用文档</a> ·
  <a href="https://hiroute.ai/docs/decision-extensions/">决策模型</a> ·
  <a href="CONTRIBUTING.md">参与贡献</a>
</p>

<p align="center">
  <a href="https://github.com/higress-group/HiRoute/releases/latest"><img alt="最新版本" src="https://img.shields.io/github/v/release/higress-group/HiRoute?display_name=tag&amp;sort=semver&amp;style=flat-square"></a>
  <a href="LICENSE"><img alt="Apache-2.0 许可证" src="https://img.shields.io/badge/license-Apache--2.0-5A6FE8?style=flat-square"></a>
  <a href="https://hiroute.ai/download/"><img alt="支持 macOS 和 Linux" src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux-171A21?style=flat-square"></a>
</p>

<p align="center">
  <a href="https://hiroute.ai/#tour">
    <img src="apps/website/public/media/hiroute-promo-zh-readme.jpg" alt="观看 1 分钟 HiRoute 产品介绍" width="720">
  </a>
</p>

继续使用你熟悉的 Codex、Claude Code、Qoder、Pi、OpenCode、Hermes 或 DeepSeek Harness。使用按难度分流计划时，
HiRoute 根据可核实的原生用户输入判断接下来的工作：读代码、查资料这类常规工作可以交给经济模型，
设计取舍和疑难修复留给订阅或主力模型。订阅、API Key 和本地模型集中管理，每个会话实际用了哪个
模型、用量和成本都可以查看；独立任务还可以委派给其他 Agent。

同一轮里的工具调用在历史连续、已捕获决策仍有效时通常沿用本轮决策，让增长中的上下文前缀持续复用缓存；
收到可用的新任务输入时，HiRoute 重新判断，也可能继续原模型。缓存实际能否复用取决于供应商。HiRoute
在本地运行，提供 Desktop 应用以及无界面 CLI 和后台服务。

客户端无法充分证明输入来源时，分类、历史捕获和跨轮评价仍可继续，来源不确定性会明确记录，缺失的执行历史仍标为不完整。
Codex 已压缩会话在 Gateway 重启后可通过新提问恢复，无需反复新建会话。Claude、Hermes 和 Qoder 保留智能路由，
但任务关联和评分仍可能有误差；Qoder 共享主子会话身份是接受的限制。已知辅助摘要不会进入主会话历史。
回答成功本身不代表智能路由或评分准确性已验证。具体条件见[模型路由与恢复限制](apps/website/content/guides/model-routing.zh.md)。

## News · 最新动态

- **最新稳定版本** — 提供 macOS Desktop 与 Linux 无界面版的 x86_64 / ARM64 安装包。[版本说明](https://github.com/higress-group/HiRoute/releases/latest) · [下载](https://hiroute.ai/download/)
- **2026-10-14** — [切换模型会损失缓存，智能路由还省钱吗？](news/2026-10-14-routing-cache-cost.zh-CN.md) · [官网阅读](https://hiroute.ai/news/routing-cache-cost/)
- **2026-10-04** — [HiRoute 智能路由实战：GPT-6 Astra × Qwen-3.8 Flash，同等质量下成本降低 90%](news/2026-10-04-astra-qwen.zh-CN.md)。[官网阅读](https://hiroute.ai/news/astra-qwen-smart-routing/) · [复现实验](experiments/README.md)
- [让 AI Agent 更能干，让模型订阅更耐用：HiRoute 正式开源](news/hiroute-open-source.zh-CN.md)。[官网阅读](https://hiroute.ai/news/hiroute-open-source/)
- [全部新闻](news/README.md)

## 为什么使用 HiRoute

| | |
| --- | --- |
| **按阶段路由长任务** | 不必让一个模型包办整段任务，而是根据后续工作选择合适的模型。 |
| **在降本时守住质量** | 胜任度用于阻止冒险降本，任务复杂度用于寻找采用经济模型的机会。 |
| **让接力有明确边界** | 校验能力要求、按顺序尝试候选，候选耗尽时明确停止，不静默改变行为。 |
| **让执行成为下一次决策的证据** | 保留任务类别、实际模型组与候选、工具结果、接受的输出和可选阶段胜任度。 |

HiRoute 当前提供固定模型、按难度分流、自定义分支和免费优先四种路由模式。有序候选与故障接力是
各模式共享的执行机制。产品还提供 Agent 工作计划与 Worker 委派，本地会话、用量、成本和胜任度
查看，以及用于接入自有决策服务的自定义扩展 API。

## 快速开始

### macOS Desktop

从 [hiroute.ai](https://hiroute.ai/download/) 下载当前 DMG，拖入“应用程序”目录。如果 macOS
要求手动批准，请按[自签名安装说明](docs/macos-installation.zh-CN.md)操作。HiRoute 支持
macOS 15 或更高版本，提供 Apple 芯片（`arm64`）与 Intel（`x86_64`）安装包。

### Linux 无界面版

支持 Linux `x86_64` 与 ARM64（`aarch64`），无需 `sudo`，安装到当前用户目录：

```sh
curl -fsSL https://hiroute.ai/install.sh | sh
hiroute service start --output json
hiroute system status --output json
```

安装器会校验已发布的 package manifest 与文件摘要，并且不会自动启动服务。服务生命周期和
机器可读命令见 [Linux 安装指南](https://hiroute.ai/docs/install-linux/)与
[CLI 参考](docs/standalone-cli.zh-CN.md)。

### 配置第一条路由

1. 接入受支持的本机订阅、注册 API 或兼容的自定义 API。
2. 如需模型判断，在**模型 → 决策模型**中连接百炼、OpenRouter Jev、TypeSafe 或兼容接入点。
   使用内置连接无需自行部署 Jev 服务。
3. 根据任务与成本目标创建并发布路由计划，按需使用按难度分流或自定义任务分支。
4. 接入 Agent 客户端，或开放 Worker 计划供主 Agent 委派任务。
5. 在会话和模型表现中查看任务类别、实际模型组和执行结果。

继续阅读[智能模型路由](https://hiroute.ai/docs/model-routing/)、
[智能任务路由](https://hiroute.ai/docs/task-routing/)或
[无界面 CLI 指南](https://hiroute.ai/docs/cli/)。

## 查看真实产品中的运行表现

<p align="center">
  <img src="decision-extensions/assets/quality-native-zh-CN.png" alt="HiRoute 桌面端会话：阶段胜任度与模型路由" width="100%">
</p>

查看模型在具体任务中的阶段评分，结合执行记录与用户反馈，为下一次模型选择和任务委派提供依据。
每个分数都对应具体执行阶段，可以继续查看决策依据与执行记录。

## 路由机制

![决策机制：判断当前任务、评价上一阶段，由 HiRoute 选择并执行模型组](decision-extensions/assets/jev-decision-zh-CN.svg)

内置决策模型按计划判断任务类别，在双模型组的路由中返回简单/复杂概率，并可评价上一执行阶段。
HiRoute 提供已发布的任务条件与评分标准，应用你设置的阈值，再选择模型组与组内有序候选。
在**模型 → 决策模型**中配置连接即可，无需部署扩展服务。

**按难度分流**只有一个任务范围，配置常规与主力两个模型组。本轮任务简单的概率达到门槛，且没有
适用的完整评分低于胜任度下限时，使用常规组；否则使用主力组。缺失或部分评分保留为未评分。
旧低分不会把后续轮次锁在主力组，新一轮也不会自动回到常规组，而是重新判断当前工作。

**自定义分支**区分写稿、审稿等任务类别，先选类别，再根据该类别内的简单/复杂程度选择常规组或
可选主力组。只有常规组的类别无需判断程度。评分始终属于真实执行过的阶段，写稿低分不会推动审稿
升级；只有本次完整、适用且与类别、发布版本和冻结标准匹配的评分，才能影响本轮模型组选择。

HiRoute 能将请求识别为同轮工具续接，且本轮冻结决策仍可复用时，保持已有决策。候选故障按计划有边界地
接力：常规组可以接力到同类别主力组，直接选中主力时只在主力组内接力，候选耗尽则明确失败。
故障接力不会生成胜任度分数或改变任务类别。

这套机制的原则是：

> **胜任度只负责阻止冒险降本，复杂度负责提供降本机会。**

按难度分流也可以使用启发式规则。自定义扩展是可选方式，适合需要自己负责模型接入、判断、评分与
上下文裁剪的场景。官方 [Jev 扩展](decision-extensions/extensions/jev-decider/README.zh-CN.md)
是该接口的一种实现。可从[决策模型与路由机制](decision-extensions/README.zh-CN.md)、
[自定义扩展 API 说明](decision-extensions/api/README.zh-CN.md)或规范化
[OpenAPI 文档](decision-extensions/api/decision.openapi.json)开始。

## 面向技术开发者

HiRoute 是 Rust workspace，Desktop 使用 Tauri，官网使用 Astro。Rust 工具链由
`rust-toolchain.toml` 固定；Desktop 使用 Node.js 24，官网使用 Node.js 22。

```sh
git clone https://github.com/higress-group/HiRoute.git
cd HiRoute

cargo fmt --check
cargo test --locked --workspace --exclude hiroute-desktop --all-features
```

前端改动请在受影响的应用目录运行对应检查：

```sh
cd apps/desktop   # 或 apps/website
npm ci --ignore-scripts
npm test
npm run build
```

原生依赖、Clippy、验证范围和语言约定见[贡献指南](CONTRIBUTING.md)。macOS 原生 Desktop 行为和
真实供应商集成需要相应环境，源码构建成功本身不等于产品验收通过。

### 代码结构

| 目录 | 职责 |
| --- | --- |
| `apps/desktop` | Desktop 界面与 Tauri 宿主 |
| `crates/cli`、`crates/daemon` | 无界面客户端与本地 role-all 服务 |
| `crates/application`、`crates/client-core` | 应用工作流和共享客户端行为 |
| `crates/gateway`、`crates/gateway-core` | Agent 协议入口、路由、执行与故障接力 |
| `crates/observation`、`crates/local-storage` | 本地执行证据、查询与持久化 |
| `decision-extensions` | 决策模型指南、路由机制、扩展 API、产品截图和可选 Jev 扩展 |
| `contracts`、`assets` | 当前运行合同、模型元数据和产品资源 |
| `apps/website` | `hiroute.ai` 双语官网、下载和版本发布 |
| `e2e`、`tools`、`scripts` | 产品场景、验证工具、打包和自动化 |

## 项目状态

最新稳定版本提供 macOS Desktop 与 Linux 无界面版，两种平台均支持 x86_64 和
ARM64。最新安装包与版本说明见[官网](https://hiroute.ai/download/)。

HiRoute 使用 [Apache License 2.0](LICENSE)。缺陷和功能建议请提交到
[GitHub Issues](https://github.com/higress-group/HiRoute/issues)。安全问题和 crash report 请按
[SECURITY.md](SECURITY.md)中的流程提交。

友情链接：[LINUX DO](https://linux.do/)（技术交流社区）。
