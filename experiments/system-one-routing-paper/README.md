# System One routing for long-running agents

[Read the manuscript](main.md)

**HiRoute: System One Routing for Long-Running Agents with History-Aware Protection**

This is the Markdown manuscript for community review before final typesetting and
arXiv submission. The main research snapshot is dated 18 October 2026, with a
mathematical dependency-state supplement dated 28 October 2026. It studies how a
System One decision interface can support routing that combines current task
demand with historical performance and the present applicability of an execution
problem. HiRoute supplies the open-source implementation of that decision contract.

## Reading guide

Start with the configuration-edit example in the [introduction](main.md#1-introduction).
It follows one failure through protection and release. The [method](main.md#3-method)
then explains why three judgments share one request and how their outputs select
a group. In the results, distinguish direct-selector comparisons from rule
ablations: they answer different questions. All action accuracies measure group
selection under the declared policy; they are not executor task-completion rates.

- [Method](main.md#3-method): state boundaries, three judgments, continuous scoring
  and the deterministic routing rule.
- [Evaluation](main.md#4-policy-conditioned-evaluation): declared engineering and
  mathematical policies, bilingual construction, AI roles, AI-assisted human review,
  and separation of determinate, boundary and insufficient cases.
- [Results](main.md#5-results-and-mechanism-evidence): direct and decomposed selectors,
  protection and release, and the propagation of component errors into actions.
- [MR010_b sensitivity](main.md#56-a-disputed-evaluability-choice-mr010_b): a disputed
  evaluability choice, reported separately from the original labels and main tables.
- [Mathematical dependency states](main.md#58-mathematical-workflows-protecting-unresolved-dependencies):
  independent workflows, controlled protection/release states and separate
  chronological checkpoints, with [data and offline replay](supplements/mathematics-history/README.md).
- [Decision costs](main.md#observed-decision-costs): API-reported Jev charges, including
  approximately USD 0.084–0.086 per 1,000 three-question requests in the measured workloads.
- [Implementation](main.md#6-hiroute-implementation-and-reproducibility) and
  [limitations](main.md#7-limitations-and-conclusion): what the evidence establishes
  and what remains untested.

The strongest supported claim is conformance to the declared routing policy on the
reported synthetic checkpoints, with observable protection, release and failure
modes. Final action accuracy is not executor task-completion rate or realized cost
savings. The manuscript does not claim a universal backend ranking or a universal
advantage of three questions over a well-specified direct selector.

## Exact study questions

The manuscript calls the two configurable routing roles **standard tier** and
**enhanced tier**. Frozen questions, labels and evaluator inputs retain `economy`
and `primary`, respectively. These identifiers and the numerical policy remain
unchanged. Difficulty-based routing can select the enhanced tier for either
current reasoning demand or applicable historical protection.

These files are byte-preserving copies of the frozen research question definitions.
The [policy manifest](policy-manifest.json) records their SHA-256 hashes. They are
study artifacts, not new product defaults or a replacement for the production
configuration contract.

| Question file | Use in the manuscript |
| --- | --- |
| [Engineering: three questions](policies/engineering-three-questions.json) | Complete method in the engineering studies; the development identifier C refers to this configuration |
| [Mathematics: three questions](policies/mathematics-three-questions.json) | Custom-policy study; only the two current-demand criteria differ from engineering |
| [Engineering: original direct](policies/engineering-direct-original.json) | Original frozen direct encoding in Section 5.1; retained with its representation confound disclosed |
| [Engineering: explicit direct](policies/engineering-direct-explicit.json) | Policy-equivalent expression sensitivity in Appendix D; not a newly blind dataset |
| [Mathematics: explicit direct](policies/mathematics-direct-explicit.json) | Contemporaneous direct comparison in Section 5.4 |

For eligible, complete history and valid answers, the paper selects the enhanced tier when
`s < 0.5 OR (h < 1 AND a >= 0.5)`: `s` is the simple-current-work probability,
`h` is the returned historical scalar on the 0–2 scale, and `a` is applicability.
Otherwise it selects the standard tier. The returned continuous scalar is retained; the
modal grade is not substituted for it. Invalid or ineligible history follows the
product contracts described in the manuscript. The paper's 0.5 simple threshold is
separate from the tested product default of 0.8.

## Research material availability

This directory includes the [bilingual datasets](datasets/README.md), [curated response
ledgers](responses/README.md), [exact execution requests and plans](execution/README.md), [anonymous
review evidence](reviews/README.md), and a portable [offline replay and evaluator](REPRODUCE.md).
The [data card](DATA-CARD.md) documents construction, dependence, disclosures,
intended use and label boundaries. [MANIFEST.json](MANIFEST.json) records public
file hashes; original source hashes are separately recorded under `provenance/`.

| Panel | Records | Use |
| --- | ---:| --- |
| Engineering primary | 104 | 26 workflows; original cohort and coverage supplement remain distinguishable |
| Engineering paraphrases | 24 | Separate expression panel, not extra independent workflows |
| Mathematics | 64 | 16 workflows with custom current-demand criteria |
| Development V3 | 384 | Historical development and audit evidence, not blind validation |
| Native direct probes | 8 | Interface diagnostics, not a general accuracy test |
| Mathematics: independent supplement | 136 | 16 independently authored workflows; separate from the original main panel |
| Mathematics: dependency states | 48 | Six targeted workflows with balanced protection and release states |
| Mathematics: chronological checkpoints | 44 | Separate-message representation; 32 determinate, eight conditional and four insufficient cases |

The nine execution archives retain matched, independent, expression-control,
development and abandoned campaigns separately. Invalid output, unknown outcomes
and not-started calls remain visible. Complete semantic response fields and LLM
completion text are retained; service metadata and remote response IDs are removed.
Original labels remain the scoring source. The [MR010_b overlay](sensitivities/mr010.json)
is separately identified as a post-hoc sensitivity and does not replace them.

The [mathematics supplement](supplements/mathematics-history/README.md) retains its
2,912 calls separately. In its 48-record dependency-state panel, the existing
three-question definitions reach 47/48 and 48/48 for Jev 1.13 and 1.14, versus
42/48 and 29/48 for matched direct selection. Both versions protect all 24 related
failures; release accuracy is 23/24 and 24/24. These controlled, explicitly stated
dependencies show where the mechanism helps; their frequencies do not estimate
production traffic. The independent and chronological panels retain their own
denominators, version differences and conditional labels.

From the repository root, with Python 3.9+ and no external packages or model calls:

```sh
python3 experiments/system-one-routing-paper/reproduce.py verify
python3 experiments/system-one-routing-paper/reproduce.py report --check --output /tmp/hiroute-paper-replay
python3 -m unittest discover -s experiments/system-one-routing-paper/tests -v
```

Choose a fresh output directory. The replay parses the saved responses and
recomputes actions, component scores, intervals, cohorts, error propagation, usage
charges and sensitivity. [REPRODUCE.md](REPRODUCE.md) also explains how to inspect a
complete case and evaluate another router in the generic or typed track. Commands
elsewhere in `experiments/` apply to their own case studies, not this paper.

The public draft identifies AI construction and review systems by role, without
disclosing their model identities, and describes the hosted decision access service
without naming it. Evaluated Jev/Qwen versions, AI participation, procedural
separation, numerical settings, usage charges and transport differences remain
explicit. These disclosure limits constrain exact regeneration and service-level
reproduction. They do not convert AI review into independent human review.

Original labels and reported decision-quality tables are retained. Any proposed relabeling or
analysis change must be explicit and separate from the frozen results. Private
reviewer submissions, invitation links, identity mappings, authentication details
and workstation configuration are not part of the public research materials.

The [pinned HiRoute implementation](https://github.com/higress-group/HiRoute/tree/62186edf22550c24807dd4606a36c31727840598)
is already public. Its controlled tests establish the specific production paths
reported in Section 6, not live executor utility or complete replay of the paper's
saved responses under its 0.5 threshold.

## Contributing to the manuscript

Please comment on the draft PR or propose focused edits to `main.md`. Useful review
includes whether the routing objective is clear, whether each claim matches its
reported population and comparison, and whether the examples and limitations make
the result understandable to agent developers. Keep concerns about annotations,
protocol variants and statistical interpretation explicit rather than silently
changing the existing results.

The frozen question files document the evaluated configurations. A proposed new
protocol belongs in a separately named variant, not an overwrite of those files.
Authors, affiliations, acknowledgments and final publication metadata will be
confirmed with contributors before typesetting and submission. This directory
contains no arXiv identifier or claim of peer-reviewed acceptance.
