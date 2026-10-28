# Mathematics: historical protection and release

This supplement reproduces the mathematics follow-ups in manuscript Sections
5.8–5.9. It adds three **separately scored** panels; it does not replace the original
mathematics main table or its labels.

| Panel | Bilingual records | Workflow families | Repeats per cell | Calls | Exact-action coverage |
|---|---:|---:|---:|---:|---:|
| Independent confirmation | 136 | 16 | 2 | 2,176 | 136/136 |
| Targeted dependency states | 48 | 6 | 1 | 384 | 48/48 |
| Chronological conversations | 44 | 22 | 1 | 352 | 32/44 |

Every record is evaluated with Jev `jev-1.13-20260917` and
`jev-1.14-20261014`, each under the **existing** and **concise** definitions,
with a matched direct selector and three-question form. Both forms receive the
same context. Each direct question embeds that definition set's exact component
definitions and composition rule. The three answers are returned in one call.

## Reproduce offline

From this directory, with Python 3.9 or later and the surrounding public package:

```sh
python3 replay.py verify
python3 replay.py report --check --output .checks/replay
python3 -m unittest discover -s tests -v
```

The output directory must be new. `verify` checks the manifest, public parser
dependencies, full request factorial, input-to-request context equality, question
identity, response accounting and **1,024 reference assertions**. It reparses all
2,912 retained responses; reference aggregates are never used to produce scores.
No network access, credentials or third-party Python packages are needed. The
manifest pins the required files in the parent package's `analysis/` and
`policies/` directories, so copying this subdirectory alone is insufficient.

`report` writes `results.json`, `results.md`, and `parsed-predictions.jsonl`.
The JSON includes per-panel/model/definition/form results, family-weighted
intervals, state and language breakdowns, component agreement, protection and
release subsets, unforced actions, scalar sensitivity, usage and validity.
The overall execution count and cost are accounting totals, not pooled accuracy.

Inspect a complete published example, including its label and saved answers:

```sh
python3 replay.py case --panel targeted --id 11a5d305c8b581a6069c
```

That Chinese R17-manifest case has an unexecuted repair promise. Under existing
Jev 1.14 definitions its three-question values are `s=0.98, h=0.07, a=0.94`
(protect), while matched direct chooses the standard tier. The completed-repair
counterpart is `8dad1ccafea55e4648cf` (`s=1.00, h=0.02, a=0.23`, release).
The empty-condition-card boundary example is `a8d99b895d21d96513b2`: concise
applicability is 0.49 in 1.13 and 0.53 in 1.14. These are inspection examples,
not extra observations or separate experiments.

## Reading the results

The independent confirmation supports improved protection in Jev 1.14; its
concise 1.13 direct comparator is already near ceiling and the three-question
form has four release errors. The targeted cohort shows gains for both versions
with either definition set. Its existing and concise three-question totals are
the same, although their matched direct comparators differ. The chronological
cohort has 32/32 for both concise forms and supports retention on that
representation, not an additional advantage over direct selection.

The 48 targeted records deliberately emphasize dependency distinctions and were
authored with knowledge of earlier results. The independent 136-record author
reviewed its own material. A separate AI reviewer anonymously labeled all 92
extension records before author-label reconciliation and evaluated calls. None
of these follow-ups has expert-human label validation. All three panels are
synthetic; translations, states and repeats are correlated. See the
[data card](DATA-CARD.md) for the uncertainty and construction boundaries.

## Contents and disclosure

- `datasets/`: complete conversations and final pre-call labels, joined by `id`.
- `questions/`: exact existing/concise definitions and matched direct questions.
- `execution/requests.jsonl.gz`: complete semantic requests and logical call IDs.
- `responses/answers.jsonl.gz`: original answer objects, exact Jev version,
  status, attempt index and reported token/cost usage for every call.
- `reviews/`: author labels, anonymous first labels and reconciliation evidence;
  [review guide](reviews/README.md) distinguishes historical labels from scoring gold.
- `reference-results.json`: assertions taken from the preexisting source results.
- `provenance/source-hashes.json`: original-source digests with logical locations.
- `MANIFEST.json`: hashes of the public files and required public parser code.

Request and response model namespaces are removed while preserving Jev versions.
All answer content is retained, including probabilities, returned scalars or
choices, confidence and score legends. Remote IDs, access-service identifiers,
credentials, host configuration, timestamps and operational logs are omitted.
Logical call IDs and original planned order still connect every request and
answer. This supports offline numerical reproduction, not exact regeneration of
hosted execution or the private annotation process. No latency is claimed for
these follow-ups.

All 2,912 calls returned HTTP 200, parsed successfully and required no retries.
The saved usage totals are 6,849,208 input tokens, 151,052 output tokens and
USD 0.287666736 (confirmation 0.217922208; targeted 0.036224832;
chronological 0.033519696). These are observed decision-call charges, not current
prices, executor costs or measured savings.
