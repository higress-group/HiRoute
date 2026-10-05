# Why the HTTPX handoff mattered / HTTPX 接力发生时的实际进展

This note explains the original `unattended-httpx-formal-r02` run. It does not
introduce a new experiment, change a score, or regrade the archived implementation.
The [frozen assessment](results/2026-10-04/assessment.json) and
[request/tool timeline](results/2026-10-04/evidence.json) remain authoritative.

## Complete routing sequence

Grouping the 40 recorded requests by consecutive executed model gives four stages
and **three actual model changes**. These are two upgrades to Astra and one return
to Qwen, within the same uninterrupted native task.

| Requests (one-based) | Model | Recorded work |
| --- | --- | --- |
| 1–11 | Qwen | Source/test investigation and small inspection probes |
| 12 | Astra | First context summary; tools absent from this request |
| 13–26 | Qwen | Continued investigation, including the second context summary |
| 27–40 | Astra | Implementation, testing, repair and final response; 13 actual tool calls |

The five decisions in the frozen evidence explain both the changes and the choice
to keep a model. Timestamps below are UTC and identify completed decisions.

| Time | Preceding-stage score | Recorded choice | Effect |
| --- | --- | --- | --- |
| 21:10:45.414 | None | `economy_eligible` | Start with Qwen |
| 21:21:04.720 | 0.41 | `competence_guard` | Qwen → Astra, for the first summary |
| 21:22:19.272 | 0.775 | `economy_eligible` | Astra → Qwen, after that summary |
| 21:34:28.843 | 0.615 | `economy_eligible` | Keep Qwen for the next summary request |
| 21:35:12.593 | 0.485 | `competence_guard` | Qwen → Astra, with tools for continued execution |

The two native compactions completed at 21:22:18.318 and 21:35:11.774. Summary
requests occur before completion; resumed execution follows it. Therefore three
model changes must not be described as three distinct native compactions. A
reassessment also need not change the model, as the fourth decision demonstrates.

The first upgrade applied the competence guard to the next available work, a context
summary. The return to Qwen followed the configured economy-first rule once the
summary stage passed the floor. **0.775 assesses Astra's summary stage, not Qwen's
ability to implement the remaining feature.** It explains why the policy allowed
the return; it does not establish that the return was optimal or that the remaining
task had become easy. Subsequent investigation still did not produce a product
patch, and the later guard decision brought Astra back for implementation.

All five decisions retain their recorded partial-history flags. The public article
describes this as adaptive execution with continued feedback, not as proof that
every intermediate decision was optimal. The final implementation and independent
acceptance remain the strongest evidence of delivery progress.

## Before the second context handoff

Qwen's completed tool calls were source/test reads and small inspection probes.
They covered `httpx/_models.py`, `httpx/_decoders.py`, JSON stream tests, Python's
JSON encoding detection, MIME charset handling, and existing iterator interfaces.
Some files and requirements were read again after the first context compression.
There was no product-code patch in those calls. The first command after the second
handoff started with `git status --short`, whose output was empty before the
following file reads. This supports the observation that execution had not yet
moved into implementation; it does not establish that Qwen could never finish.

The second native compression occurred at `2026-10-03T21:35:11.774Z` (UTC).
Decision `ec5c889a03f54992b09ab15d93eb6916`, completed at timestamp
`1791063312593` ms, recorded:

| Field | Value |
| --- | --- |
| Initial branch choice | `smart_saving_simple` |
| Simplicity criterion | passed |
| Stage competence | `0.485` |
| Competence floor | `0.5` |
| Competence criterion | failed |
| Final branch / reason | `smart_saving_complex` / `competence_guard` |
| History partial | `true` |

The guard changed an otherwise economy-eligible choice. The record does not contain
a free-text explanation tying the score to a particular command. Describing the
lack of implementation as a reason this upgrade was sensible is an interpretation
of the execution trace, not a fabricated quotation from the assessor. The score
uses the available preceding-stage evidence, with its recorded partial-history flag.

## After the handoff

The first Astra tool call followed at `21:35:18.991Z`. Astra checked the working
tree and interfaces, then added `httpx/_json.py` and updated `httpx/_models.py`.
The original JSON-stream feature suite passed 108 checks. Further validation found
asynchronous iterator-closure issues; later patches and tests addressed those,
encoding boundaries and test typing. The trace ends with its selected validation
passing 487 checks and three predeclared transport exclusions. That is the native
agent's validation, separate from the independent evaluation below.

The handoff was followed by 13 actual Astra tool calls. Independent evaluation
accepted 108 feature checks, 229 existing regressions and six incrementality
assertions. The initial instruction was followed by zero intermediate operator
prompts and zero operator product patches. Native elapsed time was 1,896.015 seconds.
An earlier Astra summary stage's 0.775 score is not a score for this final repair.

## 中文摘要

完整请求顺序为 Qwen 11 次 → Astra 1 次 → Qwen 14 次 → Astra 14 次，
即三次实际模型切换：两次升级、一次回切。首次 Astra 请求用于上下文摘要，
没有工具；摘要阶段评分 0.775 后，经济优先策略允许 Qwen 接续。
期间另一次评分 0.615 的决策保持 Qwen，说明重新判断并不必然切换。
0.775 是 Astra 摘要阶段的评分，不能当作 Qwen 后续实现能力的评分，
也不能据此断言回切最优。整个任务有两次原生上下文压缩，不能把三次换模写成三次压缩。

第二次上下文交接前，工具轨迹主要是读源码、读测试和探查编码行为；尚无产品代码补丁。
Astra 接手后的首次 `git status --short` 也没有列出改动。随后它才写入流式解析实现，
并处理异步迭代器关闭、编码边界与类型检查问题。

该交接点的初始选择仍允许经济分支，但阶段胜任度 0.485 低于 0.5 下限，
胜任度保护将最终选择改为主力分支。评分依据的是当时可见的部分历史；
“执行迟迟未进入实现，所以此时升级合理”是结合轨迹做出的解释，不是评分服务的原话。
之后发生了 13 次 Astra 工具调用，最终独立验收 343 项全部通过，全程没有中途人工提示或人工代码补丁。

## Trace provenance

The original private native tool log is already identified by the frozen assessment:
`evidence/unattended-httpx-formal-r02/stdout.jsonl`, SHA-256
`4ca0b69a69a95136468f905312a786c35b8e61a2528e1ea909fc46dd306d66d5`.
The working-tree check is at one-based JSONL line 109, the first implementation
patch at line 114, and subsequent feature-test completion at line 116. Later
completed tool records at lines 121, 123, 126, 128 and 130 show testing and repair.
This note publishes only command roles, public project paths and outcomes; it does
not publish account state, raw private model reasoning or unrelated workstation data.
