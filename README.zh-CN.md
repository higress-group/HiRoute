<p align="right"><a href="README.md">English</a></p>

<p align="center">
  <a href="https://hiroute.ai/">
    <img src="apps/website/public/brand/github-social-preview.png" alt="HiRoute——面向长程 Agent 的智能路由" width="640">
  </a>
</p>

<p align="center">
  <strong>让 Codex 和 Claude Code 的订阅更耐用。</strong><br>
  在熟悉的 Agent 中组合订阅、API 和本地模型。<br>
  常规工作交给经济模型，困难任务留给主力。
</p>

<p align="center">
  <a href="https://hiroute.ai/download/">下载</a> ·
  <a href="#快速开始">快速开始</a> ·
  <a href="#jev-114-选模实验">实验结果</a> ·
  <a href="https://hiroute.ai/docs/">使用文档</a> ·
  <a href="CONTRIBUTING.md">参与贡献</a>
</p>

<p align="center">
  <a href="https://github.com/higress-group/HiRoute/releases/latest"><img alt="最新版本" src="https://img.shields.io/github/v/release/higress-group/HiRoute?display_name=tag&amp;sort=semver&amp;style=flat-square"></a>
  <a href="LICENSE"><img alt="Apache-2.0 许可证" src="https://img.shields.io/badge/license-Apache--2.0-5A6FE8?style=flat-square"></a>
  <a href="https://hiroute.ai/download/"><img alt="支持 macOS 和 Linux" src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux-171A21?style=flat-square"></a>
</p>

## 熟悉的 Agent，更多模型选择

继续使用 **Codex、Claude Code、Qoder、Pi、OpenCode、Hermes 或 DeepSeek Harness**。
HiRoute 在本地运行，将受支持的订阅、API 接入和本地模型集中管理。例如，用经济型 API 模型
搭配自己的 Codex 订阅，或在同一个受支持的来源中组合较小模型与主力模型。

| 你希望做到 | HiRoute 提供什么 |
| --- | --- |
| **保留熟悉的工作流** | 继续在原来的 Agent 中工作，让不同模型在同一会话中分工。 |
| **让主力额度更耐用** | 把合适的工作交给经济模型，同时保留主力保护。 |
| **逐步试用新模型** | 选择初始信任程度，保留熟悉的主力，再调整新模型承担的范围。 |
| **看懂每次选择** | 查看实际模型、切换依据、阶段表现、用量和费用估算。 |

## 同一个任务，为不同阶段选择合适的模型

按难度分流同时关注接下来的工作和此前的执行表现：

- **常规工作**：任务满足你设定的标准时，使用经济模型。
- **相关失败仍未解决**：上一阶段表现不佳，且问题仍影响当前工作时，保留主力保护。
- **问题解决或任务变化**：重新判断当前工作，以及旧问题是否仍然相关。

**System One 判断三件事：当前工作有多难、上一阶段做得怎样、之前的问题是否仍影响现在。**
HiRoute 结合这些判断与你的路由计划，选择实际执行的模型。

<!-- README-SCREENSHOT S1: decision-extensions/assets/readme-session-zh-CN.png -->
> **截图待补：真实会话时间线。** 展示同一任务的多个阶段、清晰的模型名称，以及选中阶段的切换原因。

你可以继续追问，也可以让 Agent 自动推进长任务。HiRoute 在受支持的新输入与上下文重建窗口
重新判断，同一阶段的工具续接通常沿用已有决策。详见[路由行为与客户端支持范围](apps/website/content/guides/model-routing.zh.md)。

[观看产品介绍](https://hiroute.ai/#tour) · [了解路由模式](decision-extensions/README.zh-CN.md)

## 新模型逐步试，主力随时准备接手

选中想试用的新模型，搭配熟悉的主力，再选择初始信任程度。可以点击档位，也可以拖动滑块调整；
检查改动后发布，并映射到自己的 Agent。新模型承担多少工作，由你决定。

<!-- README-SCREENSHOT S2: decision-extensions/assets/readme-trial-zh-CN.png -->
> **截图待补：试用配置。** 展示新模型、主力模型及其来源，以及信任滑块和可点击档位。

### 根据执行报告，决定下一步怎样调整

会话时间线展示每次实际用了哪个模型、为什么切换。模型表现报告汇总阶段评分、常规组承担比例
和可用的费用估算。证据足够时，你可以查看调整建议、检查改动，再决定是否发布。

<!-- README-SCREENSHOT S3: decision-extensions/assets/readme-report-zh-CN.png -->
> **截图待补：真实执行报告。** 展示报告范围、已评分阶段；如果已有充分依据，再展示对应的调整建议。

也可以按自己的任务调整判断标准，使用[公开评测方法](experiments/system-one-routing-paper/README.md)
对照检查提示词改动的效果。

## Jev 1.14 选模实验

在同轮对照中，两种方法获得相同的上下文与完整选模策略：一种直接选择模型组，另一种分别回答
三个问题，再由 HiRoute 组合判断。

![Jev 1.14 最终选组准确率：工程从 69.23% 到 96.15%；数学从 62.50% 到 97.92%](decision-extensions/assets/readme-jev114-zh-CN.svg)

| 场景 | 完整策略直接选组 | 三问组合 |
| --- | ---: | ---: |
| 工程 | 69.23% | **96.15%** |
| 数学 | 62.50% | **97.92%** |

指标为最终选组准确率，按工作流等权统计，来自 2026 年 10 月 28 日对已有案例的重测。
工程包含 26 个工作流、104 条记录，其中 86 条有明确选组答案；数学包含 6 个工作流、48 条明确的
依赖状态案例。每个条件调用一次，同轮交错执行，边界与证据不足案例单列。
这些受控结果衡量指定策略下的选组表现，不代表任务完成率或实际节省比例。

[研究与方法](experiments/system-one-routing-paper/README.md) ·
[评测集与审核说明](experiments/system-one-routing-paper/DATA-CARD.md)

## 快速开始

[**下载 HiRoute**](https://hiroute.ai/download/) ·
[安装指南](https://hiroute.ai/docs/) ·
[最新版本](https://github.com/higress-group/HiRoute/releases/latest)

1. **接入模型**：加入受支持的订阅或 API，选择经济模型与主力。
2. **配置分工**：在**模型 → 决策模型**中连接判断服务，选择信任程度，发布并映射到 Agent。
   内置连接支持百炼、OpenRouter Jev、TypeSafe 与兼容接入点，无需自行部署 Jev。
3. **照常工作**：继续在原来的 Agent 中使用，到 HiRoute 查看模型切换和执行报告。

<details>
<summary>macOS Desktop 与 Linux 无界面版安装</summary>

**macOS：** 下载 DMG，拖入“应用程序”；如系统要求批准，请按
[自签名安装说明](docs/macos-installation.zh-CN.md)操作。支持 macOS 15+、Apple 芯片与 Intel。

**Linux：** 支持 x86_64 与 ARM64，无需 `sudo`，安装到当前用户目录：

```sh
curl -fsSL https://hiroute.ai/install.sh | sh
hiroute service start --output json
hiroute system status --output json
```

安装器校验已发布的 manifest 与文件摘要；服务由用户显式启动。详见
[Linux 安装指南](https://hiroute.ai/docs/install-linux/)与 [CLI 参考](docs/standalone-cli.zh-CN.md)。

</details>

## 继续探索

- **更多路由模式**：除按难度分流外，还支持固定模型、自定义分支与免费优先。
  [智能模型路由](https://hiroute.ai/docs/model-routing/)
- **Agent 协作**：通过 Worker 计划委派独立任务。[智能任务路由](https://hiroute.ai/docs/task-routing/)
- **自己的决策服务**：配置判断标准，或实现可选的[决策扩展 API](decision-extensions/api/README.zh-CN.md)。
- **详细行为**：候选故障接力、评分适用条件、恢复与客户端限制，统一查看
  [模型路由指南](apps/website/content/guides/model-routing.zh.md)。

## 最新动态

- [最新稳定版本与版本说明](https://github.com/higress-group/HiRoute/releases/latest)
- **2026-10-14** — [切换模型会损失缓存，智能路由还省钱吗？](news/2026-10-14-routing-cache-cost.zh-CN.md)
- [全部新闻与实战案例](news/README.md) · [实验材料](experiments/README.md)

## 参与贡献

开发环境与验证方法见[贡献指南](CONTRIBUTING.md)，各项能力的实现入口见
[代码地图](docs/code-map/README.md)。HiRoute 使用 [Apache License 2.0](LICENSE)。
缺陷和功能建议请提交到 [GitHub Issues](https://github.com/higress-group/HiRoute/issues)；
安全问题与 crash report 请按 [SECURITY.md](SECURITY.md) 处理。

友情链接：[LINUX DO](https://linux.do/)（技术交流社区）。
