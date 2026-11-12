# Higress engineering routing experiments

The experiments used two public Higress repairs to compare fixed-model execution
with staged routing. The streaming tool-call repair was a pilot. The concurrent
request-state repair was the formal comparison. The two tasks have separate
acceptance records. [Higress #5209](https://github.com/higress-group/higress/pull/5209) ·
[Higress #5311](https://github.com/higress-group/higress/pull/5311)

## Formal comparison on #5311

All nine runs started at Higress commit
`bfcd87d1a22fedae9efaa1d901caadd1a9ffcf6d`. The public reference was
`7dc1162c4629d437bc518a3e24b42c33250d50d3`. Each arm had three runs and
the same 45-minute budget for the agent and its tools, with no model-turn
limit. The execution models used the recorded `xhigh` setting. The
[CSV](formal-runs.csv) preserves the frozen run order.

All arms used the same reproduction, core repair and regression stages.
Independent acceptance included Go and Wasm checks and interleaved requests
through the direct Claude and Vertex Gateway paths. Acceptance required the
sealed candidate to satisfy the same checks across arms. For the native
HiRoute arm, GPT-6 Astra handled reproduction and the core repair, and
Qwen3.8 Flash added regressions. Each native run recorded one switch in the
same product session and Plan revision, with no fallback.

| Arm | Accepted deliveries | Total API-equivalent cost | Cost per accepted delivery | Mean delivery time |
| --- | ---: | ---: | ---: | ---: |
| All Qwen | 1/3 | ≥ $0.5989 | ≥ $0.5989 | 51 min 34 s |
| All Astra | 3/3 | $19.824 | $6.608 | 17 min 33 s |
| Native HiRoute | 3/3 | $7.863 | $2.621 | 38 min 15 s |

Native HiRoute used 60.3% less API-equivalent cost per accepted delivery
than all Astra in these runs. Both arms delivered three accepted repairs.
The native runs took longer on average. The comparison covers one task with
three repeats per arm.

## Read the run data

- Sum `api_equivalent_usd` across the arm and divide by its number of
  `complete_acceptance` rows. The costs use observed upstream attempts and
  public rates recorded on November 10, 2026. Native runs include the GPT-6
  Luna decision cost. Actual billed amounts are unknown.
- A blank `api_equivalent_usd` leaves the total unknown. Use
  `known_cost_lower_bound_usd` to sum the covered portion. Q33 was cancelled
  at the cutoff and has final usage for 100 of 101 attempts.
- Average `effective_delivery_seconds` for accepted rows. Q32's sealed
  cutoff candidate passed the same independent gate after the run, without
  another model request. Its effective delivery time includes the measured
  acceptance work. The later acceptance timestamp also includes the wait
  before grading.
- Treat `first_patch_seconds` for a `not_ready` row as cutoff bookkeeping.
  Compare handoff-ready patch time using the rows with a sealed candidate.
  Earlier qualification and pre-protocol runs are outside this nine-run
  dataset.

## Pilot on #5209

The two time-boxed pilot runs started at Higress commit
`43ad04ee54139935e9b51fbebec19986fb9f8508` and used
`f16adbc79a943da2622ec99e8084ee5500ca5c51` as the public reference.
Qwen sealed a candidate after 826.466 seconds and Astra after 940.604
seconds. Both candidates fell short of complete independent acceptance.
The pilot yielded no pair of accepted deliveries for a cost comparison.
