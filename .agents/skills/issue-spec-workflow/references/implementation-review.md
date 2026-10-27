# Implementation Review and Publication

Read for requested review or publication, not ordinary planning or implementation. Load the project's `guidance.review` references from `issue-spec/config.yaml` when configured. A project may place review at a stage boundary under a coordinator; issue-spec does not prescribe that role.

The current owner implements and self-tests, then reports the exact base/candidate, checks, gaps and review needs. Do not automatically spawn a reviewer or require a PR/MR. Self-tests are not independent review. Perform the review the user requested, within the host's capabilities and project ownership rules.

Review against the actual candidate and inspect existing evidence before rerunning checks. An actionable finding names a concrete failure path or violated invariant. Send repairs to the responsible owner, then check the changed candidate and affected regressions; unchanged evidence retains its original revision. Do not claim an independent reviewer was used when it was not.

For changed user-facing critical paths, check that each added wait is necessary, unrelated state growth does not add heavyweight work, and completion evidence reaches the point when the user can proceed.

Publication, push, approval and merge require actual authorization. If provider comments are requested, validate anchors against the published candidate and preserve the writer's authorship. Report failed writes with the rendered content for recovery. Explanations should cover non-obvious decisions, with no quota or mandatory rationale ceremony.

Findings and handoff are ordinary review context. Do not create typed REVIEW/VERIFY, readiness receipts, rationale evidence, finalization or Archive gates. They never certify provider mergeability. Follow required provider checks and project gates before an authorized merge.
