# HiRoute

[English](README.md)

HiRoute 是面向长程任务、本地运行的模型路由与 Agent 协作引擎。它将模型来源、路由计划、Agent
接入和执行证据集中管理，提供 Desktop、headless CLI 与本地后台服务。

在长程任务中，HiRoute 可以在 Agent turn 和 ContextHold 边界重新选择模型。这些位置本身会改变
稳定前缀，因此每个阶段内仍保持前缀稳定，对模型供应商的 KV cache 友好。下一次决策可以结合新的
用户请求与上一阶段的胜任度表现。

## 当前能力

- 接入受支持的本机订阅、注册 API 和兼容的自定义 API。
- 发布固定模型、智能省钱、免费优先和有序故障接力路由计划。
- 为受支持的 Agent 客户端配置模型接入和协作计划。
- 通过 CLI 运行和管理 Worker 任务，并在 Desktop 查看任务状态与结果。
- 查看本地会话、请求执行、已知用量与成本，并删除选定数据。
- 通过公开 Decision API 扩展模型选择；官方 Jev 决策器提供可部署的参考实现。

HiRoute 当前处于 MVP 阶段。下载页会说明各版本已验证的平台和架构；源码存在不代表未验证环境已经
完成产品验收。

## 开始使用

- [下载 HiRoute](https://hiroute.ai/download/)
- [使用文档](https://hiroute.ai/docs/)
- [macOS 自签名安装](docs/macos-installation.zh-CN.md)
- [Linux headless 与 CLI](docs/standalone-cli.zh-CN.md)
- [模型选择决策机制与扩展](decision-extensions/README.zh-CN.md)
- [Decision API OpenAPI 文档](decision-extensions/api/decision.openapi.json)

参与开发时，Rust 版本由 `rust-toolchain.toml` 固定，Desktop UI 使用 Node.js/npm 与 Tauri。
请阅读 [贡献指南](CONTRIBUTING.md)、[Desktop 开发入口](apps/desktop/README.zh-CN.md)和
[官网开发及发布指南](apps/website/README.md)。

## 代码结构

| 目录 | 职责 |
| --- | --- |
| `apps/desktop` | Desktop 页面与 Tauri 宿主 |
| `apps/website` | `hiroute.ai` 双语官网、下载与 OSS 发布 |
| `crates/cli`、`crates/daemon` | 命令行客户端和本地服务 |
| `crates/client-core`、`crates/application` | 共享客户端与业务编排 |
| `crates/gateway-core`、`crates/gateway` | 协议适配、请求执行与模型故障接力 |
| `crates/observation`、`crates/local-storage` | 本地执行证据、查询与持久化 |
| `contracts`、`assets` | 运行合同、模型资料与产品资源 |
| `decision-extensions` | 决策机制、OpenAPI 合同与官方扩展 |
| `tools`、`e2e` | 验证工具与可执行产品场景 |

## 许可与安全

HiRoute 使用 [Apache License 2.0](LICENSE)。安全问题和 crash report 请按
[SECURITY.md](SECURITY.md) 提交。
