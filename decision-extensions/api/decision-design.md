# 决策协议 v1：类别、程度与执行评估

本期实现范围仅为模型路由；工具精选只在本文保留协议设计，不开放配置、调用或产品入口。实现入口与契约测试见[决策机制代码地图](../../docs/code-map/decision-foundation.md)。

当前代码与 [OpenAPI](decision.openapi.json)、[接入说明](README.zh-CN.md)使用本文结构。开发中的 **v1** 已原位替换 Gateway、官方扩展、Desktop 示例与唯一 OpenAPI，继续采用 `POST /v1/decisions`。不新增协议版本、版本协商、旧字段读取器或双协议路径；HiRoute 仍请求用户配置的完整 endpoint，不自行追加路径。

路由、发布快照和观测使用新结构与全新开发/验收数据，不迁移、兼容读取或恢复旧数据。旧结构、编解码和只验证旧行为的测试随实现替换，不为本次改造新增兼容注册项。开发与验收建立独立的新数据环境。

## 1. 三个独立问题

| 问题 | 数学含义 | 例子 | 谁使用结果 |
| --- | --- | --- | --- |
| 当前做什么 | 无序类别 `categorical` | 写稿、审稿 | HiRoute 选择任务分支 |
| 当前需要什么程度 | 有序量表 `ordinal` | 简单、复杂；未来也可为其他程度、多档 | HiRoute 按计划阈值选择该类别内的模型组 |
| 上一阶段做得如何 | 绑定历史目标的 `assessment` | 胜任度 0.35 | HiRoute 记录实际表现，并决定本次是否保护性使用主力组 |

智能省钱没有任务类别选择，只提交一个 `ordinal`。自定义分支提交一个 `categorical`，每个选项可附一个 `ordinal refinement`。分支条件描述任务类别，程度条件描述该类别内的难易，胜任标准评价已发生的工作；三者不能共用同一个提示词。

自定义分类每次只选择一个任务类别。多个条件重叠时按当前用户的主要意图选择；均不匹配时选择计划配置的默认分支。HiRoute 将这一规则及默认分支 ID 编入 `categorical.instructions`，不增加优先级、类别置信度阈值或多分支并行执行。无匹配仍是一次有效分类，继续使用默认分支的本次程度结果；不要与调用失败后直接选择主力组混淆。

`ordinal.levels` 的数组顺序表示从低到高的全序，ID 没有数值含义，也不假设等级等距。输出完整概率分布，不返回含义模糊的 `complexity` 或加权平均档位。未来消费方可据此计算累计概率；本期模型路由仅使用两档的 `P(simple)`。这不是模型成功率或已校准的质量保证。

`subset` 表示允许集合的子集，用于未来工具精选。它既不是一个类别，也不是程度序列。本期只记录其契约，不实现。

## 2. 请求与结果

顶层请求恰有以下五个字段；不再并存 `branches` / `assessment_from`：

| 字段 | 约束 |
| --- | --- |
| `decision` | 下述定义对象；模型路由支持 `ordinal` 或 `categorical` |
| `latest_user` | 完整、非空的当前用户 content parts；文本从 Replay 恢复 |
| `visible_conversation` | 原顺序保留的已封存执行轮次，见下文 |
| `history_partial` | HiRoute 已知的历史缺口；扩展不得把缺口当作完整证据 |
| `assessment_target` | `null`，或 `{from, instructions, criteria}`；只描述一个可归属的历史阶段 |

定义对象的闭合字段：

`instructions` 和 `criterion` 为非空文本，不规定逐字段字符上限。名称、ID 和结果的结构约束独立保留。HTTP/运行时的整体请求预算及供应商上下文限制由各自传输和适配层处理，不进入通用决策定义，也不静默截断用户条件或评分标准。

- `ordinal`：`{kind, instructions, levels}`；`levels` 至少两项，每项 `{id, criterion}`，ID 在本量表中唯一，条件非空。协议表达多档，本期路由编译器只生成简单/复杂两档；供应商限制另行验证。
- `categorical`：`{kind, instructions, options}`；每项 `{id, criterion, refinement?}`，选项 ID 唯一。本期自定义分支为 2–16 项，`refinement` 只能是一个 `ordinal`，不能继续嵌套分类。无主力模型的分支不附 refinement。此处不引入任意递归决策树或工作流引擎。
- `subset`（未来）：`{kind, instructions, items}`，每项 `{id, description}`，ID 唯一。允许返回空集；业务约束由消费方执行。

HTTP 200 响应为 `{decision, assessment?}`。`decision` 依定义分别为：

```json
{"kind":"ordinal","probabilities":{"simple":0.93,"complex":0.07}}
```

```json
{"kind":"categorical","choice":"review","refinement":{"kind":"ordinal","probabilities":{"simple":0.28,"complex":0.72}}}
```

```json
{"kind":"subset","selected":["search_repository","read_file"]}
```

校验规则：

1. `kind` 与请求一致；choice 必须是本次允许 ID；refinement 只对应所选选项，存在与否也与定义一致。不能返回模型 ID、候选顺序或未允许能力。
2. 概率必须覆盖所有 level ID，既不缺项也不多项，每项为有限的 `[0,1]`。接受 `abs(sum-1) <= 1e-6` 的浮点误差并除以总和；更大误差无效，不静默补值或伪造概率。供应商确有更粗舍入时，应凭实测证据调整其适配边界和测试，不放松通用契约。
3. subset 的 ID 必须来自 items，去重前有重复即无效；顺序没有优先级含义。它不返回工具参数，也不授权执行。
4. JSON 对象拒绝重复字段、未知字段、Markdown 与原生供应商 envelope。响应上限沿用 64 KiB。错误定位到当前决策、选中 refinement 或可选 assessment；未被选中的供应商问题无效，不应连带废弃已验证的选中路径。
5. assessment 无效不废弃合法当前决策。类别非法/整体决策失败时不用该响应驱动评分或升级；类别合法但程度失败时，可保留独立且有效的历史评分，并在所选类别内兜底。

### 历史和评分目标

历史轮次为 `{user, status, steps}`，复用现有 content parts、轮次顺序和状态含义，不对外暴露 `branch_id` / `executed_branch_id`。执行类别、档位、精确模型/profile、阶段 ID、计划版本和标准摘要由 HiRoute 在请求快照内绑定，不交给扩展猜测或回传。任务内容及目标评分标准足以解释评分对象，不需要暴露内部模型身份。

`assessment_target.from` 是 `visible_conversation` 中零基下标，范围为 `[0, len-1]`；从该下标到末尾构成**同一个**可评分执行阶段。HiRoute 必须先确认该后缀可归属单一实际执行身份和兼容的发布标准，否则传 null。`instructions` 与 `criteria` 是该目标执行时冻结的有效评分提示词，包含分支覆盖；不是本轮新选分支的标准。`criteria` 本期固定三项 `{score, criterion}`，score 按顺序为 `0, 0.5, 1`。所有字段必填。

响应 assessment 为 `{score, partial, reason?}`：score 为有限 `[0,1]`，partial 必填，reason 可选且 1–1024 个 Unicode 标量。目标为 null 时不应返回 assessment；若返回，只丢弃这个无请求目标的评分。目标后缀被服务裁剪则 partial 为 true；目标全被移除则省略评分。HiRoute 合并自身缺口标记，服务的 false 不能抵消它。有效零分必须保留，缺失/非法/部分评分不是零分。

`history_partial` 描述整段可见历史的缺口；评分的 `partial` 只判断 `assessment_target.from` 起的目标阶段证据是否缺失。接入 HiRoute 前的更早消息未采集，不应把随后完整采集的新阶段永久标成部分证据；当前阶段内已发生但未采集的回答、裁剪或中断仍须标记为部分证据。

历史仅含用户内容、已接受回答、工具名称/顺序/显式粗粒度状态、不可用内容标记。仍不发送 system/developer prompt、工具参数或结果正文、凭据。重复用户文本、继续或摘要本身不算负面反馈。省略评分不删除已保存的观测，但旧观测不能再作为本次新得低分使用。

## 3. HiRoute 的两档执行政策

政策属于路由消费方，不属于 `kind`，不在协议内重复传模型组、简单阈值或胜任阈值。扩展负责实现判断与评分；HiRoute 负责定义允许选项、有效提示词、阈值、执行候选与发布快照。自定义扩展也必须使用传入的有效评分标准；不能一边在产品中承诺可编辑、一边由服务固定另一套标准。

对本次选中的类别，记简单概率为 `p`，简单阈值 `t`（默认 0.8），胜任阈值 `c`（默认 0.5）。两档计划的选择如下：

| 当前条件 | 本次执行组 | 观测原因 |
| --- | --- | --- |
| `p >= t` 且没有本次适用的低分 | 省钱/常规 | 当前任务适合常规组 |
| `p < t` | 主力 | 简单概率未达阈值 |
| `p >= t` 且本次有效胜任度 `< c` | 主力 | 上一阶段不胜任 |
| 类别有效，程度结果缺失/非法 | 该类别的主力 | 程度判断失败；概率显示不可用 |
| 仅一个模型组 | 常规 | 不判断程度；胜任评分只用于观测 |

“本次适用的低分”同时要求：本次返回、完整有效、可归属上一实际阶段、类别与本次相同、发布版本/标准兼容。来自写稿的低分不能使审稿升级。分数等于阈值算达标。上一阶段使用主力组时，低分可继续保护性选择主力，但不存在第三档。

每条**新用户消息**到达决策边界后重新判定，不维护跨轮单向升级游标。下一次 `p >= t` 且没有适用低分即可回到常规组；不复用旧低分永久锁定。高分本身不证明更便宜的模型胜任，回落仍必须有本次程度判断。工具续接/同轮重放继承冻结决策，不在一轮内 C→A 来回切换。“优先保持当前模型”仅在本次所选组的合格候选中生效，不能覆盖类别或档位。

每个组的有序模型列表是**可用性故障接力**，不是更多胜任档位。选择常规组后，依次尝试本组候选，耗尽后继续同一类别的主力组候选，仍无可用候选则明确失败；单组类别耗尽即失败。直接选择主力组时只在主力组内接力，不降回常规、不跨任务类别。智能省钱以省钱组对应常规组，复用同一规则，不另设故障回退模型列表。

故障接力复用现有能力检查、原生推理配置、状态保真、预算、凭据 lease、超时及提交机制，仅在既有执行边界允许接力时继续，不重放已提交输出。候选故障不产生低分。实际执行类别/组必须单独记录，混合执行不能冒领单模型评分。同一模型/profile 在不同组仍是不同执行位置，但同次接力不重复尝试已经失败的同一执行候选。

整体决策失败时：智能省钱沿用启发式规则；自定义分支使用配置的默认分支，有主力则选其主力，否则常规。新判定、失败兜底、低分保护和候选故障必须分开记录。源取消、源 deadline 与 Replay 完整性失败仍直接终止，不能伪装成决策失败继续请求。传输复用现有一次调用、认证、禁止重定向和总 deadline，不新增后台重试。

## 4. 完整示例

以下都是**说明性数值，不是模型实测结果**。可解析样本与供应商调用放在 [decision-examples.json](decision-examples.json)。

### 智能省钱：直接判断程度

```json
{
  "decision": {
    "kind": "ordinal",
    "instructions": "判断当前任务需要的处理程度，不评价上一阶段表现。",
    "levels": [
      {"id":"simple","criterion":"需求明确、边界清楚，可沿用已有模式完成。"},
      {"id":"complex","criterion":"需要深入推理、跨模块诊断或设计新的方案。"}
    ]
  },
  "latest_user": [{"kind":"text","text":"把这段说明改成三条清晰的要点。"}],
  "visible_conversation": [],
  "history_partial": false,
  "assessment_target": null
}
```

```json
{"decision":{"kind":"ordinal","probabilities":{"simple":0.93,"complex":0.07}}}
```

默认阈值下用省钱组；不把简单/复杂记录成两个任务分支。

### 自定义写稿/审稿：先类别，再该类别的程度

```json
{
  "decision": {
    "kind":"categorical",
    "instructions":"按当前请求的主要意图选择一个工作类别，不按任务难度或上次失败选择。条件重叠时选择最符合主要意图的一项；均不匹配时选择默认类别 writing。",
    "options":[
      {"id":"writing","criterion":"撰写、续写或改写文章，包括根据反馈修稿。",
       "refinement":{"kind":"ordinal","instructions":"假设当前任务交给写稿分支，判断写稿工作所需程度。",
         "levels":[{"id":"simple","criterion":"事实和提纲充分，仅需常规组织或局部改写。"},
                   {"id":"complex","criterion":"需要综合冲突材料、构建论证或重组全文。"}]}},
      {"id":"review","criterion":"审阅、核实文章并提出意见，不直接重写全文。",
       "refinement":{"kind":"ordinal","instructions":"假设当前任务交给审稿分支，判断审稿工作所需程度。",
         "levels":[{"id":"simple","criterion":"范围明确的格式、措辞与已知事实核查。"},
                   {"id":"complex","criterion":"需要逐项核对来源、检查因果推断或全篇论证。"}]}}
    ]
  },
  "latest_user":[{"kind":"text","text":"请逐项核对这篇稿件的来源与因果论断，给出审稿意见。"}],
  "visible_conversation":[
    {"user":[{"kind":"text","text":"根据所给材料写一篇文章。"}],"status":"completed",
     "steps":[[{"kind":"text","text":"文章已写出，但将两个没有因果证据的现象写成了因果关系。"}]]}
  ],
  "history_partial":false,
  "assessment_target":{
    "from":0,
    "instructions":"评价上一写稿阶段的材料忠实性与论证，不评价尚未发生的审稿。",
    "criteria":[
      {"score":0,"criterion":"关键事实失真、无进展或需要大幅纠正。"},
      {"score":0.5,"criterion":"有用但不完整，仍有明显事实或论证问题。"},
      {"score":1,"criterion":"忠实于材料、归因准确、完成写稿要求。"}
    ]
  }
}
```

```json
{
  "decision":{"kind":"categorical","choice":"review","refinement":{"kind":"ordinal","probabilities":{"simple":0.28,"complex":0.72}}},
  "assessment":{"score":0.35,"partial":false,"reason":"上一写稿阶段存在没有证据支撑的因果断言。"}
}
```

本次用审稿主力组，原因是**审稿简单概率不足**；写稿 0.35 只归给上一写稿阶段。即使本次审稿简单概率改为 0.93，写稿低分也不能触发审稿升级。

### 工具精选：未来协议，非本期能力

```json
{
  "decision":{
    "kind":"subset",
    "instructions":"选择为当前任务提供足够能力所需的工具；不要执行工具。",
    "items":[
      {"id":"search_repository","description":"搜索仓库中的代码和文档。"},
      {"id":"read_file","description":"读取指定仓库文件。"},
      {"id":"send_email","description":"向他人发送电子邮件。"}
    ]
  },
  "latest_user":[{"kind":"text","text":"查一下路由阈值在哪里实现，并解释代码。"}],
  "visible_conversation":[],
  "history_partial":false,
  "assessment_target":null
}
```

```json
{"decision":{"kind":"subset","selected":["search_repository","read_file"]}}
```

未来消费方仍须保护显式 `tool_choice`、必要工具及依赖，保留历史调用/结果/ID；非法输出或选择失败保留原允许工具集合。空集是否合法由消费方判断。逐工具独立判断不承诺找到全局最小或最优工具集，也不增加任何调用权限。本期不得把 subset 声明为服务测试或产品支持能力。

## 5. 产品、实现与验收边界

产品用语是“决策模型 / 自定义扩展”，不把 categorical、ordinal、System One 暴露在普通路由表单。配置方式详见[路由指南](../../docs/smart-saving-model-classification.zh-CN.md)，供应商调用详见 [System One 映射](system-one-design.md)。

实现只需复用现有 classification 生命周期，增加定义编译、请求局部绑定表与结果校验/归约；不引入通用插件系统、并行执行工作流或永久决策状态。发布冻结类别条件、程度条件、阈值、目标评分标准、候选/profile 及连接版本。

实现验收必须覆盖：0.8/0.5 边界；同类别低分保护到主力后，新轮次仍重新决策，复杂仍选主力，简单且无本次适用低分才选常规；换类别不继承低分；缺失/部分评分；类别合法但程度失败；条件重叠与无匹配默认分支；单组分支；同模型跨组精确归属；常规故障接力耗尽后到同类别主力、主力耗尽后失败；完整分支设置覆盖与恢复；一轮工具续接不重新决策；评分与触发记录不可被草稿改写；计划和会话观测一致。当前实现的自动化检查与真实供应商验收分别记录，不以示例或旧实现验收代替重做后的产品验收。
