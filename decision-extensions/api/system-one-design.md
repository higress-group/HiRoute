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

## OpenAI Decisions and API connection reuse

The current native connection explicitly selects either `system_one` or
`openai_decisions`. Provider presets are suggestions, not runtime protocol
detection. OpenAI Decisions uses `https://api.openai.com/v1/decisions`, an editable
model ID and the same frozen logical questions as System One. Its wire request
contains shared `input_json` with `{state: ...}` and an array of questions with
stable names, original instructions, `choices` or ordered `levels`. Responses bind
by question name and answer value, never array order. Raw competence score is still
divided by two; difficulty and applicability still consume validated probability
vectors. Refusal or invalid output on one question preserves valid independent
answers where the selected path permits it. See the
[OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions).

A native decision model can reuse an existing Native API connection or keep
independent credentials. Reuse selects the source that already owns the Keys;
it does not copy a Key into another secret. The decision model has its own full
endpoint, model ID, protocol and timeout. Its saved target becomes an authorized
destination of the source through the same recoverable save. Key rotation and
disablement affect subsequent calls without republishing; changing the decision
configuration requires selecting and publishing its new version. OpenRouter and
both Bailian access types can use this mode. TypeSafe and compatible endpoints
remain configurable independently. Source availability is local configuration
state, not proof of provider entitlement; an explicit saved-version test checks
connectivity. Custom extensions keep their own independent authentication.

V2 service records carry `hiroute.decision-service/v2`; supported V1 records and
published C v1/v2 question bundles keep their original bytes and meaning. The
service version, wire codec and logical C protocol identify different boundaries.
An OpenAI codec does not change prompts, routing thresholds, history partition or
continuous scoring. Both codecs enforce the actual encoded request-byte budget
without materializing a large history twice merely to translate envelopes.

## 三步适配，不增加执行框架

1. **编译**：验证计划，构建当前 decision 定义和冻结的 assessment_target；分配本请求内的 opaque question ID，并保留 ID 到语义位置的局部绑定表。
2. **调用**：使用既有 classification deadline、Replay、认证和取消逻辑提交一次请求。state 按冻结协议提供当前输入、历史分区与覆盖事实；不用第二次请求才能知道的内容出题。
3. **归约**：校验本次需要的 answers，映射回 v1 的 decision/assessment；HiRoute 再执行阈值和候选政策。绑定表不持久化，不让供应商返回模型/执行身份。

| 决策协议语义 | System One question | 使用的 answer |
| --- | --- | --- |
| `categorical` | `choice`，criteria 为 ID → 类别条件 | `.choice`，必须在允许集合内 |
| `ordinal` | `score`，criteria 为按 level 顺序排列的程度条件 | `.probabilities["0".."n-1"]` 映射到原 level ID |
| `assessment_target` | 独立 `score`，criteria 为 0 / 0.5 / 1 三个胜任锚点的条件 | 有限 `.score ∈ [0,2]` 除以 2 |
| `subset`（未来） | 每个 item 一个 `noul` | `.noul` 为是的概率，再由未来消费方阈值归约 |

供应商 question ID 仅用于关联，不应承载提示词。完整问题必须写在 instructions 中。同一次调用的问题彼此独立，不可写“根据上一问题选中的分支判断难度”。本设计一次提交类别问题，以及每个有双档配置分支的假设性程度问题，最后只读取选中类别的程度答案。[TypeSafe primitives](https://docs.typesafe.ai/primitives)。

类别分类不需要消费方配置置信度阈值。程度判断不能把 `.confidence`、`.score` 或 `1-score` 当成通用简单概率；多档量表的均值会丢失分布。胜任评分则明确采用三级等距锚点，因此使用供应商原始 score/2；不能从已舍入的 probabilities 重新计算。两处采用 Score，但消费字段和含义不同。

类别 Choice 的 instructions 必须包含按主要意图处理重叠条件、无匹配时选择计划默认类别的规则和准确 ID；不要求供应商额外返回置信阈值或无匹配标记。当前分区协议按实际阶段边界分配历史，早期冻结执行保留自己的 assessment_from；局部绑定仍指向原实际阶段；裁到评分目标则标记 partial，目标全部移除则不提交评分问题。

## Current native requests and frozen publications

Historical problem applicability is the sole authoring behavior for new built-in System One publications. There is no protocol switch, CLI editor field or background setting. Background retains at most eight complete turns. Existing frozen publications keep executing unchanged until the next publication. Default assessments then follow the current default silently; custom instructions and every criterion remain byte-for-byte unchanged. Unavailable state paths fail Preview with their scope and field. Local rules and custom-extension v1 retain their behavior.

New native publications use the internal frozen identity `hiroute.system-one-c/v2`; supported v1 publications keep their original identity and fixed content. A publication contains actual fixed questions, complete effective assessments, thresholds, the eight-turn background bound and a content digest. The [v1](../../crates/domain/src/routing/fixtures/system_one_c_v1.json) and [v2](../../crates/domain/src/routing/fixtures/system_one_c_v2.json) Domain bundles each define their exact fixed questions. Do not append a suffix to the legacy assessment wrapper. Editor callers do not supply fixed prompts or digests.

State separates `prior_context`, `assessment_stage`, `latest_user` and `history_partial`. Background is a continuous suffix before the actual assessed stage. A complete target stage is not limited to eight turns. Current input is retained separately; tool-result bodies, goal parsing and semantic extraction are excluded. The serialized request limit is 262144 bytes. Budget the applicability question before admitting the complete target, then allocate the target and background. A partially retained target omits applicability without refilling history using the freed bytes. Oversized current input fails explicitly.

Difficulty routing asks current degree and, for a complete attributable target, independent historical assessment and applicability questions. Branch routing adds a category Choice and a degree question for every two-group category. With sixteen such categories and a complete historical target, one request contains nineteen questions: one category, sixteen degree questions, one assessment and one applicability question. Questions cannot depend on another answer; reduction consumes only the selected category's degree.

Applicability requires exactly `applicable` and `not_established`, with finite probabilities in 0..1 summing to one within 1e-6. The choice string cannot substitute for probabilities. A malformed applicability answer preserves valid category, selected degree and assessment results. Existing version, attribution, category, completeness and frozen-rubric eligibility checks still apply first. When simple probability reaches its threshold and an eligible score is below the competence floor, applicability below 0.5 suppresses the existing protection. A value at least 0.5 or an unavailable result conservatively retains it. Complex tasks independently select the primary group. Default thresholds remain 0.8 / 0.5; same-turn inheritance and availability relay keep their behavior.

## Connection test and maximum-shape diagnostic

The connection page has one explicit saved-version test. Current V2 calls bind the
saved ID/revision and configuration digest; results also identify the credential
state admitted for the test. Supported `hiroute.classifier-decision-test/v1`
callers retain their exact request/result contract. The fixed example uses the
selected wire codec and current logical request frame. There is no route-editor
test. Save, inspect and publish do not call a provider. A changed credential state
makes a previous success stale, without starting an automatic test.

CLI and acceptance tooling can explicitly invoke the separately named [historical problem applicability diagnostic](../../crates/application-api/src/classifier_diagnostic_c.rs), `hiroute.historical-applicability-test/v1`, through the existing protected test operation. It binds an exact saved service revision, endpoint/model, frozen content and `standard` / `max_branches` shape, using production construction, transport and parsing to verify raw Score and complete probability vectors. The maximum shape submits all nineteen questions once, without splitting, dropping questions or inferring capacity from a smaller success. Results describe that shape and connection only, without semantic accuracy or continuous-health claims. Explicit calls may consume quota.

## Earlier frozen-publication execution support

Supported earlier native publications retain their frozen `visible_conversation`, `assessment_from`, independent legacy category template and assessment wrapper. Reads do not rewrite content or digests. Startup validation of retained Operations uses the protocol already sealed in that Operation to validate its original editor; current authoring defaults must not reinterpret it or replay completed effects. The compatibility register records the owner and removal condition. Current editor input cannot select this legacy behavior.

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
- provider 量表长度和问题数上限分别验证。百炼官方同一页对 Score 上限描述不一致，本期只依赖两级程度与三级评分；不推断对任意档位的支持。现有原生协议的 16 个双档分支有 17 个判断问题，加评分为 18 个；C 加上适用性问题最多 19 个。两种形状均需要实测延迟/输入容量，不把官方建议数量误写成硬限制或性能保证。
- 使用决策模型实际上下文能力，沿用有界历史准备，只裁完整旧轮次；当前用户输入不能暗中截短。业务模型/实验的 272k 上下文配置不自动等于决策 provider 的能力，不能为了测试另加任意更小限制。
- 自定义协议严格闭合；供应商 envelope 中的 model、usage、legend、confidence 等是其元数据，只投影已验证的必要答案，不能当作自定义协议字段直接透传。
- 若供应商实测不能提供所需 typed 结果，报告能力不满足。只有证据表明协议不同才增加一个小适配点，不预建多 provider 框架。


真实 C 验收需先冻结准确保存的连接版本、供应商/模型、冻结协议及评分摘要、T=0.8/F=0.5、三问与十九问形状和实际接纳的上下文大小。分别记录完整有效结果、供应商失败、输入用量、延迟、费用及配对的第三问开销。质量比较复用独立审阅的同一组中英文输入，区分确定与有争议标签，并把产品 T=0.8 与研究 T=0.5 分开报告。不得用本地确定性端点的成功替代供应商容量或质量结论；真实调用仍需后续明确授权。
