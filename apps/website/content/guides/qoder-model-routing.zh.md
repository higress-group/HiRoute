# Qoder CLI 模型路由与订阅接入

先用 Magpie headless 发现 Qoder 订阅里的模型，再用 HiRoute 配置按难度分流。例如让 DeepSeek-Flash 处理常规开发，复杂轮次交给 Kimi-K3.1-Pro。最后把路由接入 Qoder CLI；如果你的 Qoder 不支持 BYOK，也可以按本页截图在 Codex Desktop 中使用这些模型。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/qoder-model-routing/
请帮我按本页完成获取模型、配置智能路由和接入 Agent，再用本页的轻量 Pi 场景说明如何用更少的主力模型 token 完成独立任务。按我的需要启用委派。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 检查 Magpie 模型接入：hiroute compute list · compute scan · compute connection test / preview / apply
- 配置常规组和主力组：routing options · routing preview / apply
- 有自定义模型入口时接入 Qoder CLI，否则按本页接入 Codex Desktop 独立窗口。配置用 agents connect preview / apply，独立窗口从 HiRoute 启动，也可在本机图形会话用 agents desktop launch。
- 核对实际选模，需要任务委派时再选择执行方：sessions timeline · worker executors · worker plans · worker result
界面和 CLI 共用配置和记录。由我在本机完成登录，模型以实际目录为准。参数按 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 确认。
```

</details>
</aside>

[1. 扫描或登录，获取模型](#connect-qoder) · [2. 配置智能路由](#configure-routing) · [3. 接入 Agent](#use-in-qoder) · [4. 了解任务委派](#task-delegation)

<h2 id="connect-qoder">1. 扫描或登录，获取模型</h2>

用 Magpie headless 读取 Qoder 订阅可用的模型，再把模型接入 HiRoute。Magpie 负责账号登录和提供本机模型 API。

在 HiRoute 服务所在电脑[安装 Magpie CLI](https://usemagpie.ai/docs/start)。按系统选择启动方式。

- macOS 或使用 systemd 的 Linux 运行 `magpie daemon install --no-web`，用 `magpie daemon status` 确认服务状态。需要启动已安装的服务时，运行 `magpie daemon start`。
- Windows 或其他无法安装后台服务的环境，另开终端运行 `magpie serve` 并保持运行。

已有网关时可直接登录。

运行 `magpie login qoder`。使用社区插件时，先运行 `magpie plugin add @magpie-community/opencode-qoder-auth`，再运行 `magpie plugin login qoder`。Qoder CN 使用 `magpie plugin login qoder-cn`。

在浏览器完成授权后查询目录。Windows PowerShell 中使用 `curl.exe`。

```sh
curl -fsS -H 'Authorization: Bearer magpie' http://127.0.0.1:3425/v1/models
```

按账号返回的完整 ID 选择模型。例如 `qoder/dfmodel` 对应 DeepSeek-Flash，`qoder/kmodel_latest` 对应 Kimi-K3.1-Pro。

在 HiRoute 打开“模型 → 通用模型 → 添加模型 → **新建 API 接入 → 自定义 API**”，按下面填写并检查。

| 设置 | 填写内容 |
| --- | --- |
| 连接名称 | 例如“Magpie Qoder” |
| API 地址 | `http://127.0.0.1:3425/v1`，网关改过地址时用实际值 |
| 协议 | Responses，适用于下方的 Codex Desktop 接入 |
| API Key | 默认本机网关为 `magpie`，改过 Key 时填自己的值 |
| 模型 | 从返回目录选择，保留完整来源前缀 |

点击“检查并继续”，选择模型并保存。目录没有显示时，在“高级设置”把模型目录路径设为 `/v1/models`，也可以手动填写刚查到的完整 ID。如果已经有受支持的 Agent 配置指向这个网关，可先用“扫描本机”发现并复用接入。

<figure class="guide-shot-models">
  <a href="/guide-assets/agent-guide-qoder-models-zh.webp"><img src="/guide-assets/agent-guide-qoder-models-zh.webp" alt="通过 Magpie 发现并接入的 Qoder 模型列表" width="1200" height="900" loading="lazy" /></a>
  <figcaption>保存后，Qoder 模型会出现在通用模型列表，下一步即可配置分流。</figcaption>
</figure>

<h2 id="configure-routing">2. 配置智能路由</h2>

1. 打开“**智能路由 → 新建智能路由**”，命名为“Qoder 日常开发”，选择 **“按难度分流”**。
2. 在“**常规模型**”组点击“添加模型”，选择第一步接入的快速或经济模型，处理读文件、找定义、整理资料等常规工作。
3. 在“**主力模型**”组添加适合复杂分析、方案判断或多文件修改的模型。两组可以使用不同订阅、API 或本地模型，按各自账号可用的模型组合。
4. “判断方式”先选 **“启发式规则”**，然后点击 **“启用”**。发布后，这条路由就能在下一步的 Agent 接入设置里选到。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-zh.webp"><img src="/guide-assets/agent-guide-difficulty-zh.webp" alt="HiRoute 按难度分流配置中的判断方式、常规模型组和主力模型组" width="1200" height="900" loading="lazy" /></a>
  <figcaption>常规组负责日常步骤，主力组处理复杂轮次。图中的模型可替换为你第一步接入的模型。</figcaption>
</figure>

想根据执行表现进一步调整时，可在同一位置选择决策模型，使用阶段评分和低分保护。[了解难度路由的判断方式](/zh/docs/model-routing/#difficulty-routing)

<h2 id="use-in-qoder">3. 接入 Agent</h2>

有自定义模型入口时[接入 Qoder CLI](#qoder-cli)。也可以直接[用 Codex Desktop 打开这条路由](#codex-desktop)。

<h3 id="qoder-cli">在 Qoder CLI 使用</h3>

使用支持自定义模型 / BYOK 的 Qoder CLI 时，先完成 Qoder 原生登录。在 HiRoute 打开“Agent → Qoder → 模型路由”，勾选“Qoder 日常开发”并保存。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-qoder-zh.webp"><img src="/guide-assets/agent-guide-qoder-zh.webp" alt="为 Qoder CLI 勾选已发布的 HiRoute 路由" width="1200" height="900" loading="lazy" /></a>
  <figcaption>把刚创建的智能路由加入 Qoder CLI 的模型列表。</figcaption>
</figure>

重启 Qoder CLI，输入 `/model` 选择路由，再开始一个任务。到 HiRoute“会话 → 路由时间线”查看分流结果。

<h3 id="codex-desktop">Qoder 没有 BYOK 入口时，用 Codex Desktop</h3>

如果你使用的 Qoder 版本或套餐没有自定义模型 / BYOK 入口，可以让 **Codex Desktop 使用前面接入的 Qoder 模型和智能路由**。这里选择“Qoder 日常开发”计划即可。

1. 在 macOS 或 Windows 上安装 ChatGPT / Codex 桌面应用，然后打开 **HiRoute 桌面应用 → Agent → Codex Desktop**。
2. 点击“选择路由并启用”。已有接入时，点“模型路由”右侧的“调整”。勾选刚发布的计划。只有一条路由时会自动作为默认；选择多条时，在“默认选择”中选定常用路由。点击“启用”或“保存配置”。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-codex-desktop-zh.webp"><img src="/guide-assets/agent-guide-codex-desktop-zh.webp" alt="Codex Desktop 模型路由对话框中勾选日常开发路由" width="1200" height="900" loading="lazy" /></a>
  <figcaption>勾选刚创建的路由；只有一条时，独立窗口会默认使用它。</figcaption>
</figure>

3. 回到 Codex Desktop 详情，在“独立窗口”区域点击 **“启动独立窗口”**。HiRoute 会打开使用这套路由的独立桌面窗口。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-codex-desktop-launch-zh.webp"><img src="/guide-assets/agent-guide-codex-desktop-launch-zh.webp" alt="HiRoute 的 Codex Desktop 详情页，显示启动独立窗口按钮" width="1200" height="900" loading="lazy" /></a>
  <figcaption>保存路由后，点击右上方的“启动独立窗口”，在新窗口中开始工作。</figcaption>
</figure>

4. 在新窗口选择项目，点击“新聊天”，在输入框右下角确认或选择刚创建的路由。再发一条任务，例如“找出这个项目的启动入口并说明调用关系”。到 HiRoute“会话 → 路由时间线”查看实际选用的模型。

<figure class="guide-shot-window">
  <a href="/guide-assets/agent-guide-codex-desktop-window-zh.webp"><img src="/guide-assets/agent-guide-codex-desktop-window-zh.webp" alt="Codex Desktop 独立窗口中的新聊天，右下角已选择 Qoder Responses 路由" width="2560" height="1640" loading="lazy" /></a>
  <figcaption>在独立窗口的右下角确认路由。图中选中的是“Qoder Responses”，使用时选择你刚创建的路由。点击截图查看原图。</figcaption>
</figure>

更新路由设置后，退出这个独立实例，再用同一按钮重新启动。

<h2 id="task-delegation">4. 了解任务委派</h2>

准备给一个模块加功能时，可以先让轻量的 Pi 找入口、调用方和相关测试。给 Pi 配一个经济模型来做多轮搜索，Qoder 主会话只需读一份带文件位置的索引，就能直接开始设计。

创建“Pi 模块索引”固定模型路由，在“更多设置 → 任务委派”中开启委派并选 Pi，写清用途并启用。用 Qoder CLI 时，在 HiRoute 的“Qoder”条目开启“任务路由”。使用上面的独立 Desktop 接入时，可从 [Codex CLI](/zh/docs/codex-model-routing/#codex-cli) 发起委派。

> 用 HiRoute 把当前模块的梳理交给“Pi 模块索引”。列出入口、直接调用方和对应测试，附文件路径。

先用一个模块试一下，再比较主会话与委派任务的用量。[配置 Pi 轻量委派](/zh/docs/task-routing/#lightweight-pi)
