# Qoder CLI 模型路由与订阅接入

把 HiRoute 路由加到 Qoder CLI 的模型列表，也可以通过 Magpie headless 接入 Qoder 订阅模型。比如让 DeepSeek-Flash 做常规开发，复杂轮次交给 Kimi-K3.1-Pro，再用 Pi、DeepSeek Harness 或 Codex CLI 执行独立任务。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/)

- 已发布路由时，[直接在 Qoder CLI 里使用](#use-in-qoder)。
- 想复用 Qoder 账号的模型时，[接入 Qoder 订阅](#connect-qoder)，再配置难度分流和执行 Agent。

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/qoder-model-routing/
请帮我把已有路由加入 Qoder CLI，或用 Magpie 提供的 Qoder 模型新建难度路由。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 检查 Magpie 模型接入：hiroute compute list · compute scan · compute connection test / preview / apply
- 配置常规组、主力组与执行方：routing options · routing preview / apply · worker executors
- 把路由加入 Qoder CLI：agents connect preview / apply
- 核对实际运行：worker plans · worker result · sessions timeline
界面和 CLI 共用配置和记录。由我在本机完成登录，模型以实际目录为准。参数按 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 确认。
```

</details>
</aside>

<h2 id="use-in-qoder">在 Qoder CLI 里使用已有路由</h2>

在 Qoder CLI 完成登录。在 HiRoute 打开“Agent → Qoder → 模型路由”，选中已发布的计划并保存。重启 Qoder CLI，输入 `/model`，就能在原有模型旁边选用路由。还没有计划时，可以[用已有模型新建路由](/zh/docs/model-routing/)，或继续按下面的步骤接入订阅。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-qoder-zh.webp"><img src="/guide-assets/agent-guide-qoder-zh.webp" alt="Qoder CLI 模型路由设置中勾选 Qoder 日常开发计划" width="1200" height="900" loading="lazy" /></a>
  <figcaption>把已发布的路由加入 Qoder CLI 的模型选择器。</figcaption>
</figure>

<h2 id="connect-qoder">1. 接入 Qoder 订阅模型</h2>

[启动 Magpie headless 服务](/zh/docs/magpie-model-sources/#start-magpie)，或复用正在运行的实例。首次接入账号时，运行 `magpie login qoder` 完成浏览器授权。如果使用[社区 Qoder 插件](https://github.com/magpie-community/plugins/blob/main/registry.json)，先运行 `magpie plugin add @magpie-community/opencode-qoder-auth`，再运行 `magpie plugin login qoder`。Qoder CN 使用 `qoder-cn`。

授权后[查询模型目录](/zh/docs/magpie-model-sources/#model-catalog)，用完整模型 ID [在 HiRoute 保存 Magpie API](/zh/docs/magpie-model-sources/#save-api)。如果已有受支持的 Agent 配置指向 Magpie，也可以先在“模型 → 通用模型 → 添加模型”点“扫描本机”。下面的 Pi 示例可从 Responses 协议开始。

<figure class="guide-shot-models">
  <a href="/guide-assets/agent-guide-qoder-models-zh.webp"><img src="/guide-assets/agent-guide-qoder-models-zh.webp" alt="HiRoute 中通过 Magpie 接入的 Qoder DeepSeek-Flash 与 Kimi-K3.1-Pro 模型" width="1200" height="900" loading="lazy" /></a>
  <figcaption>保存接入后，在模型列表中找到 DeepSeek-Flash 与 Kimi-K3.1-Pro。</figcaption>
</figure>

## 2. 让快模型先做，主力处理难点

在“智能路由 → 新建智能路由”选择“按难度分流”，命名为“Qoder 日常开发”：

- **常规模型**：例如 `qoder/dfmodel`（DeepSeek-Flash），用于读代码、整理线索和小改动。
- **主力模型**：例如 `qoder/kmodel_latest`（Kimi-K3.1-Pro），用于定位复杂故障、改接口和审查方案。
- **判断方式**：先选“启发式规则”跑通；想看阶段评分或让主力在适用的低分之后接手，再配置[决策模型](/zh/docs/model-routing/#difficulty-routing)。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-zh.webp"><img src="/guide-assets/agent-guide-difficulty-zh.webp" alt="Qoder 路由中的常规组 DeepSeek-Flash 和主力组 Kimi-K3.1-Pro" width="1200" height="900" loading="lazy" /></a>
  <figcaption>快模型负责常规步骤，主力模型留给更难的轮次。</figcaption>
</figure>

模型随账号目录变化，可以用已有模型替换任意一组。点“启用”发布计划，再[把路由加入 Qoder CLI](#use-in-qoder)，或继续绑定执行 Agent。

## 3. 把同一套模型交给 Pi 执行

在计划里打开“更多设置 → 任务委派”，开启“允许委派任务给执行 Agent”，写明用途（例如“阅读模块并给出重构建议”），选 **Pi**，按页面提示检查依赖并发布。也可以用同一组模型另建计划，为 Codex CLI 或 DSH 指定不同的用途。每份可委派计划绑定一个执行 Agent。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-pi-worker-zh.webp"><img src="/guide-assets/agent-guide-pi-worker-zh.webp" alt="HiRoute 路由计划的任务委派区域选择 Pi 为执行 Agent" width="1200" height="900" loading="lazy" /></a>
  <figcaption>任务交给 Pi 执行，选哪组模型仍由这条 HiRoute 路由决定。</figcaption>
</figure>

在常用的主 Agent 上[开启任务路由](/zh/docs/task-routing/#delegation-skill)，试一句：“使用 HiRoute，把当前项目的模块结构梳理委派给 Qoder 日常开发，列出入口和测试，不修改文件。”完成后到“Agent → 任务记录”看执行方和结果，在“会话 → 路由时间线”查看实际选用的模型。

下一步：[用 WorkBuddy 模型做同样的分流](/zh/docs/workbuddy-model-routing/) · [配置 Pi 执行 Agent](/zh/docs/pi-model-routing/)
