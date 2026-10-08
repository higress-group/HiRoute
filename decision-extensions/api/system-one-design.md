# 内置决策模型：System One 映射

模型路由适配使用一次调用的请求内问题映射，不保留旧分支结构兼容路径。工具精选仅为未来协议示例。本说明与[当前 v1 决策协议](decision-design.md)配套；实现入口与契约测试见[决策机制代码地图](../../docs/code-map/decision-foundation.md)。

## 边界与协议选择

产品连接只配置 provider、完整 endpoint、model、认证及超时。HiRoute 从已发布计划构造判断问题并归约答案；用户不需要维护 Jev 服务或填写完整 API JSON。自定义扩展则自己完成模型集成与决策算法，消费相同的类别、程度和历史评分定义。

当前三个供应商的基础形状为 `model + state + questions → answers`。百炼声明兼容 TypeSafe System One，支持 Choice、Score、Noul；OpenRouter 提供 Jev decisions 入口。复用一个编码器和现有 HTTP 传输即可，不为各 provider 建插件框架。不同供应商的模型、端点、上下文上限和问题数量建议不能当作相同保证，保存后的连接测试与最终真实验收仍需逐一验证。[百炼 API](https://help.aliyun.com/zh/model-studio/decision-model-api)、[OpenRouter Jev](https://openrouter.ai/blog/tutorials/how-to-use-jev/)、[TypeSafe API](https://docs.typesafe.ai/api)。

| 接入 | 完整 endpoint | 默认模型 |
| --- | --- | --- |
| 百炼 Token Plan | `https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/systemone` | `decision-model-preview` |
| 百炼业务空间 | 用户填写包含地域和 WorkspaceId 的完整 System One 地址 | `decision-model-preview` |
| OpenRouter | `https://openrouter.ai/api/alpha/decisions` | `typesafe/jev-1.13` |
| TypeSafe | `https://api.typesafe.ai/v1/systemone` | `jev-latest` |
| 兼容接入 | 用户填写完整地址与模型 | 无预设 |

Token Plan 保持一个入口，不根据个人/团队版做本地资格判断；以用户接入的实际调用结果为准。[Token Plan 官方入口](https://help.aliyun.com/zh/model-studio/token-plan-decision-model)。这些地址与模型名是连接预设，不能成为运行时按名字识别协议的隐式规则。

## 三步适配，不增加执行框架

1. **编译**：验证计划，构建当前 decision 定义和冻结的 assessment_target；分配本请求内的 opaque question ID，并保留 ID 到语义位置的局部绑定表。
2. **调用**：使用既有 classification deadline、Replay、认证和取消逻辑提交一次请求。state 包含完整当前用户输入、已保留历史、缺口事实及评分起点；不用第二次请求才能知道的内容出题。
3. **归约**：校验本次需要的 answers，映射回 v1 的 decision/assessment；HiRoute 再执行阈值和候选政策。绑定表不持久化，不让供应商返回模型/执行身份。

| 决策协议语义 | System One question | 使用的 answer |
| --- | --- | --- |
| `categorical` | `choice`，criteria 为 ID → 类别条件 | `.choice`，必须在允许集合内 |
| `ordinal` | `score`，criteria 为按 level 顺序排列的程度条件 | `.probabilities["0".."n-1"]` 映射到原 level ID |
| `assessment_target` | 独立 `score`，criteria 为 0 / 0.5 / 1 三个胜任锚点的条件 | 有限 `.score ∈ [0,2]` 除以 2 |
| `subset`（未来） | 每个 item 一个 `noul` | `.noul` 为是的概率，再由未来消费方阈值归约 |

供应商 question ID 仅用于关联，不应承载提示词。完整问题必须写在 instructions 中。同一次调用的问题彼此独立，不可写“根据上一问题选中的分支判断难度”。本设计一次提交类别问题，以及每个有双档配置分支的假设性程度问题，最后只读取选中类别的程度答案。[TypeSafe primitives](https://docs.typesafe.ai/primitives)。

类别分类不需要消费方配置置信度阈值。程度判断不能把 `.confidence`、`.score` 或 `1-score` 当成通用简单概率；多档量表的均值会丢失分布。胜任评分则明确采用三级等距锚点，因此使用供应商原始 score/2；不能从已舍入的 probabilities 重新计算。两处采用 Score，但消费字段和含义不同。

类别 Choice 的 instructions 必须包含按主要意图处理重叠条件、无匹配时选择计划默认类别的规则和准确 ID；不要求供应商额外返回置信阈值或无匹配标记。裁剪历史时同步重算 state.assessment_from，局部绑定仍指向原实际阶段；裁到评分目标则标记 partial，目标全部移除则不提交评分问题。

## 智能省钱请求

下列数值及回答均为说明性样本，不是真实调用结果。首次请求只问程度，有可靠上一阶段才附评估。

```json
{
  "model":"decision-model-preview",
  "state":{
    "latest_user":[{"kind":"text","text":"把这段说明改成三条清晰的要点。"}],
    "visible_conversation":[],
    "history_partial":false,
    "assessment_from":null
  },
  "questions":{
    "q0":{
      "type":"score",
      "instructions":"只判断 state.latest_user 所要求的当前任务需要的处理程度，不评价上一阶段表现。",
      "criteria":["需求明确、边界清楚，可沿用已有模式完成。","需要深入推理、跨模块诊断或设计新的方案。"]
    }
  }
}
```

```json
{"answers":{"q0":{"type":"score","score":0.07,"probabilities":{"0":0.93,"1":0.07}}}}
```

局部绑定 `q0 → decision.levels` 得到 `{"simple":0.93,"complex":0.07}`；HiRoute 使用已发布简单阈值 0.8 选择省钱组。精简响应省略了供应商可返回的 legend、confidence、usage 等元数据。

## 写稿 + 审稿请求

以下 state 对应决策请求的上一写稿阶段。评分绑定写稿，不因 q0 选择审稿而变化。两类都有双档时，首轮是三个问题；有评分目标时是四个问题。

```json
{
  "model":"decision-model-preview",
  "state":{
    "latest_user":[{"kind":"text","text":"请逐项核对这篇稿件的来源与因果论断，给出审稿意见。"}],
    "visible_conversation":[
      {"user":[{"kind":"text","text":"根据所给材料写一篇文章。"}],"status":"completed",
       "steps":[[{"kind":"text","text":"文章已写出，但将两个没有因果证据的现象写成了因果关系。"}]]}
    ],
    "history_partial":false,
    "assessment_from":0
  },
  "questions":{
    "q0":{"type":"choice",
      "instructions":"按 state.latest_user 的主要意图选择一个工作类别，不按任务难度或上次失败选择。条件重叠时选择最符合主要意图的一项；均不匹配时选择默认类别 writing。",
      "criteria":{"writing":"撰写、续写或改写文章，包括根据反馈修稿。","review":"审阅、核实文章并提出意见，不直接重写全文。"}},
    "q1":{"type":"score",
      "instructions":"假设 state.latest_user 交给写稿分支，判断写稿工作所需程度。不要读取其他问题的答案。",
      "criteria":["事实和提纲充分，仅需常规组织或局部改写。","需要综合冲突材料、构建论证或重组全文。"]},
    "q2":{"type":"score",
      "instructions":"假设 state.latest_user 交给审稿分支，判断审稿工作所需程度。不要读取其他问题的答案。",
      "criteria":["范围明确的格式、措辞与已知事实核查。","需要逐项核对来源、检查因果推断或全篇论证。"]},
    "q3":{"type":"score",
      "instructions":"从 state.visible_conversation 的 state.assessment_from 下标起评估已执行的写稿阶段。评价材料忠实性与论证，state.latest_user 仅可作为明确反馈证据；不要评价本轮尚未发生的审稿。重复或继续消息本身不是失败。",
      "criteria":["关键事实失真、无进展或需要大幅纠正。","有用但不完整，仍有明显事实或论证问题。","忠实于材料、归因准确、完成写稿要求。"]}
  }
}
```

```json
{
  "answers":{
    "q0":{"type":"choice","choice":"review","probabilities":{"writing":0.02,"review":0.98}},
    "q1":{"type":"score","score":0.15,"probabilities":{"0":0.85,"1":0.15}},
    "q2":{"type":"score","score":0.72,"probabilities":{"0":0.28,"1":0.72}},
    "q3":{"type":"score","score":0.7,"probabilities":{"0":0.4,"1":0.5,"2":0.1}}
  }
}
```

归约取 q0 的 review、q2 的概率、q3 的 `0.7/2=0.35`，忽略 q1。q1 即使缺失或非法也不影响这条选中路径。q2 失败则记录“程度判断失败”，使用审稿主力；q3 失败只丢弃评分。q0 失败则执行计划的整体决策失败政策。没有主力模型的分支不生成对应程度问题。

System One 不产生自由文本解释；内置适配器不伪造 reason。partial 来自 HiRoute 的证据覆盖和本次裁剪事实，不能让一个评分数值替代完整性判断。连接测试使用合成输入验证类别和程度必要字段，不调用业务模型、不生成用户阶段评分，也不测试未来 subset 支持。

## 工具精选的未来调用示例

未来可将 subset 的 items 放在 state.allowed_tools，每项逐一问 Noul。完整可解析请求/响应见 [decision-examples.json](decision-examples.json) 的 `tools_future`；本期不编译或执行该映射。

```json
{
  "model":"decision-model-preview",
  "state":{
    "latest_user":[{"kind":"text","text":"查一下路由阈值在哪里实现，并解释代码。"}],
    "allowed_tools":[
      {"id":"search_repository","description":"搜索仓库中的代码和文档。"},
      {"id":"read_file","description":"读取指定仓库文件。"},
      {"id":"send_email","description":"向他人发送电子邮件。"}
    ]
  },
  "questions":{
    "q0":{"type":"noul","instructions":"为完成 state.latest_user，是否需要保留 state.allowed_tools[0] 的 search_repository 搜索能力？只判断必要性，不执行。"},
    "q1":{"type":"noul","instructions":"为完成 state.latest_user，是否需要保留 state.allowed_tools[1] 的 read_file 文件读取能力？只判断必要性，不执行。"},
    "q2":{"type":"noul","instructions":"为完成 state.latest_user，是否需要保留 state.allowed_tools[2] 的 send_email 发信能力？只判断必要性，不执行。"}
  }
}
```

```json
{"answers":{"q0":{"type":"noul","noul":0.98},"q1":{"type":"noul","noul":0.91},"q2":{"type":"noul","noul":0.02}}}
```

举例以 0.5 保留阈值得到前两项，阈值只是未来消费政策示意。本期不定义工具配置默认值、不承诺单工具独立判断能解出最优子集。

## 供应商验证边界

- 百炼与 OpenRouter 的多轮验证需要覆盖二档程度概率、三级胜任评分、类别与分支程度并问，以及无历史/历史缺口。协议结果可读取不证明判断准确；供应商质量应按具体任务单独评估，旧结构的 Choice+Score 成功不能替代当前协议的验证。
- provider 量表长度和问题数上限分别验证。百炼官方同一页对 Score 上限描述不一致，本期只依赖两级程度与三级评分；不推断对任意档位的支持。16 个双档分支有 17 个判断问题，加评分为 18 个；需要实测延迟/输入容量，不把官方建议数量误写成硬限制或性能保证。
- 使用决策模型实际上下文能力，沿用有界历史准备，只裁完整旧轮次；当前用户输入不能暗中截短。业务模型/实验的 272k 上下文配置不自动等于决策 provider 的能力，不能为了测试另加任意更小限制。
- 自定义协议严格闭合；供应商 envelope 中的 model、usage、legend、confidence 等是其元数据，只投影已验证的必要答案，不能当作自定义协议字段直接透传。
- 若供应商实测不能提供所需 typed 结果，报告能力不满足。只有证据表明协议不同才增加一个小适配点，不预建多 provider 框架。
