# 用 Magpie headless 为 HiRoute 接入模型

在后台运行 Magpie，就能把 [Qoder](/zh/docs/qoder-model-routing/) 或 [WorkBuddy](/zh/docs/workbuddy-model-routing/) 等账号中的可用模型接入 HiRoute。Magpie 负责登录和提供本地模型 API；接入后，在 HiRoute 选择模型组、配置难度分流和绑定执行 Agent。

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/magpie-model-sources/
请帮我把已登录的 Magpie 模型接入 HiRoute。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
- 查找并复用已有接入：hiroute compute list · compute scan · compute connection options
- 测试与保存 API：compute connection test · compute connection preview / apply
- 查看模型并配置路由：routing options · routing preview / apply
界面和 CLI 共用配置和记录。由我在本机完成登录与凭据输入；参数按 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 确认。
```

</details>
</aside>

<h2 id="start-magpie">1. 启动后台网关</h2>

按 [Magpie 安装说明](https://usemagpie.ai/docs/start) 安装 CLI，把它和 HiRoute 服务放在同一台机器上。在 macOS 或有 systemd 的 Linux 上，安装并检查用户服务：

```sh
magpie daemon install --no-web
magpie daemon status
```

服务装好但还没启动时，运行 `magpie daemon start`。临时试用可以在终端运行 `magpie serve`。默认地址是 `http://127.0.0.1:3425`，以状态输出为准。从具体 Agent 教程跳到本节的读者，可以回到原教程完成账号登录。

<h2 id="model-catalog">2. 查询模型目录</h2>

完成模型来源的账号授权后，用下面的命令从本地网关读取模型 ID：

```sh
curl -fsS http://127.0.0.1:3425/v1/models \
  -H 'Authorization: Bearer magpie'
```

如果改过监听地址或网关 Key，使用实际值。模型 ID 保留来源前缀，例如 `qoder/dfmodel`；选账号目录中出现的模型。更多来源见 [Magpie 社区插件目录](https://github.com/magpie-community/plugins/blob/main/registry.json)。

<h2 id="save-api">3. 在 HiRoute 保存 Magpie API</h2>

打开“模型 → 通用模型 → 添加模型 → 新建 API 接入”，选择“自定义 API”：

| 项目 | 填写方式 |
| --- | --- |
| 连接名称 | 例如“Magpie 本机” |
| API 地址 | `http://127.0.0.1:3425/v1`，使用你的实际地址 |
| 接口协议 | 按可用端点选择。本文的 Pi/Codex CLI 示例从 Responses 开始；若另有 Chat Completions 用法，可为同一地址单独保存一个接入 |
| API Key | 默认本机网关填 `magpie`；启用自定义 Key 时填实际 Key |
| 模型 | 检查连接后从返回目录选择，或填入目录里的完整模型 ID |

若目录未显示，在“高级设置”中填写模型目录路径 `/v1/models` 后重新检查；也可以用“手动编辑模型”填入已经查到的 ID。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-magpie-api-zh.webp"><img src="/guide-assets/agent-guide-magpie-api-zh.webp" alt="HiRoute 自定义 API 表单中的 Magpie 本机地址与 Responses 协议" width="1200" height="900" loading="lazy" /></a>
  <figcaption>把 Magpie 添加为模型接入，选好实际可用的协议和模型后保存。</figcaption>
</figure>

如果 Codex 或 Hermes 等受支持的配置已指向同一个 Magpie 网关，也可以先试“扫描本机”，再检查并导入匹配的接入。没有扫描结果时，用上面的 API 入口即可。通过远程 Web UI 配置时，`127.0.0.1` 指的是 HiRoute 服务所在主机。

接下来到 [智能模型路由](/zh/docs/model-routing/) 配置常规组和主力组，或回到 [按 Agent 接入](/zh/docs/agent-integrations/) 选择执行路径。
