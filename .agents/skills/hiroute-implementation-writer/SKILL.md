---
name: hiroute-implementation-writer
description: "Own one assigned HiRoute Proposal/change in its current session: coordinate authorized planning, implement and self-test in its worktree, and repair stage-review findings. Use for an assigned change; not ordinary unscoped edits, project coordination or final review."
---

# HiRoute Proposal Owner

Own the assigned change from authorized planning through implementation and self-tests in this session. Do not create other sessions or subagents unless the user explicitly requests delegation. A project coordinator is optional: use that handoff only when the user has designated one for a coordinated stage; otherwise report to the user.

This skill can be selected automatically for an assigned HiRoute Proposal/change in Codex or Claude Code; no explicit skill mention or role marker is required. Selection does not assign a new change or authorize another phase. Use the current session and checkout. Report cross-session handoffs in the conversation when no authorized messaging capability is available; do not require Codex app tools in Claude.

## Establish the assignment

- Confirm the assigned Proposal, stage objective/non-goals, authorized phase, worktree/branch/base, ownership, shared interfaces and required checks. Recover omitted factual metadata with bounded reads. Pause only the affected work for a real contract or authority conflict; routine implementation choices are yours.
- A Proposal-only assignment ends at Proposal handoff; a Design-only assignment ends at Design handoff. This role does not extend permission to implement, push or start another stage. Follow later authorization in the same thread.
- Preserve the same Proposal/task/worktree across authorized model changes. A planning-only assignment stops at its requested handoff. In a coordinated stage, honor the agreed alignment checkpoint; an already authorized end-to-end assignment can continue to implementation. Changing model settings alone is not permission to extend the scope. Do not independently choose or switch models.
- Read only the skills needed now: `$hiroute-design-author` and `$issue-spec-propose` for authoring, `$issue-spec-apply` for implementation. Do not load project-coordinator or stage-review instructions to self-certify the change.
- For implementation, read the complete selected Issue or current Design, effective answers, selected repository durable specs and relevant code/tests. Do not depend on external design archives; required behavior must already be recorded in the selected Design or repository durable specs. Use this worktree's versioned skills; if the assigned policy revision is missing or stale, request a scoped update rather than loading an unrelated checkout's policy.
- Follow the bounded-read procedure in `$issue-spec-workflow`: read the shared Design and applicable decisions, list TASKs, then expand the current TASK and required dependencies/SPECs. Execute its sequence and acceptance without loading unrelated task bodies. Check actual code against the supplied starting evidence; a detailed plan is not proof that an entrypoint or assumption is correct.

## Coordinate and implement this change

- Maintain this change's authorized issue-spec Proposal/Design/TASK/Implement/PROCESS records and blocking QUESTION/ANSWER decisions through the configured CLI. Creating a Design or TASK does not require handing implementation to a second agent. Do not create PROCESS merely to describe your own single-writer work or a review.
- Keep the work aligned with the assigned MVP outcome. Implement the smallest sound architecture that satisfies the contract; do not add speculative platforms, blanket abstractions or unsupported compatibility paths. Necessary responsibility boundaries are not optional polish.
- For resource/performance work, apply the [serviceability rule](../../../AGENTS.md): reuse backing and existing streaming/spill paths, isolate optional work from required request admission, and preserve ownership and final release. Required state that cannot spill must not be refused solely to meet an internal memory target. Account for synchronous fallback CPU/I/O as foreground work; an asynchronous entrypoint name does not establish isolation.
- Edit only the assigned worktree and owned paths. Shared manifests, lockfiles and registries follow the agreed branch-local intent/convergence protocol. Do not edit peer worktrees, absorb another Proposal's work, merge to the target branch or silently change shared interfaces.
- In coordinated work, keep consumed dependency revisions fixed until the designated coordinator authorizes an exact update; send cross-Proposal needs to that coordinator in one concise batch. For a standalone change, resolve dependencies within the user's scope and record consequential decisions in its Issue.
- Aim for an independently deliverable owned module. Use the agreed interface and contract-level test doubles for unavailable peer implementations; preserve fail-closed behavior and report the missing production wiring. Do not invent a replacement public contract, ship a test double as a real backend, or weaken acceptance to finish independently.
- Amend an inaccurate Design within your authoring authority before implementing a changed contract. Escalate changes outside the authorized product scope, shared interfaces or acceptance to the designated coordinator or user. Do not demand another decision for a settled choice or invent issue-spec workflow gates.
- If a supposedly mechanical step requires unresolved architectural research, report the concrete mismatch, impact and smallest proposed resolution. Continue unaffected work; do not compensate with speculative frameworks, permissive compatibility layers or weaker tests. Private helper choices remain yours.
- Finish the behavior and fix introduced failures. Use [HiRoute validation](../hiroute-validation/SKILL.md) for selected checks, configured execution, failure repair, generators and final evidence. Preserve explicit product/native gates and the distinction between fixtures and the real Client Core/CLI/Daemon/Desktop entry.
- Organize Rust by responsibility. Treat 700 lines as a soft review threshold, not a hard cap; split only for a clear ownership boundary, never through low-value fragments or distorted logic. Split candidate-introduced files above 1100 lines; exact-base oversized files remain with their owning change. Preserve the [existing size gate](../../../scripts/check-rust-file-size.sh).
- Self-inspection and tests are normal implementation work. They are not the independent stage review and cannot produce a final “approved/merged” verdict.

## Retire superseded build artifacts

Use the owner-cleanup route in [HiRoute validation](../hiroute-validation/SKILL.md)
before another local Rust build and at handoff. Decide which owned outputs are
still needed; record retained run IDs, purpose and end condition in the Issue/PR.
Uncertain ownership or a busy cleanup lock leaves the output retained while
unaffected work continues. Cleanup adds no approval round or source-worktree deletion.

## Handoff and repair

- Own the durable SPEC update for this change, not just code and self-tests. Before handoff, follow [Durable SPEC maintenance](references/durable-spec.md); report affected scenario files, relevant tests and any unresolved spec/implementation mismatch. First-pass SPEC authoring is yours. Human PR review is the default; stage review applies only in explicitly coordinated work.

- Commit/push only as authorized, preserving unrelated work; satisfy DCO when required. Never delete your source worktree as part of handoff; retire unused build outputs as described above.
- Return a concise handoff to the user, or “ready for stage review” in explicitly coordinated work: Proposal/Design links; base and exact candidate commit (or explicit uncommitted state); worktree/branch; outcome and changed responsibilities; checks with exit/scenario results and tested revision; known gaps/base failures; shared-file integration notes. No sealed role receipt, forced digest catalog or filler rationale is needed.
- Record completed work or actionable blockers in the corresponding Issue/PR and report to the user or designated coordinator. Stop at the requested handoff; continue only within existing phase authorization. Do not launch a reviewer automatically.
- In explicitly coordinated work, at the independent-delivery checkpoint, include the consumed dependency SHAs, produced interface changes, shared-file intent and pending integration gaps in the normal handoff, then pause the delivered scope. Do not independently merge peers to complete central integration. Escalate immediately only if a contract conflict would invalidate implementation, there is a concrete safety/data-loss issue, or no useful independent work remains; continue unaffected work otherwise. No acknowledgement-only replies are needed.
- Fix findings sent to this thread, test the affected behavior and required gates, and return the new head plus addressed findings. Ask for clarification when a finding conflicts with the agreed MVP contract; do not silently widen scope or bypass a failing test.
