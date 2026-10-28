# HiRoute: System One Routing for Long-Running Agents with History-Aware Protection

Community-review manuscript · main research snapshot: 18 October 2026; mathematical supplement: 28 October 2026. We circulate this Markdown version for collaborative review before typesetting and arXiv submission. It keeps the frozen labels and principal result tables, reports the MR010_b sensitivity separately, and distinguishes AI construction from AI-assisted human review. The exact study questions, bilingual datasets, curated responses and an offline reproduction package are described in [the reading guide](README.md).

## Abstract

An agent's next task can be routine while an unresolved execution failure still calls for a stronger model. Once that failure is corrected or stops affecting the work, the reason to escalate disappears. We study this temporal routing problem through a System One decision interface and its open-source implementation in HiRoute. A single request asks three typed questions: what the current task requires, how the assessed stage performed, and whether a historical problem still applies. HiRoute combines the returned probabilities and continuous score with a fixed rule. Our bilingual benchmark evaluates final group selection and its component judgments against explicit task criteria. On 26 engineering workflows, matched three-question configurations reach 97.12% and 96.63% workflow-averaged action accuracy for Jev 1.13 and 1.14. Mathematical-policy comparisons show that the benefit depends on the backend and context. In an independently authored 136-record panel, existing three-question definitions reach 99.22% for Jev 1.14 versus 89.69% for matched direct selection, while Jev 1.13 is near parity. A separately reviewed, targeted 48-record dependency-state panel shows protection gains in both versions; chronological checkpoints show retention with concise definitions but no extra advantage over matched direct selection. Case analysis explains how the composition rule blocks, propagates or cancels judgment errors. Measured Jev three-question charges are about USD 0.084–0.086 per 1,000 requests. Published data, offline replay and a pinned implementation make the mechanism inspectable. AI construction, AI review and AI-assisted human review are documented separately. The evidence concerns declared-policy conformance; downstream task success, realized savings and broader generalization remain open.

## 1. Introduction

Consider an agent asked to edit a supplied configuration value. The path is known and the change is bounded, so the standard tier would normally handle it. Now suppose the previous stage was already asked to make this edit and failed to deliver it. A policy may then call for the enhanced tier, even though the remaining edit is still simple. We call this override **historical protection**. If the user later completes the edit and asks for an unrelated label change, the old failure no longer justifies the override.

**Standard tier** and **enhanced tier** name configurable routing roles. The standard tier handles routine work; the enhanced tier handles greater reasoning demand or an applicable historical-protection override. Both roles are defined relative to the user's task policy and model configuration. The frozen prompts, data and evaluator keep the identifiers `economy` and `primary` for these roles; the public terminology leaves those experimental artifacts unchanged.

HiRoute gives conversation interpretation to a decision model and policy execution to application code. It separates three judgments:

1. **Current demand:** does the pending work follow a settled path, or does it require unresolved reasoning under the configured task standard?
2. **Historical performance:** did the assessed stage deliver, or reliably advance, the work assigned to it?
3. **Present applicability:** does an unresolved problem from that stage still affect the pending work?

In the example, current demand stays routine throughout and historical performance stays poor for the failed stage. Only applicability changes once the edit is complete. This third judgment lets the router release an obsolete reason for escalation while keeping protection for work that is still affected. It concerns visible evidence and dependency; it does not predict whether a particular executor will succeed.

Our focus is the use of System One interfaces for these decisions. We use the term operationally: such a service accepts shared state and typed questions and returns structured categories, probabilities or scores. Jev provides this kind of interface. Its documentation recommends asking bounded questions and composing the outputs in software; we read that as a rationale for the interface design, not as proof that the resulting judgments are accurate or that their errors are independent [1]. We examine observable API behavior and draw no conclusions about the proprietary architecture behind it.

Task difficulty depends on policy. A label such as “simple” means little without a declared scope of work and intended executor roles. The engineering policy studied here separates bounded changes with an established path from unresolved investigation, consequential design choices and intricate interactions, and users can customize it. Our evaluation therefore asks whether the router applies the declared standard consistently as a task progresses, not what difficulty means universally across executor models.

The system contribution lies in connecting this decision interface to an executable temporal routing policy. The three questions share one state and one API request; they are not three sequential model calls. Their numerical outputs show which judgments drive HiRoute's group selection. We compare this format with a direct selector that receives the same facts and the full policy but returns the final group itself. An LLM numerical adapter also answers the three questions, so the choice of backend can be separated from the choice of output format.

Our contributions are:

1. An open-source System One routing mechanism that combines current task demand with applicable historical failure.
2. A policy-conditioned bilingual benchmark and offline evaluator that both generic group selectors and typed decision interfaces can use. Ambiguous and score-boundary cases stay visible instead of being forced into a single action label.
3. Experiments and case traces showing when historical protection helps, when it should be released, and how component errors reach the final action.

The construction and audit method is meant to transfer to other policies, although its simple/complex labels remain tied to the declared task standard and executor roles.

Developers of multi-model agents and gateways will want to know three things: whether the mechanism protects affected work, whether it releases unrelated work, and whether the extra judgments select better than a fully informed direct answer. Sections 5.1–5.4 report these comparisons, Section 5.5 traces the errors behind the aggregate scores, and Section 5.6 examines a disputed choice about which cases can be scored.

## 2. Related work and scope

Preference-based routing methods such as RouteLLM learn to trade execution quality against resource use [3]. Our immediate target is different: faithfully executing an explicit contextual routing policy. The two targets complement each other. A policy can be implemented correctly and still have poor downstream utility, and claims about the latter require an executor-quality study.

Context-guided routing for stateful agents studies history access at user-turn boundaries and the effects of switching executor models [5]. TwinRouterBench separates intermediate-prefix tier evaluation from live task resolution and spend [6]. These studies establish temporal routing and execution-based validation as existing research directions. We contribute to the scope and temporal interpretation of historical protection under an explicit policy, measuring judgments and actions separately; we do not claim to introduce stateful routing or to replace live-execution benchmarks. Arch-Router's use of user-defined preferences further motivates making the routing contract explicit [7].

Our contextual contrasts follow the behavioral-testing principle of probing capabilities through controlled changes rather than relying on an aggregate score [4,8]. We apply that principle to routing checkpoints: the pending request can stay fixed while prior delivery, correction, requirement validity or dependency changes. Translations, state variants and paraphrases are correlated observations, so the workflow is the principal statistical unit.

Typed decision interfaces make intermediate judgments directly observable. A Jev Score returns a distribution over ordered rubric levels together with a continuous scalar [2]. An LLM can also be instructed to emit numerical judgments, but its reported numbers and a native decision distribution arise through different mechanisms, and we presume neither is calibrated. Attributing differences to decomposition or to the backend therefore requires a matched backend-by-format experiment; an obsolete internal protocol is not an adequate principal competitor.

## 3. Method

### 3.1 State and temporal boundaries

A **routing checkpoint** is the moment before the next request is assigned to a model group. A **stage** is the preceding execution segment whose performance is being assessed. At each checkpoint, the input is divided into three regions:

| Region | Content | Evidential role |
|---|---|---|
| `prior_context` | Up to eight complete preceding turns | Earlier requirements, settled facts and references |
| `assessment_stage` | The full stage under assessment, subject to explicit product size/partial handling | Requested stage outcome and visible execution |
| `latest_user` | The pending request, kept separate | Current work and any reported feedback or completed correction |

The method organizes the original records instead of generating a semantic summary. Raw tool-result bodies are not supplied for scoring, and there is no dedicated goal extraction or source-type classifier. An instruction to continue is not in itself negative feedback. A single message can report a correction that is already complete and also request a different action that has not yet been performed; these evidential roles must be kept apart.

The eight-turn background limit is an implementation choice, not an optimum established by the main studies. The independent validation below contains zero or one preceding turn, so it cannot establish robustness over long histories or behavior when relevant requirements fall outside the window.

### 3.2 Three independently specified judgments

**Current demand (q0): what must be done now?** Under the published engineering standard, routine work follows explicit requirements and existing patterns. Complex work requires unresolved diagnosis, a consequential choice among alternatives, or intricate architectural, concurrency, state or algorithmic reasoning. Neither project size nor words such as “fix” decide the answer. History can resolve references such as “that file”, but a past failure does not by itself make the remaining work complex.

**Historical performance (q1): what did the assessed stage achieve?** The ordinal rubric has three anchors: zero for a supported core non-delivery or persistent core failure; one for useful progress with a bounded, unresolved local issue; and two for delivery or reliable advancement without a supported unresolved execution problem. The reference is the work assigned at the time. Recording an unsuccessful research direction can fulfill an investigation request, whereas promising an artifact does not fulfill a request to deliver it. The router relies on the visible record and does not independently verify whether hidden implementation is correct.

**Present applicability (q2): does that problem still matter here?** A historical execution problem applies only if it is still unresolved and has a concrete relationship to the pending work: an affected deliverable, an immediate prerequisite, or a supported common operation or constraint. Belonging to the same project is not enough. A completed correction or a withdrawn requirement can remove applicability, while an instruction or promise to repair does not show that the repair happened.

In short, q1 judges what the stage was responsible for when it executed, and q2 judges the problem's status and relevance now. All three prompts are frozen. Provenance records keep their development identifier, C, which is not the method's public name. The [reading guide](README.md#exact-study-questions) links the exact question files.

### 3.3 Continuous score and deterministic composition

The rule has two reasons to select the enhanced tier: the current work requires reasoning, or an applicable historical problem warrants protection. Let $s$ be the probability that the current work is simple, $h\in[0,2]$ the returned historical performance score, and $a$ the applicability probability. Given an eligible, complete historical assessment and valid answers, the paper configuration is:

$$
\mathrm{enhanced} \iff s < 0.5\;\lor\;(h<1\;\land\;a\geq0.5).
$$

Under these valid-answer conditions, the rule otherwise selects the standard tier. The simple threshold is configurable: this paper fixes it at 0.5, whereas the tested product default is 0.8. HiRoute stores the normalized historical score $h/2$ with a corresponding strict threshold of 0.5, which is algebraically identical rather than a second scoring rule.

Because the historical score is continuous, a most-likely grade of one does not imply a score of exactly one. Probabilities $(0.06,0.93,0.01)$ for grades zero, one and two, for example, have modal grade one, but their weighted score is $0\times0.06+1\times0.93+2\times0.01=0.95$. If the supplied scalar is 0.95, the strict $h<1$ condition activates protection when applicability is high; a supplied score of exactly 1 does not. We keep HiRoute's continuous-score rule, so grade-one cases near this boundary need not have a unique standard-tier target. Routing uses the supplied scalar. The weighted example only explains the distinction, and any discrepancy between scalar and distribution is recorded rather than silently corrected.

A missing or invalid applicability answer preserves historical protection that is otherwise eligible. Invalid, partial or incorrectly attributed historical assessments follow the existing product contracts. We evaluate these operational paths separately from the semantic accuracy of valid answers.

### 3.4 When a component can change the action

The three judgments do not vote. If current demand already selects the enhanced tier, the historical answers cannot change that action. If the current work is routine and the historical score is at least one, protection is inactive. Applicability is decisive only for routine work with a historical score below one. Formally, with the other outputs held fixed, changing applicability can change the action **if and only if** $s\geq0.5$ and $h<1$.

| Observed outputs | Final decision | What an applicability error can do |
|---|---|---|
| $s<0.5$ | Enhanced tier | No change; current demand already determines the action |
| $s\geq0.5$, $h\geq1$ | Standard tier | No change; historical protection is inactive |
| $s\geq0.5$, $h<1$ | Enhanced tier iff $a\geq0.5$ | Can add or remove historical protection |

This conditional influence explains why a component's accuracy can differ from final-action accuracy. We call an error **masked** when another condition prevents it from changing the action; it is still a semantic error. Conversely, two mistaken judgments can produce the right group for the wrong reason, which we call **error cancellation**. A correct applicability judgment can also block an erroneously low historical score, while an incorrect one can let that score cause overprotection. We therefore evaluate individual judgments, final actions and error propagation separately, and Section 5.5 illustrates each outcome with a complete case trace.

## 4. Policy-conditioned evaluation

The benchmark measures whether a router follows the declared group-selection policy at a checkpoint. **Action accuracy** means choosing the reference model group; it does not mean that an executor completed the underlying task. Three complementary studies address this question:

- The independent engineering study tests protection and release on newly constructed workflows under a frozen protocol (Section 5.2).
- The matched engineering study compares backends and answer formats on the same inputs, then audits an expression problem in the direct prompt (Section 5.1).
- The prospective mathematics study changes the current-demand criteria while keeping the historical mechanism, against a direct prompt specified before the new results were seen (Sections 5.4–5.6).

A **workflow** is one underlying scenario, a **record** is one state and language version of that scenario, and a **response** is one model call on a record. Translations and repeated calls therefore add observations, not independent workflows. The older V3 dataset supports development diagnostics and sampled human review and is kept separate from independent validation.

Case IDs use IG for independent engineering workflows and MR for mathematics. The suffixes `a`/`b` mark paired states, and `en`/`zh` mark languages. Each case trace names the relevant record, and the reproduction tool exposes its full context and all stored responses (Section 6).

### 4.1 Two participation tracks

The action track accepts any system that chooses the standard or enhanced tier from the visible record and the full declared policy. The typed track additionally evaluates current demand, the historical rubric grade, applicability, probability outputs and error propagation. A direct selector is not penalized for lacking intermediate judgments. Both tracks use the same determinate action labels and count invalid or missing planned responses as end-to-end failures.

Labels have three statuses. **Determinate** records contain enough evidence for a unique reference action. **Boundary** records support only a conditional expectation, often because a grade-one judgment leaves the continuous score near the routing threshold. **Insufficient** records lack evidence needed for a decision. Boundary and insufficient records stay in the dataset and the descriptive results but receive no forced action label or headline accuracy credit, and we report the fraction of determinate actions alongside accuracy. Section 5.6 examines one disputed exclusion, MR010_b, without replacing the original labels or main results.

### 4.2 Development and independent construction

The earlier engineering V3 study contains 48 workflows and 384 bilingual records: 312 determinate, 48 boundary and 24 insufficient. It provides development-era diagnostics and the material for human review. Because the protocol was developed on it repeatedly, we do not retroactively present it as a fresh blind test.

For prospective validation, an independent AI author received the public engineering policy and data format, but not the frozen prompt, earlier failure cases or evaluated-model outputs. A separate AI reviewer first assigned anonymous labels and froze them, then reconciled them with the author's labels. Access separation was procedural rather than enforced by operating-system isolation. These roles are AI construction and AI review, not independent human data collection. This public draft does not disclose the model identities of the AI construction and review systems, which limits exact regeneration and analysis of model-specific bias.

The initial batch comprised 24 workflows, each with two states and two languages, giving 96 records plus 24 preselected paraphrases. Before any evaluated-model outputs existed, a coverage audit found too few determinate workflows involving obsolete core failures. A recorded amendment added two independently authored workflows with eight records, preserving the original conversations and labels. The final primary panel has 104 records: 86 determinate, 16 boundary and two insufficient. The 24 paraphrases form a separate panel, and results distinguish the original cohort from the coverage-directed supplement.

The engineering construction thus has two sequential AI authoring sources: one for the original 24 workflows and paraphrases, and another for the two-workflow supplement. Both cohorts went through the same public-policy, anonymous-first-label procedure. The matched engineering comparisons reuse this dataset and add no authoring sources. The 26 workflows are resampling units, not 26 independent human sources or 26 independent model architectures, so construction-source diversity remains limited.

### 4.3 Human review and label reliability

Two people independently reviewed the same 96 preselected V3 records. Each used their own independent AI assistant for comprehension and engineering knowledge and personally reviewed the resulting judgments. Neither saw the original answers or model outputs in the review interface. The assistants differed from the case author, but their exact models and prompts were not recorded. Responses were entered in batches, so submission intervals do not measure reading time.

The reviewers agreed on action/status for 92 of 96 records (Cohen's kappa 0.932), and both endorsed all 78 originally determinate actions. Their differences concerned ambiguity and evidential sufficiency; they remain available, and the frozen original labels are unchanged. This is a valid independent, AI-assisted human review of the sampled V3 material. It does not cover the new independent cohort, and optional ratings of scenario naturalness were not collected.

### 4.4 Analysis and fair competitors

Our main accuracy measure first averages action correctness within each workflow and then gives every workflow equal weight; we call this **workflow-macro accuracy**. Paired bootstrap intervals resample whole workflows, keeping translations, states and repeats together. They describe uncertainty within the constructed sample, and the small number of generation sources limits broader inference. We also report protection for relevant failures, release after a failure becomes obsolete, behavior after healthy stages, paired-context contrasts and per-language results.

We use two kinds of comparison. A **direct selector** receives the same visible facts, the exact assessment definitions and the full routing objective, and selects the group itself; it tests whether decomposition helps beyond giving the model the complete policy. **Mechanism ablations** remove parts of HiRoute's rule: current-demand-only ignores history, while ungated history acts on a low historical score without checking relevance. These ablations show what the removed conditions contribute. Since relevance is part of the target policy, beating an ungated rule cannot by itself show that the model's applicability judgment is reliable.

A fresh matched campaign crosses Jev 1.13, Jev 1.14 and Qwen 3.8 Flash with direct and decomposed formats: 128 unchanged inputs, two interleaved repeats and 1,536 new calls. Both Qwen formats use non-thinking mode, temperature zero, JSON output and a 768-token completion cap. Interface development used separate records, and no prompt was revised under test guidance. The campaign is prospective for its conditions, but the dataset had already been evaluated and is therefore not newly blind. Earlier Qwen configurations remain preliminary results, not the principal backend comparison. The direct encoding includes the complete assessment definitions and composition policy. It is fully informed, but we do not presume it is the best possible direct prompt for each backend.

### 4.5 Mathematical custom-policy construction and matched execution

The mathematical study asks whether the same historical mechanism can operate under a different declared task standard. Its criteria distinguish a supplied, settled mathematical path from an unresolved proof obligation, theorem application or counterexample search. Only the two current-demand criteria change; q0's general instructions, the exact historical rubric and applicability question, continuous scoring and composition all stay fixed. The router interprets these criteria from the record and does not independently establish mathematical truth.

One independent AI author prepared 16 workflows, each with two states and two languages, working from the public policy and format rather than prior evaluated-model outputs. It wrote both languages, sealed the conversations before annotating them, and recorded a self-review. All 64 conversations keep their reviewed wording and have empty prior-context arrays. Bilingual versions do not count as independent authoring sources.

A separate AI reviewer first labeled the anonymous records and froze its answers, then reconciled them with the author's labels. Two independent human participants later reviewed all 64 records, each with their own AI assistance and without author or model answers in the interface. Neither reported mathematical research experience. Their action/status agreement is 60/64 (kappa 0.902), and both endorse all 50 original determinate actions. Agreement on naturalness category is 37/64; every rating is natural or plausible, which does not validate the scenarios as representative workflows. Before any model calls, the plan's domain-expert prerequisite was explicitly amended to allow this disclosed non-expert review; we do not retrospectively claim that the prerequisite was met. The study keeps the AI author, the anonymous AI labeling and adjudication, and the later AI-assisted human judgments separate, and none of them counts as an additional mathematical expert. The provenance record keeps role and source identifiers and does not invent model versions for authors or human assistants where none were recorded.

Pre-call adjudication retains 50 determinate, 12 boundary and two insufficient actions (78.125% determinate coverage). The only grade change is MR005_a, whose bilingual single grade moves from 1 to the allowed set {0,1}: useful proof structure coexists with non-delivery of the only missing bound, and current reasoning still fixes the enhanced tier. MR015_a remains an action boundary and MR010_b insufficient, while MR016_b keeps its component-level reference ambiguity even though both humans favor not-established. The originals and all disagreements remain available. Unique component labels cover 62 current-demand, 60 grade and 62 applicability records, so their denominators differ from the action panel.

Before any mathematical-model response, an independent method review and a 33-file freeze bound the inputs, labels, exact prompts, requests and analysis. We use the single explicit-condition, local-scope direct rewrite from Appendix D, replacing only its embedded demand criteria and domain phrase. Native and Qwen conditions receive the same state and question semantics. The six cells run in randomized interleaved order with two repeats, for 768 calls. Every call returned HTTP 200, with no retries or unknown outcomes. One Qwen direct response copies the question definitions and hits the 768-token cap; it stays in the planned denominator as a format failure. No prompt, threshold, case or label changed after the outputs were seen.

## 5. Results and mechanism evidence

### 5.1 Engineering comparison and representation audit

The matched engineering campaign presents the same full objective in direct and typed formats. The large Jev gaps below come with an important qualification: although the original direct prompt contains the full policy, a later audit found that its wording can suppress the historical override. We keep the frozen comparison and examine this expression problem before interpreting the gain.

| Backend | Original direct action | Three typed judgments | Difference (percentage points) | Paired 95% interval |
|---|---:|---:|---:|---|
| Jev 1.13 | 67.31% | 97.12% | +29.81 | [+16.35, +44.23] |
| Jev 1.14 | 67.31% | 96.63% | +29.33 | [+15.87, +43.27] |
| Qwen 3.8 Flash | 91.35% | 91.83% | +0.48 | [−3.85, +5.77] |

All 1,536 planned responses are valid, with no retries or unknown outcomes. Main accuracy is workflow-macro over 86 determinate records in 26 workflows, each repeated twice; the 16 boundary and two insufficient records remain unforced. On the original 24-workflow cohort, three-question/direct accuracies are 96.88%/68.75%, 96.35%/68.75% and 92.71%/92.19%. Qwen's small net gain combines 11 corrected actions with ten regressions, so decomposition does not help every backend uniformly.

Both typed Jev configurations protect all 52 related-core-failure responses and release all 28 obsolete-core-failure responses. The original direct configurations also release all 28, but miss all 52 protections. In IG001, a CSV download still contains an excluded account, and adding a column while regenerating the same download remains affected. The first English Jev 1.13 typed response gives $s=0.89,h=0,a=0.99$ and correctly invokes protection, whereas original direct selects the standard tier. Qwen selects the enhanced tier in both formats.

The expression problem has two parts. The direct categories refer back to a policy instead of stating the selection conditions, and instructions meant only for the current-demand assessment can appear to govern the whole decision. A model may therefore read “history does not change task complexity” as a reason to ignore history when selecting the group. The original direct development checks covered healthy continuations, not failure protection. On all 172 determinate responses per native version, the original direct actions equal the corresponding typed current-demand-only actions. This demonstrates that the historical override was underused, although the separate typed answers cannot reveal the direct model's internal reasoning.

A single policy-equivalent rewrite states the action conditions explicitly and scopes the embedded component instructions locally. In a frozen supplement on the already-seen inputs, it raises Jev 1.13 direct accuracy to 92.31% but Jev 1.14 only to 68.27%, while the original controls reproduce every earlier determinate action. The rewrite changes two expression features at once, and the supplement reruns neither the three-question nor the Qwen cells. Appendix D preserves the probes, full results and audit. These findings establish sensitivity to representation, not an intrinsic inability to make compound decisions, and they do not identify an optimal direct prompt.

Comparing those saved explicit-direct outputs with the earlier three-question run shows where the two still differ. Among 28 responses requiring current reasoning, every native method and version gets 24 right. For the 52 routine responses that need protection from a related core failure, explicit direct gets 42/52 and 2/52, against 52/52 for three questions. For the 92 routine responses that should use the standard tier, explicit direct gets 92/92 in both versions, against 90/92 and 89/92 for three questions. The remaining tradeoff is therefore stronger failure protection at the cost of some overprotection. These comparisons span separate runs and are descriptive; Section 5.4 compares the same frozen explicit-direct rewrite with three questions within one contemporaneous mathematics campaign.

### 5.2 Independent validation and structural ablations

Does applicability control protection usefully, beyond simply escalating after any low historical score? The earlier independent campaign compares the complete method with a separately queried two-question ablation that omits applicability.

| Frozen method | Independent primary panel | Original cohort only |
|---|---:|---:|
| Jev 1.13, complete three-judgment method | 97.12% | 96.88% |
| Jev 1.14, complete three-judgment method | 96.15% | 95.83% |
| Jev 1.13, ungated two-judgment ablation | 73.08% | 75.00% |
| Jev 1.14, ungated two-judgment ablation | 73.08% | 75.00% |

These are workflow-macro accuracies on determinate actions, not task-completion rates; determinate primary coverage is 82.69%. The native three-minus-two differences are 24.04 percentage points for Jev 1.13 (paired 95% interval [11.54,38.46]) and 23.08 for Jev 1.14 ([11.54,36.54]). On the original cohort alone the differences remain positive at 21.88 and 20.83 points. Because the native requests differ, these contrasts may reflect changes in the individual answers as well as in the composition rule.

To isolate the rule, we also hold each complete three-question response fixed and recompute its action with and without the applicability condition. Restoring the condition corrects 38 and 36 actions in the earlier independent campaign, with no added regression on its determinate primary panel. The fresh matched campaign, a separate set of observations, gives 38 and 37 corrections. Twenty-eight corrections per version concern obsolete core failures; the rest concern irrelevant local problems or uncertain severity under withdrawn requirements. This isolates the gate's arithmetic contribution on observed answers; it does not show that every history-aware router needs this decomposition.

Both versions keep protection in all 52 responses from the 13 workflows with relevant core failures, and release it in all 28 responses from the seven workflows with obsolete core failures. These counts include bilingual variants and repeats, and zero observed misses is not a population guarantee. Across 48 preselected original/paraphrase response pairs, neither version changes its action, and all 36 determinate pairs are correct under both wordings. This shows local robustness to expression, not invariance under arbitrary custom policies.

### 5.3 Two mechanisms must remain distinct

The independent panel mainly demonstrates how genuine historical problems are restricted to their current scope. In IG017, an earlier signed archive used the wrong team. The user later reports re-signing it correctly and passing the portal's profile check. A low historical score is still appropriate for the failed original stage, while low applicability lets the pending upload-preset work use the standard tier.

Earlier V3 diagnostics show a second phenomenon: the applicability judgment sometimes stops an erroneously low score on a stage that was actually reliable from propagating. Holding full responses fixed corrects 74/82 actions, including 22/30 of this second type. Those cases do not show that the historical scorer was right. The new independent panel contains no corrections of this type, so the two studies' mechanism contributions must not be pooled into a single claimed replication.

### 5.4 Prospective mathematical custom-policy results

| Backend | Explicit direct | Three questions | Difference (points) | Paired workflow 95% interval | Corrected / regressed calls |
|---|---:|---:|---:|---|---:|
| Jev 1.13 | 96.88% | 96.88% | 0.00 | [−4.69, +4.69] | 2 / 2 |
| Jev 1.14 | 95.31% | 97.66% | +2.34 | [−2.34, +9.38] | 4 / 1 |
| Qwen 3.8 Flash | 92.19% | 98.44% | +6.25 | [−1.56, +15.62] | 10 / 2 |

These are equal-workflow means over the original 50 clear records in 16 workflows, with 100 planned clear calls per cell, and every difference interval includes zero. Under this frozen evaluability choice Qwen has the highest three-question point estimate, but the alternative MR010_b interpretation in Section 5.6 changes that ordering. The study establishes neither a stable backend ranking nor a universal advantage for decomposition. The pronounced weakness of direct 1.14 in engineering does not recur in aggregate here, which cautions against attributing a dataset-specific pattern to an intrinsic limitation of that version. Datasets from different domains also do not form a non-inferiority design.

Only two workflows contain the four routine records that require protection from a related core failure (eight calls per cell). Direct protection succeeds 6/8, 4/8 and 4/8 times for Jev 1.13, 1.14 and Qwen, while three questions succeed 8/8 for each. In MR001_b, a fully supplied telescoping calculation was requested as an actual display, but the previous stage wrote only its heading. The next formatting task is still routine and depends on that missing display. English Jev 1.14 direct chooses the standard tier twice, whereas the decomposed responses give s=1, h=0.06 and a=0.94 and correctly invoke protection. This supports the declared history-aware objective on that case; it does not predict that escalation will improve mathematical execution.

The comparison contains regressions as well as gains. After healthy stages, Jev 1.13 overprotects both Chinese MR009_b repeats and 1.14 one English MR004_a response, and Qwen fails to release an externally completed correction in both Chinese MR006_b repeats. Conversely, Qwen releases the unrelated notation-key work in MR008_a, correcting three semantic errors and one invalid direct output. Section 5.5 traces these changes through the component outputs.

Across 24 boundary calls, the native three-question methods select the enhanced tier 23 and 22 times, against 1 and 0 for direct; Qwen three-question selects it once. These unforced actions reveal different boundary behavior and earn no accuracy credit. An independent recomputation from the raw responses reproduces all main and component results, the intervals and the handling of invalid calls. API-reported Jev usage charges total USD 0.051580368, and Qwen consumes 565,243 Token Plan tokens. The fixed historical mechanism thus works under this custom criterion, but because task distributions differ across domains, the study does not establish non-inferior customization.

### 5.5 Component quality, action quality and error propagation

Qwen's applicability accuracy is 77.34%, yet its final-action accuracy is 98.44%. Two things make this possible: the two percentages score different sets of records, and the rule allows an action to be right despite a component error. We examine each before interpreting the ranking. All percentages below are workflow-macro. The grade column scores the most likely rubric grade, although routing still uses the supplied continuous score.

| Backend | Current demand, 62 records | Historical modal grade, 60 | Applicability, 62 | Final action, 50 |
|---|---:|---:|---:|---:|
| Jev 1.13 | 98.44% | 85.16% | 92.97% | 96.88% |
| Jev 1.14 | 98.44% | 89.06% | 92.19% | 97.66% |
| Qwen 3.8 Flash | 100.00% | 84.38% | 77.34% | 98.44% |

First, an applicability answer can have a unique label even when the action is a score boundary or lacks sufficient evidence. To compare like with like, a post-hoc check keeps the 48 records that have both a determinate action and unique applicability, covering all 16 workflows and 96 calls per backend. Applicability/action accuracy then becomes 93.75%/96.88% for Jev 1.13, 93.75%/97.66% for 1.14 and 91.41%/98.44% for Qwen. The Qwen gap shrinks, but lower applicability accuracy still coexists with the highest action point estimate on exactly the same records. This check supplements the original panels rather than replacing them.

Second, applicability is decisive only when the current work is classified as routine and the historical score is below one (Section 3.4). The table below assigns every uniquely labeled applicability error to one place: outside the determinate action panel, blocked by current demand, blocked by the historical score, or consequential for the action. Counts are responses, including repeats. Each error enters exactly one row, with current demand checked before the score.

| Applicability errors | Jev 1.13 | Jev 1.14 | Qwen |
|---|---:|---:|---:|
| Boundary or insufficient action label | 1 | 2 | 18 |
| Determinate action already selected by current reasoning | 4 | 4 | 9 |
| Determinate action insensitive because $h\geq1$ | 2 | 3 | 0 |
| Incorrect applicability changes the determinate action | 2 | 1 | 2 |
| **All applicability errors** | **9** | **10** | **29** |

Qwen's 18 non-determinate cases comprise 14 boundary and four insufficient responses, none of which counts as a correct action. Its nine current-masked errors occur where the demand judgment correctly selects the enhanced tier, and only two applicability errors change an action in the original determinate panel. All four insufficient responses are MR010_b. Their exclusion is disputed, so Section 5.6 shows its effect instead of treating those errors as inherently inconsequential. Qwen also makes no current-demand errors in the uniquely labeled demand panel and assigns no score below one to the 44 healthy-routine responses, whereas each Jev version does so twice. Together, these error locations and the evaluability convention explain the original action ranking; they do not show better historical interpretation or a ranking that would hold under other case mixtures.

Four traces separate mechanisms that an aggregate score conceals:

- **A boundary is not a wrong action (MR002_b_en).** Four of five requested ledger rows were delivered, and the remaining row must be included in the pending table. The gold grade is one, applicability is positive, and the action is a continuous-score boundary. Qwen returns $s=0.95,h=1$ twice, with $a=0.05$ and then $0.90$. One applicability label is wrong, yet neither response triggers the strict $h<1$ condition, so both select the standard tier and neither receives determinate-action credit.
- **A correct action can mask a wrong applicability judgment (MR014_b_zh).** A requested plan was delivered, and the pending lemma note now requires resolving unproved replacement conditions. The gold answer is current reasoning, grade two and no applicable past failure, hence the enhanced tier. Qwen gives $s=0.10,h=1.75,a=0.90$: its applicability is wrong, but current demand correctly selects the enhanced tier. Jev instead assigns $s=0.67$–$0.74$ and wrongly selects the standard tier. Jev 1.13 also gives $h=0.85/0.80$, so deleting its correct negative applicability gate would accidentally recover the enhanced tier through that erroneous low score. That would be error cancellation, not justified historical protection.
- **A wrong applicability judgment can cause overprotection (MR006_b_zh).** A copy editor has completed both symbol replacements and supplied the corrected sentence, and the pending task only adds a heading and quote format. Qwen correctly keeps the original failure score, $h=0.20$, but with $s=0.95,a=0.90$ it wrongly escalates both repeats. Jev keeps the historical failure score and correctly releases applicability. Here the third judgment is consequential, and Qwen gets it wrong.
- **Two errors can reinforce each other (MR009_b_zh).** The assigned finite comparison was delivered, with generalization explicitly excluded, and the pending task only formats it. Jev 1.13 reports $s=1,h=0.97/0.91,a=0.67/0.73$: an erroneously low score and false applicability together produce the enhanced tier. In MR004_a_en, similarly low scores from Jev 1.14 produce one wrong action at $a=0.52$ and one correct action at $a=0.47$. The gate can block a score error, but it cannot guarantee that the two judgments avoid sharing a misreading of the stage's responsibility.

Finally, same-response mechanical ablations show what the stored outputs contribute to policy execution, without extra API calls or prompt changes:

| Backend | Current demand only | History without applicability | Full HiRoute rule | Gate restored: corrected / regressed calls |
|---|---:|---:|---:|---:|
| Jev 1.13 | 92.19% | 85.94% | 96.88% | 12 / 2 |
| Jev 1.14 | 92.19% | 84.38% | 97.66% | 13 / 0 |
| Qwen 3.8 Flash | 93.75% | 92.19% | 98.44% | 7 / 0 |

The denominator is the original 50 determinate records with two repeats, macro-averaged over 16 workflows. Restoring applicability reduces overprotection. In Qwen's unrelated MR008_a notation-key task, for example, $h=0.20,a=0.05$ releases protection, while the paired request for the still-missing original table stays protected. Jev 1.13's two action regressions come from removing the accidental MR014_b cancellation described above, which shows that semantic correction and action improvement are not the same thing. These ablations measure arithmetic contributions on observed judgments under a policy that explicitly values relevance. They are neither separately queried competitors nor causal evidence of better downstream execution.

The System One contribution, then, is an inspectable decision interface coupled to an explicit application rule. The component track diagnoses whether the right facts were recognized, the action track measures whether the declared policy was followed, and the propagation audit explains which mistakes mattered. A high score in any one layer cannot stand in for the other two.

### 5.6 A disputed evaluability choice: MR010_b

MR010_b asks for a lemma inventory drawn from a separately supplied marked page and explicitly requires the inventory to be displayed in the conversation. The assistant replies only that it is ready. The pending request asks it to apply the remaining margin note to that same inventory and return the updated version. The separate page, the inventory contents and the note are not available to the evaluator.

Before the model calls, annotation confirmed grade zero for non-delivery of the explicitly requested display and positive applicability, because the same deliverable is still required. It nevertheless marked current demand as unknown and the final action as insufficient, adopting a conservative requirement that the pending work's contents be known before an action label is awarded. The pre-call adjudication had already noted that an enhanced-tier answer based on protection was defensible. Both language records were kept and sent to every condition, so their exclusion from action scoring was not chosen after seeing results.

Independent review identified a genuine asymmetry in that convention. Under the declared policy, an established historical-protection branch can justify the enhanced tier even when current demand is unknown: discovering additional reasoning would add another reason to select the enhanced tier, not cancel the existing one. MR016_b already applies the converse logic, keeping the enhanced tier on the basis of current reasoning while applicability is unknown. Nor does the record establish an additional product-history eligibility failure that would justify treating MR010_b differently. We therefore regard the original exclusion as a disputed evaluation choice, not a necessary consequence of the routing rule, and we add no runtime requirement that every detail of the current task be known before historical protection can operate.

The practical question is whether these two records belong in the action denominator. The table reports a separate, reviewer-proposed sensitivity that scores MR010_b_en and MR010_b_zh as enhanced tier, because non-delivery and applicability are established. Every original label and main result stays unchanged. The calculation uses the same saved responses, continuous scalars, thresholds, other labels and workflow aggregation, giving 52 scored records, 104 calls per cell and the same 16 workflows. It makes no new calls and does not substitute a discrete grade for the returned score.

| Backend | Original direct | Original three | Sensitivity direct | Sensitivity three |
|---|---:|---:|---:|---:|
| Jev 1.13 | 96.88% | 96.88% | 93.75% | 96.88% |
| Jev 1.14 | 95.31% | 97.66% | 92.19% | 97.66% |
| Qwen 3.8 Flash | 92.19% | 98.44% | 89.06% | 95.31% |

Both Jev three-question configurations select the enhanced tier on all four added responses, while every direct configuration and Qwen three-question select the standard tier. Qwen's four added responses give $s=0.95$, $h=1.75/1.80$ and $a=0.05$, both overestimating historical delivery and rejecting applicability. Under the candidate interpretation these become four additional action errors, and fixing applicability alone would not repair them, because the high historical score still blocks protection. The applicability-error partition would then be 14 boundary, nine current-demand masked, four score-masked and two action-changing errors, still 29 in total. Here masking means the error has no marginal influence on the action, not that the resulting action is correct.

The sensitivity shows that the original highest point estimate is not robust to the evaluability convention. That it favors Jev is no reason to adopt the alternative as the official score. The main table and its intervals remain tied to the original frozen panel; the sensitivity does not inherit those intervals and does not establish a new significant ranking. What it does reveal is a labeling-consistency issue that future prospective evaluability rules should resolve symmetrically for either sufficient policy branch.

### 5.7 Remaining failure boundaries

In IG008_b, two release owners have not resolved whether to remove an old command or keep an alias for automation, and the assessed stage correctly records that disagreement. The pending instruction asks to finish a compatibility paragraph. Both Jev versions treat this as routine writing, even though it requires resolving an unapproved tradeoff. Historical protection should not invent a past failure to repair a current-demand error. Only four independent-panel workflows carry reasoning-demand labels, and their workflow recall is 75% in both versions, a weakness that high aggregate demand accuracy hides.

In IG002, a deployment plan omits a bounded rollback section. The pending task only creates a migration file from an agreed template, keeps the old read path and does not execute the deployment. Historical scores below one sometimes combine with a wrong applicability judgment to escalate this step. The modal grade is one, but switching to modal-grade routing would violate the frozen rule. The case illustrates how a project-level concern can be overextended to the responsibility of the immediate step.

These two workflows account for every remaining three-question response error on the primary panel: six for Jev 1.13 and eight for Jev 1.14 in the original validation, and six and seven in the fresh matched campaign. IG008 also fails in both Qwen formats, pointing to a shared boundary rather than a Jev-specific defect. Different questions can misread the same evidence, so semantic separation does not guarantee statistically independent errors. In particular, applicability can neither rescue a genuine failure scored above the protection threshold nor override a current-demand judgment that already selects the enhanced tier.

### 5.8 Mathematical workflows: protecting unresolved dependencies

The original mathematical panel contains only two workflows in which routine work requires protection from a related core failure. A separate, independently authored panel covers 16 mathematical-workflow families and 136 bilingual records. It tests a specific mechanism: protecting bounded work that still consumes a failed prerequisite, while releasing protection after repair, withdrawal or a switch to independent input. The [supplementary research package](supplements/mathematics-history/README.md) contains the cases, labels, exact question definitions, semantic responses and offline replay for this section and Section 5.9.

The independent AI author received a neutral policy and schema without evaluated prompts, earlier cases or model outputs, and audited its own cases; this is not expert-human review. Both evaluated definition sets were fixed before the cases and labels were inspected. The existing definitions and a concise alternative each have a matched direct selector embedding their exact component definitions and composition formula. The concise alternative distinguishes a supplied derivation from a merely named method, retains the historical grading question, and defines applicability through a supported failure that remains unresolved and affects current work. Continuous scoring and thresholds are unchanged. Its serialized applicability definition is approximately 41% shorter in characters, not tokens. It is evaluated separately from the definitions used in the original main tables.

Two repeats across two versions, two definition sets and two output forms produced 2,176 calls, all valid. Each comparison cell has 272 calls, from 136 records repeated twice, rather than 272 independent tasks. As in the original analysis, the principal estimates below give equal weight to workflow families and resample paired workflows for uncertainty.

| Backend | Definition set | Matched direct | Three questions | Difference (points) | Paired workflow 95% interval |
|---|---|---:|---:|---:|---|
| Jev 1.13 | Existing | 97.03% | 97.11% | +0.08 | [−3.44, +4.30] |
| Jev 1.14 | Existing | 89.69% | 99.22% | +9.53 | [+4.69, +14.45] |
| Jev 1.13 | Concise alternative | 99.61% | 98.59% | −1.02 | [−3.20, +0.78] |
| Jev 1.14 | Concise alternative | 93.20% | 100% | +6.80 | [+2.89, +10.94] |

The action-changing distinction is visible when the concise-definition results are separated by their role in the policy. Counts below retain both language versions and repeats.

| Pending-work condition | Jev 1.13 direct → three | Jev 1.14 direct → three |
|---|---:|---:|
| Bounded work still requires an unresolved failed prerequisite: protect | 47/48 → 48/48 | 30/48 → 48/48 |
| Bounded work after repair, withdrawal, independent input or healthy progress: release | 160/160 → 156/160 | 160/160 → 160/160 |
| New mathematical reasoning is required: enhanced tier | 64/64 → 64/64 | 64/64 → 64/64 |

For Jev 1.14, all 18 paired gains with concise definitions occur in protecting bounded work with an unresolved dependency; there are no paired action regressions in this panel. The existing protocol also captures the protection benefit: 21/48 becomes 48/48, while release changes from 160/160 to 158/160. Thus the protection benefit also holds with the existing definitions. The concise alternative's incremental gain over the existing three-question form is only two calls in 1.14 and four in 1.13; its advantage includes a shorter expression, not a large new routing improvement.

The 1.13 comparator is already near ceiling. Its concise three-question form protects one additional call but unnecessarily upgrades four calls, corresponding to two Chinese records repeated twice: one supplies independent new counts despite an unresolved old cleaning error; the other withdraws the old document and supplies a complete replacement. The result supports a substantial 1.14 protection benefit under these definitions and inputs, while identifying release errors in 1.13.

This panel is reported separately from the original mathematical main panel. Its synthetic workload frequencies are not estimates of production traffic. All records package post-stage context as a recap inside the latest user message; Section 5.9 also tests conversations with separate prior messages. The direct and decomposed forms differ in length and interface, so this comparison does not isolate token length as a causal factor. The supplementary comparisons evaluate the two Jev versions; Qwen results remain those in the original main panel. All action scores measure conformance to the declared routing policy, not downstream mathematical success or realized savings.

### 5.9 Protection and release across dependency states

Two complementary cohorts test dependency states and their temporal representation. They contain 92 bilingual records and produce 736 interleaved calls, all valid, with one call per record, model version, definition set and output form. The dependency-state cohort has six workflows, each with unresolved output, a promise-only repair, a completed user repair with complete replacement content, and a withdrawn or independently replaced target, in both languages (48 records). Its author knew earlier results: this is a targeted challenge panel, not an independently authored population sample. It covers worked derivation notes, numerical manifests, coefficient exports, lemma-condition cards, source records and interval-certificate formatting. Mathematical inputs are supplied, so labels do not require the router to solve the underlying mathematics.

The chronological cohort contains 44 records from a separate AI author, with prior messages, the last assessed execution and one current user request stored separately. An independent AI reviewer anonymously labeled all 92 records before viewing author answers; reconciliation and label freezing preceded the evaluated model calls. The chronological cohort has eight conditional cases: a supported local defect at rubric grade one does not uniquely determine whether the continuous score is below one. Four cases have insufficient evidence. Author labels, independent initial labels and reconciled labels are available in the research package. This is independent AI review, not expert-human validation.

| Backend | Definitions | Targeted matched direct | Targeted three questions | Chronological direct | Chronological three |
|---|---|---:|---:|---:|---:|
| Jev 1.13 | Existing | 42/48 (87.50%) | 47/48 (97.92%) | 32/32 | 31/32 |
| Jev 1.14 | Existing | 29/48 (60.42%) | 48/48 (100%) | 30/32 | 31/32 |
| Jev 1.13 | Concise alternative | 34/48 (70.83%) | 47/48 (97.92%) | 32/32 | 32/32 |
| Jev 1.14 | Concise alternative | 34/48 (70.83%) | 48/48 (100%) | 32/32 | 32/32 |

The targeted cohort is balanced across six families, so record- and workflow-weighted estimates coincide. The existing three-question form protects 24/24 related failures in both versions, compared with 20/24 and 5/24 for its direct counterpart; post-problem release is 23/24 and 24/24, compared with 22/24 and 24/24. The concise alternative protects 23/24 and 24/24, compared with 10/24 in both direct conditions, and all concise methods release 24/24. There are no direct-correct/three-wrong pairs within either definition set on this panel. The existing-definition paired differences are +10.42 and +39.58 points; six-family bootstrap intervals are [+2.08,+18.75] and [+33.33,+45.83]. Concise-definition differences are +27.08 and +29.17, with intervals [+20.83,+33.33] and [+16.67,+41.67]. These are descriptive intervals for the six deliberately constructed families.

For example, the agent was asked to record the supplied R17 experiment settings but wrote an unrelated R28 manifest. Its latest assessed response promises to replace the fields and states that the change has not been made. The next request only converts the manifest into a table. With the existing definitions in Chinese, Jev 1.14 returns s=0.98, h=0.07 and a=0.94, retaining protection; its matched direct selector chooses the standard tier. When the user instead supplies the complete repaired R17 manifest, the historical score remains low (h=0.02), but applicability falls to a=0.23 and the three-question rule releases protection. The historical execution did not become successful retrospectively; its consequence for the pending input changed.

The chronological cohort has 32/44 determinate coverage (72.73%). It tests twelve post-problem release records, eight healthy regular tasks and twelve new-reasoning records. All concise methods score 32/32: this is retention on a different temporal representation, not additional evidence of superiority. Its eight conditional records contain the applicable local defects; no determinate grade-zero protection cases remain after review, so this cohort does not independently replicate protection recall. Three-question methods choose enhancement for all eight conditional records and the standard tier for all four insufficient records, without receiving accuracy credit for those choices.

Final routing remains distinct from component correctness. On the chronological cohort, concise applicability is correct on 40/40 evaluable records in both versions, while modal historical grades agree on only 23/40 and 20/40; the latter disagreement is retained. Across the eight three-question cells, discrepancies between the native scalar and its ordinal probability expectation are at most approximately 0.01 and cause no threshold crossing or action change. The main calculation always uses the native scalar.

These results strengthen the evidence for decomposed historical protection in explicitly stated dependency scenarios across both tested Jev versions. They do not show that the concise rewrite itself improves targeted three-question accuracy: both definition sets have the same total correct counts, and their direct comparators change substantially. The concise alternative's one targeted 1.13 miss is a Chinese empty-condition-card case with applicability 0.49; 1.14 returns 0.53. Neither threshold nor prompt was tuned after this observation. The synthetic cues are explicit, repairs provide complete content, and corpus proportions are controlled. We therefore keep this challenge set, the chronological cohort, the earlier 136-record confirmation and the original main panel separate, rather than combining them into a larger headline accuracy.

## 6. HiRoute implementation and reproducibility

HiRoute provides the executable path from the decision interface to model selection. Users configure task criteria, rubric descriptions and thresholds. The application assembles the context, attributes the assessed stage, validates the response, selects the group and records the decision. By publishing this path, we let other developers inspect and integrate the mechanism together with its research questions and results.

The public research package has three entry points. The [datasets](datasets/README.md) contain the bilingual inputs and original labels; the [data card](DATA-CARD.md) explains construction, dependence and disclosure limits; and the [reproduction guide](REPRODUCE.md) gives the offline commands. Replay parses the saved responses, reapplies the continuous-score rule and recomputes the principal machine-result tables without private files, credentials or network calls. It can also score another router and display a complete case with its stored model outputs.

The package keeps the exact request plans, invalid and abandoned outcomes, frozen parsing and scoring adapters, anonymous construction and review evidence, and a separate MR010_b sensitivity overlay, and all original labels remain available. Independent arithmetic replay verifies all 1,791 valid actions in the earlier prospective engineering campaign and all 1,536 in the fresh matched campaign; the earlier campaign's one invalid response remains a failure. Appendix B describes the freeze checks and execution deviations. Appendix C reports service-specific decision charges and latency: the matched engineering run records USD 0.096355392 in Jev charges and 1,006,558 Qwen Token Plan tokens. These figures describe routing overhead, not realized executor savings.

Product-path evidence is separate from offline arithmetic. At [public revision 62186edf](https://github.com/higress-group/HiRoute/tree/62186edf22550c24807dd4606a36c31727840598), 27 classification-module checks and one real-listener test passed. The listener test covers both difficulty-based and categorical routing over four successive decisions, verifies the provider calls, and observes protection being applied, suppressed and applied again. The pinned implementation keeps the mode identifier `smart_saving`, which this manuscript calls difficulty-based routing in product-facing text. The test uses controlled services and the product's default simple threshold of 0.8, whereas the paper uses 0.5. Replaying every saved paper response through the production path at the paper configuration is still open, and Qwen's research numerical adapter does not certify a shipped Qwen decision-provider integration.

Private human submissions and the undisclosed identities of the construction and review models remain outside the [versioned package](README.md#research-material-availability). The public files reproduce the machine-result arithmetic and expose the available labeling evidence, but they cannot fully regenerate the annotation process.

## 7. Limitations and conclusion

The study measures policy-conforming decisions at synthetic checkpoints within continuing tasks. It does not measure actual executor success, optimal cost, how often these situations arise in natural traffic, long unattended trajectories, or robustness at the eight-turn truncation boundary. Because the labels encode a historical-protection objective, beating an ablation that omits that objective does not establish optimal routing utility. Bilingual records and repeats do not add independent workflows. AI-assisted human judgments support the reviewed labels, but the mathematical reviewers were not domain experts, and the original mathematics panel contains only two workflows testing related-core protection. The supplementary panels broaden dependency coverage and test both recap-style and separate-message contexts, but remain synthetic checkpoints rather than naturally sampled trajectories. The two Jev versions also represent a single model family, not independent System One architectures.

The open-source HiRoute mechanism separates three questions: what the current task requires, what happened during execution, and whether that execution problem still matters. Typed System One outputs make these judgments observable, and application code makes their composition deterministic. Under the declared objective, the independent engineering data and a custom mathematical policy provide evidence of useful historical protection and release. They also show that input representation, current-demand errors and jointly mistaken historical judgments limit the result.

The strongest supported conclusion is that the open mechanism can apply a declared policy with inspectable protection, release and failure modes. A final-action score alone hides why a router succeeds, since another condition may mask a wrong judgment or two errors may cancel. The experiments also bound the comparison: engineering direct results depend on how the prompt is expressed, every original mathematical direct-versus-three difference interval includes zero, and the MR010_b sensitivity changes the backend ordering. The independent mathematical panel provides additional protection evidence for Jev 1.14, while the targeted dependency-state panel shows gains in both versions. The chronological cohort supports retention with concise definitions, rather than an additional advantage over matched direct selection. These separate populations bound where the mechanism helps; they do not establish its average benefit on production traffic. Continuous-score boundary cases still need evaluation against operational outcomes. Broader independent policy samples, production replay at the paper configuration and live execution studies would test, respectively, transfer, implementation correspondence and downstream utility.

## Appendix A. Legacy migration evidence

Legacy comparisons remain available for regression and migration analysis but are not the principal efficacy baseline. On the independent primary panel, legacy Jev 1.13/1.14 score 78.85%/75.00%, against 97.12%/96.15% for the complete methods. The complete method improves 30/36 individual response actions but regresses on two per version, both in IG002. Several protocol elements changed at once, so these differences do not isolate a single mechanism. All old results and negative cases remain in the artifact.

## Appendix B. Execution deviations and retained accounting

The first Qwen development request was rejected because JSON-object mode requires the literal word JSON in the prompt. Adding that output-format marker yielded 20/20 valid development checks. The first main campaign, however, used request bodies cached before the marker change. It was stopped and preserved, including four unknown outcomes, and no semantic outputs informed any prompt change. A replacement campaign regenerated all bodies, added exact body-to-adapter consistency checks, froze the amendment and reran every cell. Successful cells from the aborted campaign were not spliced into the reported results, and the resource records of development and aborted runs are kept separate from the completed main-run costs.

The first analysis run then failed before processing any predictions, because two imported modules had not been copied. Restoring byte-identical modules from the earlier, already-frozen analysis source fixed the import error, and no frozen scoring code, requests, labels or prompts changed. Independent raw-response arithmetic agrees with the completed report, and an independent whole-workflow bootstrap recomputation reproduces the three main intervals. These checks validate the numerical reduction; they do not independently validate every schema interpretation or certify the semantic labels. We disclose these procedural failures for reproducibility and do not count them as semantic errors of a model whose valid response was never obtained.

Source-integrity checks confirm that both the older V3 freeze and the 205-file independent freeze are unchanged, and the fresh matched engineering campaign also verifies its 42-file freeze. Exact prompt hashes, input and label versions, anonymous first labels, adjudication, raw semantic responses, request identities and costs remain traceable through the published package and its disclosure record.

## Appendix C. Service costs, latency and numerical consistency

### Observed decision costs

The table below aggregates the existing API-reported USD usage charges for every planned Jev request in the two completed matched campaigns; every call in these cells carries a usage charge. It covers boundary and insufficient cases as well as the determinate panel, and excludes the separate development and aborted runs. No new API calls or price assumptions are involved. The normalized column divides each cell's total charge by its observed request count and multiplies by 1,000; it is not a separately executed thousand-request experiment.

| Campaign | Model | Format | Requests | Total usage charge (USD) | USD per 1,000 requests |
|---|---|---|---:|---:|---:|
| Engineering | Jev 1.13 | Original direct | 256 | 0.026680080 | 0.104219 |
| Engineering | Jev 1.13 | Three questions | 256 | 0.021497616 | 0.083975 |
| Engineering | Jev 1.14 | Original direct | 256 | 0.026680080 | 0.104219 |
| Engineering | Jev 1.14 | Three questions | 256 | 0.021497616 | 0.083975 |
| Mathematics | Jev 1.13 | Explicit direct | 128 | 0.014730996 | 0.115086 |
| Mathematics | Jev 1.13 | Three questions | 128 | 0.011059188 | 0.086400 |
| Mathematics | Jev 1.14 | Explicit direct | 128 | 0.014730996 | 0.115086 |
| Mathematics | Jev 1.14 | Three questions | 128 | 0.011059188 | 0.086400 |

In these observed workloads, three-question routing costs about **USD 0.084–0.086 per 1,000 checkpoints**, with all three judgments evaluated in one API request per checkpoint. A charge this small makes the method practical as a low-cost decision step at successive agent turns. In both campaigns the three-question cells also cost less than the corresponding direct encodings, but that comparison is specific to these request encodings and service charges and is not a general pricing guarantee for decomposition.

These amounts cover decision calls only, not executor work, dataset construction or review. Qwen runs on a subscription Token Plan and is reported in tokens; we do not derive a comparable per-request USD cost from that allowance or claim a measured dollar saving relative to Qwen. The evaluated backend identities and versions are disclosed, but this public draft does not name the hosted Jev access service, which limits exact reproduction of billing and transport conditions.

### Client-observed latency excluding connection setup (n=10 per cell)

| Backend | Format | Median latency | p95 latency |
|---|---|---:|---:|
| Jev 1.13 | Direct | 504 ms | 595 ms |
| Jev 1.13 | Decomposed | 468 ms | 533 ms |
| Jev 1.14 | Direct | 489 ms | 637 ms |
| Jev 1.14 | Decomposed | 468 ms | 571 ms |
| Qwen 3.8 Flash | Direct | 466 ms | 710 ms |
| Qwen 3.8 Flash | Decomposed | 992 ms | 1,530 ms |

### Scalar consistency

Native Jev historical scalars differ by at most 0.01 from the expectations reconstructed from the displayed probabilities. Qwen's generated scalar differs by more than 0.05 in seven of 256 decomposed responses, by up to 0.7. In a post-run sensitivity check, replacing only the scalar with that expectation would change three Qwen actions in the determinate main panel, each from correct enhanced-tier protection to an incorrect standard-tier selection. The reported main results keep the supplied scalars, with no arithmetic repair, modal-grade substitution or threshold calibration. This is why generated numerical reports and native decision outputs need separate consistency checks.

## Appendix D. Audited native-direct expression sensitivity

An independent post-run review identified a concrete threat in the native direct adapter: referential action categories and verbatim local component instructions may not reliably convey the intended scope. Before collecting new developer-probe responses, we froze exactly one policy-equivalent alternative. It places the enhanced-tier selection conditions and their standard-tier complement in the category definitions, and explicitly limits the embedded q0/q1/q2 restrictions to their respective assessments. The full assessment definitions, continuous scoring and thresholds are unchanged, and a separate reviewer checked semantic equivalence before any calls.

Eight new bilingual developer probes cover enhanced-tier selection by current demand, enhanced-tier selection by an applicable failure, release after a completed correction, and a local-score boundary. Across 32 calls, each native version goes from four to six correct determinate responses out of six; the two boundary responses per encoding receive no accuracy label. The improvements concern the still-undelivered required build-tag line, and Jev 1.14's new protection probabilities are only 0.53 and 0.52. Probes this small demonstrate sensitivity to expression, not general accuracy or robust margins.

This result triggered a separately frozen supplement on all 128 previously seen engineering inputs, comparing the original encoding with the unchanged explicit one for both native versions over two interleaved repeats. The original main experiment is retained in full, and the supplement estimates representation sensitivity on the already-seen sample. Because no decomposed or Qwen condition is rerun, comparisons with those earlier cells are descriptive and may include effects of execution time. The two representational changes are not separated, and no further prompt search was performed.

The supplement completed all 1,024 calls with valid responses, no retries and no unknown outcomes. Workflow-macro determinate-action accuracy rises from 67.31% to 92.31% for Jev 1.13 (+25.00 points; descriptive paired workflow interval [+13.46,+36.54]) and from 67.31% to 68.27% for Jev 1.14 (+0.96; [0,+2.40]). Both original controls reproduce every earlier determinate main action, although the probabilities vary. The explicit variant improves 42 and two determinate calls, respectively, with no observed regression. Relevant-core-failure protection becomes 42/52 and 2/52, while obsolete-failure release stays at 28/28 and healthy-history correctness at 52/52 for both. Boundary cases are unforced. On the original 24 workflows alone, explicit accuracy is 92.71%/69.27%, and on the two-workflow supplement 87.50%/56.25%.

For IG001_a in English, Jev 1.13's enhanced-tier probability moves from 0.38/0.38 to 0.83/0.80 across the two repeats, correctly protecting the still-faulty download, whereas Jev 1.14 moves from 0.05/0.04 to 0.39/0.55 and protects only once. Both explicit variants still fail the Chinese dependent-index continuation IG026_a and the unresolved compatibility decision IG008_b, and both correctly release IG002_b, where earlier decomposed outputs sometimes overprotect. These mixed outcomes are part of the result.

Passing the small developer probes did not establish semantic adequacy on the broader contextual panel. The supplement strengthens the specific diagnosis of expression sensitivity, especially for 1.13, without identifying an internal cause or proving that other direct formulations must fail, and no further prompt selection follows. API-reported Jev usage charges for this supplement total USD 0.111021120, separate from USD 0.003288432 for the developer diagnostic. Independent raw-choice arithmetic reproduces the main averages, but no full semantic reannotation was performed.

## References

1. TypeSafe. *Introduction*. https://docs.typesafe.ai/introduction (accessed 18 October 2026).
2. TypeSafe. *Score*. https://docs.typesafe.ai/primitives/score (accessed 18 October 2026).
3. Ong et al. *RouteLLM: Learning to Route LLMs with Preference Data*. https://arxiv.org/abs/2406.18665.
4. Ribeiro et al. *Beyond Accuracy: Behavioral Testing of NLP Models with CheckList*. ACL 2020. https://aclanthology.org/2020.acl-main.442/.
5. Parashar et al. *Whose turn is it? Context-guided model routing for stateful agents*. https://arxiv.org/abs/2610.19292.
6. Yang et al. *TwinRouterBench: Fast Static and Live Dynamic Evaluation for Realistic Agentic LLM Routing*. https://arxiv.org/abs/2605.18859.
7. Tran et al. *Arch-Router: Aligning LLM Routing with Human Preferences*. https://arxiv.org/abs/2506.16655.
8. Gardner et al. *Evaluating Models’ Local Decision Boundaries via Contrast Sets*. https://aclanthology.org/2020.findings-emnlp.117/.
