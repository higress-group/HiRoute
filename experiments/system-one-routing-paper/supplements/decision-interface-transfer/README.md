# History-aware routing across native decision interfaces

This follow-up compares a complete-policy direct question with HiRoute's three
questions on identical contexts and definitions. It uses Jev 1.13/1.14 responses
from 28 October 2026 and OpenAI Decisions (`gpt-6-luna`) responses from 29 October.
The OpenAI connection is a research adapter, not a production integration claim.

The reported subset excludes 14 bilingual records whose applicability labels
depend on an inferred prerequisite or an unresolved cross-target policy boundary.
**The review happened after the model outputs were available. This is a post-hoc
reviewed subset, not a new blind evaluation.** The same IDs are excluded from every
backend, language and output form. Original labels and all 912 responses remain
available, including the excluded records. No prompt, threshold or score changed.

## Results for the reviewed subset

All percentages in this table are **workflow-family macro final-group accuracy**.
A workflow receives equal weight regardless of its remaining record count.

| Panel | Decision backend | Complete-policy direct | Three questions | Change, pp |
|---|---|---:|---:|---:|
| Engineering | Jev 1.13 | 89.42% | 96.15% | +6.73 |
| Engineering | Jev 1.14 | 67.31% | 96.15% | +28.85 |
| Engineering | OpenAI Decisions / gpt-6-luna | 50.00% | 91.35% | +41.35 |
| Mathematical dependency states | Jev 1.13 | 90.97% | 100.00% | +9.03 |
| Mathematical dependency states | Jev 1.14 | 54.86% | 100.00% | +45.14 |
| Mathematical dependency states | OpenAI Decisions / gpt-6-luna | 65.97% | 88.89% | +22.92 |

Engineering retains 98 records in 26 families: 80 determinate actions, 16
conditional and two insufficient. Mathematics retains 40 records in six families,
all with determinate actions. Applicability has 90 eligible engineering labels
and 40 mathematical labels; its denominator differs from final action accuracy.
Component results, language breakdowns, full-population results and paired gains
and regressions are in [RESULTS.md](RESULTS.md) and [reference-results.json](reference-results.json).

For context, on the original full population Jev 1.14 direct/three scores are
69.23%/96.15% in engineering and 62.50%/97.92% in mathematics; OpenAI scores
48.08%/89.42% and 56.25%/85.42%. These remain a separately reported population.

## What was excluded

[selection.json](selection.json) lists every excluded ID and both translations.

- Engineering: `IG005_b` and `IG015_b` infer that a missing investigation artifact
  is a prerequisite for a subsequent design decision; `IG026_b` switches artifacts
  while retaining a supplied-content construction operation. Six records total.
- Mathematics: the `released_target` cases in `derivation_note`, `run_manifest`,
  `coefficient_export` and `source_record` change artifacts while retaining related
  copy/format operations. Eight records total.

Artifact independence supports release, while a policy permitting transfer of a
specific failed operation can support protection. The reviewed subset avoids
treating that unresolved interpretation as a unique label. This does not establish
that all remaining cases are unambiguous, nor does it retrospectively make the
selection independent of results. The review was performed by the research
assistant with outputs visible, without a new independent human adjudication.

Exclusions change the mixture. Engineering determinate labels are 36 enhanced and
44 standard; mathematics has 24 enhanced and 16 standard. The latter originally
had 24 of each. To expose both sides of the policy, the replay also reports counts
by target group. For example, mathematical direct → three correctness is:

| Backend | Required protection / 24 | Required release / 16 |
|---|---:|---:|
| Jev 1.13 | 20 → 24 | 16 → 16 |
| Jev 1.14 | 6 → 24 | 16 → 16 |
| OpenAI Decisions / gpt-6-luna | 24 → 22 | 2 → 14 |

Thus the two interfaces improve for different reasons: OpenAI mainly corrects
excess protection, with two protection regressions; Jev 1.14 mainly recovers
missing protection. Neither effect proves an executor would complete more tasks.

## Reproduce without API calls

From the repository root, with Python 3.9+ and no extra packages:

```sh
python3 experiments/system-one-routing-paper/supplements/decision-interface-transfer/replay.py verify
python3 experiments/system-one-routing-paper/supplements/decision-interface-transfer/replay.py report --output /tmp/hiroute-decision-transfer-replay
```

Use a new output directory. Replay checks hashes, bilingual exclusions, the full
152 × 3 backends × 2 forms request matrix, unchanged input states and exact question
definitions. It parses all saved native responses, applies the continuous-score
rule `s < 0.5 OR (h < 1 AND a >= 0.5)`, then recomputes both populations. It checks
the resulting metrics against the saved reference; saved aggregates are not the
source of decisions. `MANIFEST.json` also pins the parent parser dependencies.

## Evidence and limits

Every condition has one response. Translations and state variants share workflow
families; paired intervals resample whole families. These descriptive intervals
do not account for post-hoc selection. The mathematical panel deliberately tests
dependency status: every current task is routine and every assessed stage has
grade 0. It does not evaluate general mathematical difficulty or proof quality.

This measures adherence to a declared routing policy, not task completion,
production traffic accuracy or realized cost savings. Questions were developed
through earlier Jev experiments; OpenAI received an unchanged transfer, with no
backend-specific optimization. Neither direct encoding is asserted optimal.

`requests.jsonl.gz` and `responses.jsonl.gz` retain full semantic bodies and
answers. Remote IDs, access-service namespaces, provider metadata, operational
timestamps and workstation settings are omitted; evaluated model versions,
answer distributions, native scalars, confidence and usage remain. The Jev
returned versions are `jev-1.13-20260917` and `jev-1.14-20261014`.

Across all 304 formal OpenAI requests, recorded input usage was 632,820 tokens:
USD 0.063282 at the recorded input rate, with no charged output tokens. This is a
usage estimate, not an invoice. The 152 three-question calls imply approximately
USD 0.183 per 1,000 requests, versus API-reported USD 0.085 for the 152 matched Jev
1.14 three-question calls. These costs use the full executed population, not an
imagined rerun of the reviewed subset, and cover only decision calls. Original
campaign hashes and execution dates are in [provenance.json](provenance.json).
