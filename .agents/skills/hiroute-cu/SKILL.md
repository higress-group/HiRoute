---
name: hiroute-cu
description: Plan and run HiRoute real-user Computer Use acceptance, fast smoke regression, and targeted exploration of changed or immature features; verify real business effects and distill stable paths into reusable batch CU. Use Jev for bounded read-only exploration and suspected-issue screening, not ordinary implementation or API-only tests.
---

# HiRoute Computer Use scenarios

Codex owns goals, business judgments, writes and acceptance. Run stable paths as
batch CU. Use Jev to explore unfamiliar read-only segments and screen suspected
issues, then have Codex verify and extract reusable steps. Jev is an explicit
tool, never the default execution layer or an automatic fallback.

1. Read [the index](index.md), then only the selected scenario and missing prerequisites.
2. Read [orchestration](references/orchestration.md) to choose planning, smoke,
   targeted exploration, reproduction or real E2E. Run mature segments first;
   exercise writes and actual effects for immature business paths.
3. For unfamiliar read-only navigation, read [Jev use](references/jev.md).
   Refine the goal or candidates after observing a failed attempt. Codex takes
   over after repeated substantive failure, an exhausted budget or a write boundary.
4. Use [the runtime](references/runtime.md) for stable batch execution,
   [exploration](references/exploration.md) for extraction and
   [takeover](references/takeover.md) for failures. Recheck the original failure
   after an authorized fix; never blindly replay writes.
5. Keep newly found issues as local evidence first. Create an independently
   runnable [regression case](references/cases.md) only after user confirmation;
   do not automatically add it to smoke or CI.

Preserve full raw traces locally, reading selected portions as needed. Do not
commit them, publish them or send the full history to Jev. Only its bounded
current projection is external input. There is no general rollback or automatic
inverse-script generation.

A planning request authorizes planning only. Follow current user authorization,
host tool policies and `AGENTS.md`; do not delegate, merge or publish automatically.
Maturity is separate from this candidate's result: navigation, save, actual calls
and completed tasks each need their own evidence.

For affected UI acceptance, use the [shared interaction guidelines](../../../docs/desktop-interactions.md):
compare equivalent capabilities, verify Cancel/Back and recovery without unintended
effects, and check focus, reachable actions and draft retention in the actual host.
Keep source-bound before/after screenshots with matching state and display settings.
Record intentional host differences from real capabilities. Fixture appearance
does not establish native authorization, persistence or a completed provider call.
