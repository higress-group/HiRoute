# Unattended HTTPX engineering and automatic model handoff

The task adds synchronous/asynchronous streaming JSON iteration to pinned HTTPX
`b5addb64f0161ff6bfe94c124ef76f6a1fba5254`. The task comes from DeepSWE's
`datacurve/httpx-streaming-json-iteration` at source revision
`0b9fabbb63b9104d678fe965e1632f2dd9eaa2ea`. Task-derived test fixtures retain their
[Apache-2.0 license](fixtures/LICENSE); the archived HTTPX source patch retains the upstream
[BSD license notice](fixtures/HTTPX-LICENSE.md). The initial prompt was adapted to request both interfaces in one run and
explicit incrementality. No upstream contribution is implied.

## Published result

One initial task; no intermediate operator prompt or product patch; 1896.015 seconds.
Actual requests: Qwen 25, Astra 15. After the second native compaction, the stage score
was 0.485 against a 0.5 floor, followed by 13 actual Astra tool calls. The first upgrade
only produced a summary; it is not counted as an implemented repair. Native compaction
timestamps plus model/tool receipts establish the boundary attribution, not an invented
explicit ContextHold receipt flag.

Independent acceptance: 108 feature + 229 existing regression + 6 incrementality
assertions = **343 passed**. Protected tests were unchanged. Native validation separately
reported 487 passes with three predeclared transport exclusions; do not add these to 343.

[Final assessment](results/2026-10-04/assessment.json) ·
[Execution evidence archive](https://github.com/higress-group/HiRoute/releases/tag/experiment-evidence-2026-10-04) ·
[Actual model-produced source patch](results/2026-10-04/model.patch) ·
[Task](TASK.md) · [Frozen parameters](case.json)

The Release archive contains `results/2026-10-04/evidence.json` and
`fixtures/config.json` for this case. Run `python3 experiments/reproduce.py fetch`
and `python3 experiments/reproduce.py unpack` to inspect their original bytes.

This is one historical case, not a guarantee that all long tasks converge. There is
no all-Astra cost control, so **no savings claim**. The earlier aborted formal attempt
and environment adjustment remain recorded in the public evidence. Reproduction code
is a portable adaptation, not the original Linux sandbox implementation.

## Prepare a fresh subject

```sh
python3 experiments/cases/unattended-engineering/prepare.py --output /tmp/hiroute-httpx-subject
```

Use Python 3.14.6 and the baseline requirements for the original setup. The original
benchmark's [environment recipe](fixtures/environment.Dockerfile) is retained for
provenance, but its `latest` base image is not immutable and is not a byte-reproducible
environment guarantee. Record resolved dependency versions for every new run. A prebuilt
benchmark image identifier is in the task lineage; no image is executed by our tools.

For an **offline code-verification replay**, apply the archived model patch to the newly
prepared subject, then run the independent checker. This performs no model calls and
cannot reproduce routing behavior:

```sh
git -C /tmp/hiroute-httpx-subject apply /absolute/path/to/HiRoute/experiments/cases/unattended-engineering/results/2026-10-04/model.patch
python3 experiments/cases/unattended-engineering/verify.py \
  --subject /tmp/hiroute-httpx-subject --python /path/to/prepared/venv/bin/python \
  --output /tmp/hiroute-httpx-verification
```

The checker independently reconstructs baseline test bytes, checks protected files,
uses a disposable copy, confirms local HTTPX imports, and requires all three exact
nonzero assertion counts. It does not change the submitted implementation. Run it in an
isolated environment with no provider credentials and no network; its script alone is
not an operating-system sandbox.

## Fresh autonomous execution

Prepare another untouched subject; do **not** apply the archived model patch. Connect
native Codex 0.160.0 to an isolated HiRoute smart-saving plan. Use the supplied research
Jev criteria with the distinct economy-first policy in `case.json`: simple threshold 0,
competence floor 0.5, Qwen xhigh, Astra medium, 131072 context tokens, 60000 automatic
compaction threshold, 65536 output cap, and a 5400-second outer guard. Request and attempt
budgets are recorded separately. Use normally configured authentication; never duplicate
a subscription refresh token or let the subject read evaluation fixtures/results.

Give the agent the contents of `TASK.md` once, with the prepared dependencies and visible
acceptance tests. Let it implement, test, and repair without intermediate operator
feedback. Freeze its completed source and run the independent checker above. A code pass
alone does not prove automatic routing: preserve native compaction timestamps, HiRoute
route/model receipts, Jev assessments, actual subsequent tool execution, and the absence
of intermediate prompts. Manual intervention or a model override creates a different case.
