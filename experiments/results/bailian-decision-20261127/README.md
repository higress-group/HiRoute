# Bailian System One on the native decision transfer study

On 27 November 2026, the Beijing Model Studio workspace returned a valid answer for all 304 `decision-model` calls and all 304 `decision-model-preview` calls. The stable model matched both Jev versions on engineering three-question action accuracy in the full panel. On the mathematical dependency panel it missed protection in 14 of 24 cases requiring the enhanced group. It selected the standard group correctly in all 24 release cases.

## Matched conditions

The [transfer study](../../system-one-routing-paper/supplements/decision-interface-transfer/README.md) supplies the public synthetic contexts, original labels, exact question definitions, historical responses and evaluator. Each of 152 Chinese/English records has one complete-policy direct request and one three-question request per model. The engineering panel covers 104 records from 26 workflow families, of which 86 have determinate action labels. The mathematics panel covers 48 determinate dependency-state records from six families. This measures final group selection under the study policy. Executor task completion and savings were outside the study.

Both Bailian arms used the archived Jev request body with only `model` changed, via `POST /compatible-mode/v1/systemone` on an existing Beijing workspace. A fixed shuffle (seed 20261127) interleaved models and forms, with at most four concurrent requests, a fresh TLS connection per call and zero retries. The frozen evaluator uses the returned continuous history score. It selects the enhanced group when `p_simple < 0.5` or when `history_score < 1` and `p_applicable >= 0.5`. Conditional and insufficient labels remain in coverage counts. Each eligible workflow family has equal weight.

The Jev responses were collected on 28 October through OpenRouter's native `/api/alpha/decisions` endpoint. Their returned versions are `jev-1.13-20260917` and `jev-1.14-20261014`. The OpenAI responses were collected on 29 October from `/v1/decisions` with `gpt-6-luna`. That adapter expresses identical state and question definitions in OpenAI's named-question arrays. The historical run and both Bailian arms each have one observation per model, form and record. All 1,520 formal responses were valid. The Bailian IDs are provider aliases observed on the collection date.

## Final group accuracy

Percentages are family-macro accuracy, shown in direct / three-question order. The replay prints correct and eligible record counts alongside every percentage.

| Backend | Full engineering | Full mathematics | Reviewed engineering | Reviewed mathematics |
|---|---:|---:|---:|---:|
| Bailian stable | 75.00 / 96.15 | 52.08 / 70.83 | 72.12 / 94.23 | 41.67 / 64.58 |
| Bailian preview | 50.00 / 79.81 | 54.17 / 52.08 | 51.92 / 77.88 | 49.31 / 41.67 |
| OpenAI Decisions | 48.08 / 89.42 | 56.25 / 85.42 | 50.00 / 91.35 | 65.97 / 88.89 |
| Jev 1.13 | 90.38 / 96.15 | 87.50 / 97.92 | 89.42 / 96.15 | 90.97 / 100.00 |
| Jev 1.14 | 69.23 / 96.15 | 62.50 / 97.92 | 67.31 / 96.15 | 54.86 / 100.00 |

The historical post-hoc review excluded the same 14 disputed records for all five models. It retained 98 engineering records (80 determinate) and 40 mathematics records. The November experiment inherited those exclusions before calling Bailian. The primary full-population table preserves all original records. In the full panel, Bailian stable answered 82/86 determinate engineering actions and 34/48 mathematical actions correctly with three questions. Four engineering protection misses account for all its engineering errors.

## Operational measurements

Each table cell covers all 152 records for its respective form. Input token counts and client-observed median latency in milliseconds are shown in direct / three-question order. The replay also prints nearest-rank p95 and output token coverage. Transport, calendar date and network conditions differ between campaigns.

| Backend | Input tokens | Median latency (ms) | Failed / attempted | Cost across both forms |
|---|---:|---:|---:|---|
| Bailian stable | 358,029 / 320,207 | 318 / 369 | 0 / 304 | CNY 0.203471 at the Beijing input rate |
| Bailian preview | 342,930 / 236,938 | 317 / 289 | 0 / 304 | Beijing's published limited-time free offer |
| OpenAI Decisions | 354,662 / 278,158 | 1,010 / 956 | 0 / 304 | USD 0.063282 at the recorded uncached input rate |
| Jev 1.13 | 409,819 / 306,107 | 1,325 / 1,289 | 0 / 304 | USD 0.030069 API-reported |
| Jev 1.14 | 409,819 / 306,107 | 1,199 / 1,193 | 0 / 304 | USD 0.030069 API-reported |

Bailian returned input usage only. Estimates use CNY 0.30 per million stable-model input tokens in Beijing. No invoice or account-specific discount was available. OpenAI reported zero output tokens and zero cache reads/writes in this campaign. The Jev charges come from each response's `usage.cost`. Original event-ledger digests for the historical latency measurements are in the [study provenance](../../system-one-routing-paper/supplements/decision-interface-transfer/provenance.json). [Stable pricing](https://help.aliyun.com/en/model-studio/decision-model-stable) · [Preview pricing](https://help.aliyun.com/en/model-studio/decision-model-preview)

### Two separate provider probes

Using the same workspace, a stable System One request with the repository's desktop icon in a top-level `image.url` PNG data URL returned `logo` with probability 0.999908 (1,129 input tokens, HTTP 200). Another request with 80,000 repetitions of `界` in `state` and a `noul` question about repetition returned P(yes) 0.992608 (80,080 input tokens, HTTP 200). Including these two calls, stable usage was 759,445 input tokens, estimated CNY 0.227834. Neither probe enters the scored panels. The provider endpoint was exercised above the preview limit. The documented 268,288-token boundary was not exercised.

HiRoute's routing adapter renders an ordinary chat image as an unavailable marker. Its Gateway regression exercises the stable model ID, text, keyed questions and the marker on the encoded wire. The two direct API probes do not establish image forwarding or measured token capacity through HiRoute. [System One API](https://help.aliyun.com/en/model-studio/decision-model-api)

## Offline reproduction

From the repository root with Python 3.9 or newer, run `python3 experiments/results/bailian-decision-20261127/compare.py`. It verifies the historical transfer manifest, both supplementary ledger digests, every Bailian request hash against the original frozen state/questions, and the complete five-model matrix. It parses the saved answers with the original native evaluator. No credentials or network access are needed. The compressed Bailian ledger retains the semantic answers, status, usage, elapsed time and request digest. It omits service request IDs and duplicated prompt text.
