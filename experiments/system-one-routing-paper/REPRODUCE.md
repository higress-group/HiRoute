# Offline reproduction and evaluation

Python 3.9+ and its standard library are sufficient. No package installation,
provider credentials, internet access or paid calls are needed. The release was
checked with Python 3.9.6. `reproduce.py` has no network execution command.

From the repository root:

```sh
python3 experiments/system-one-routing-paper/reproduce.py verify
python3 experiments/system-one-routing-paper/reproduce.py report --check --output /tmp/hiroute-paper-replay
python3 -m unittest discover -s experiments/system-one-routing-paper/tests -v
```

Use a fresh output directory. `verify` checks the published file hashes, validates
all three paired bilingual datasets and validates their evidence spans. The report
parses saved native responses and complete LLM output, re-applies the unchanged
continuous-score rule, then scores the planned population. It does not use cached
prediction labels as the source of model decisions. Missing or invalid calls stay
in denominators; boundary and insufficient actions remain unforced.

The command produces `report.json`, readable `RESULTS.md` and parsed prediction
archives for every campaign. `--check` separately compares the recomputed reductions
with `reference-results.json`; that file does not supply parsing or scoring inputs.
It also checks the separate MR010_b sensitivity. A mismatch fails the command.

## Connection timing diagnostic

The existing comparison table in Appendix C is reduced from curated numeric records:

```sh
python3 experiments/system-one-routing-paper/diagnostics/connection_timing.py
python3 experiments/system-one-routing-paper/diagnostics/connection_timing.py --json
```

The first command reproduces the six-row setup-excluded latency table. The second
prints all recorded timing cells and their median connection durations. The
reducer excludes warmups, checks the original matched request pairs and the shared
ten-context sample, and subtracts each call's own connection duration before
aggregation. It contains no network execution. Qwen's 20 measured calls and two
warmups complete the same-context comparison. The original 256-call end-to-end
latency cells remain reproducible through `report`; they are a separate metric.

## What is reproduced

Sections 5.8–5.9 have a separate [mathematical supplement](supplements/mathematics-history/README.md)
with its own manifest, semantic response archive and offline replay. It reports
the 136-record independent panel, 48-record dependency-state panel and 44-record
chronological panel separately. Follow its commands to reproduce those tables;
the main `report` command below retains the original main-study results.

- Matched engineering direct/three actions, component scores, cohorts, language
  results, and paired workflow intervals.
- Independent engineering three/two/legacy comparisons, separate original,
  coverage-supplement and paraphrase panels, same-response gate contributions,
  and expression-pair stability.
- Mathematical explicit-direct/three actions and components, fixed-response
  current-only and ungated ablations, applicability error partition, common-cohort
  analysis, boundary actions and MR010_b sensitivity.
- Native direct expression controls, diagnostic probes, V3 development results,
  API usage charges and client-observed latency.
- The separate format-development ledgers, including the rejected request and
  the successful format checks; no semantic headline score is attached to them.
- The abandoned engineering campaign's complete plan, starts/results, four unknown
  outcomes, invalid result and not-started calls, kept separate from completed data.

Intervals retain the original conventions: engineering uses the frozen paired
cluster bootstrap with interpolated quantiles; mathematics and the expression
supplement use sorted workflow means and order statistics. Seed 20261018 and
10,000 draws are fixed. Resampling units are whole workflows, not individual
translations or responses. Intervals describe this constructed sample.

The original V3 archive includes both 0.5 and 0.8 direct-LLM configurations. They
are named separately (`llm_direct@0.5` and `llm_direct@0.8`), not pooled or selected
post hoc. Native paper replay uses simple threshold 0.5. Preliminary and abandoned
conditions are not replacements for the matched main comparisons.

## Inspect a complete case

```sh
python3 experiments/system-one-routing-paper/reproduce.py case --dataset mathematics --id MR006_b_zh
```

This prints its full context, original standard answers and parsed outputs by
campaign/model/format/repeat. Exact request and response bodies can be inspected
from the corresponding `execution/*/requests.jsonl.gz` and `responses/*.jsonl.gz`
using any gzip reader. JSONL contains one event or request per line. Provider
identifiers unrelated to scoring were removed as described in the data card;
semantic completions and failed outputs remain.

## Evaluate another router

The manuscript uses **standard tier** and **enhanced tier**. JSONL action values
remain `economy` and `primary`, respectively, so saved predictions and frozen
experiments keep their original identifiers. The generic action track accepts:

```json
{"id":"IG001_a_en","repeat":0,"valid":true,"action":"primary"}
```

Include every planned response and set `valid=false` for malformed/failed output.
Use repeats starting at zero. Omitted predictions are counted as failures;
unexpected IDs, duplicates and out-of-range repeats are rejected. Do not feed
labels or rationale to the router. The exact evaluated prompts are under `policies/`,
and the archived sent bodies show the complete context and output adapters.

```sh
python3 experiments/system-one-routing-paper/reproduce.py score --dataset engineering --predictions /tmp/my-router.jsonl --repeats 1
```

Generic scores report the primary and paraphrase panels separately; they never
pool them into a headline score. Component answers are optional in this track.

For the typed track pass `--typed` and add `current_demand`, `stage_grade`, `applicability` and
`metric_valid` booleans for each component. The typed command reports unique-label
component scores and final actions with their respective coverage. Native Jev
responses can use the public frozen parser in `analysis/fair_methods.py`; numerical
LLM adapters must preserve their reported scalar rather than substituting a modal
grade. Evaluate independent policy changes in new result files, without overwriting
these frozen datasets or published returns.

## Boundaries of this release

The release reproduces machine-result arithmetic and exposes synthetic labeling
and anonymous audit evidence. Private human votes and undisclosed author/reviewer
model identities limit regeneration of the annotation process. Controlled product
checks are summarized and linked to the pinned source; running the offline scorer
is not a new product-path test or executor-quality experiment.
