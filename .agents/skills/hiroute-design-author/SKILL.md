---
name: hiroute-design-author
description: "Author HiRoute issue-spec Proposals and Designs using scoped product references. Not for ordinary research, document review, or implementation."
---

# HiRoute Design Author

Use with `issue-spec-propose` only while creating or amending a HiRoute Proposal, SPEC, QUESTION, ANSWER, Design, or TASK. Follow the user's current model/effort selection; do not override an existing task's settings. Assess implementation complexity after inspecting the code and establishing the Design.

The assigned Proposal thread owns its authorized planning and later implementation in the same worktree. Do not create implementation or review subagents merely because a Design/TASK exists. Human PR review is the default; a designated coordinator performs stage review only in explicitly coordinated work; authoring/self-checks do not replace it.

## Reference policy

- Use current code, related issue-spec Issues, and repository durable specs as the design inputs. Before defining Proposal SPECs, follow the [code map](../../../docs/code-map/README.md) to the affected production owner and a representative scenario. Apply the shared [requirement scope guide](../issue-spec-workflow/references/planning-scope.md); identify HiRoute capabilities to reuse and the integration or behavior gaps this change must close.
- Preserve applicable shared configuration, routing, security and recovery contracts as constraints of the affected capability. Use their owning specs and the compatibility register where applicable; do not turn unchanged platform contracts into independent feature deliverables.
- For resource/performance changes, apply the [serviceability rule](../../../AGENTS.md). Identify required request state, reusable or spillable backing, optional work, ownership/lifetime and pressure behavior. State the baseline and verification plan for memory, CPU and complete local foreground latency, including synchronous fallback. Justify additional machinery with measured bottlenecks; label unmeasured estimates and keep implementation thresholds in their owning code/spec.
- When those sources lack required intent, investigate available repository evidence and record the missing decision self-contained in the selected Proposal or Design.
- Treat historical Issues as context, not fixed execution itineraries. Choose the architecture from current requirements and code without carrying obsolete scope forward.
- Make the issue-native Proposal/SPEC/Design/TASK chain self-contained for product behavior, shared interfaces, invariants, failure behavior, decisions, and acceptance. Private helpers and internal layouts need not be predetermined unless they protect an invariant or another package depends on them. A reader with only the repository and selected Issues must be able to review and implement the Design.
- Resolve actual contract conflicts through typed QUESTION decisions or a Design amendment. Investigate available evidence and decide routine design details within the confirmed scope, recording non-obvious rationale. Ask for a decision when it changes product behavior, authority, shared contracts, or acceptance; do not manufacture questions or ask implementers to reconstruct intent from external files.
- In Proposal/Design, prefer structured choice questions for genuine decisions: use stable option IDs, explain credible alternatives and their consequences, and state the recommended choice with reasons. Use the workflow's [QUESTION guide](../issue-spec-workflow/references/question-decisions.md). Keep missing facts or parameters open-ended; record settled decisions directly rather than asking again. A prose options list alone is not a `Choice Model`.

## Handoff boundary

For detailed implementation handoffs, read [Executable Design and TASKs](references/executable-design.md). Resolve shared contracts and consequential decisions before implementation; leave private helpers to the owner. Keep the Design body a complete architectural overview plus a TASK/dependency index, with independently readable implementation detail in selected TASK comments. Do not duplicate the entire design in every comment or turn temporary implementation steps into durable SPEC requirements.

Finish with the requested reviewable design artifacts and explicit unresolved decisions; authoring alone does not authorize implementation. Implementation and Review consume the complete authoritative Design Issue, latest effective ANSWERs, selected repository durable specs, code, and executable tests.

At a requested Design-only or coordinated-stage checkpoint, report interface/dependency changes, acceptance coverage and remaining uncertainty and stop for the agreed handoff. An already authorized end-to-end change may continue within that scope. The same owner thread/worktree may later resume implementation with different model settings; neither Design completion nor a model switch grants phase authorization. Include affected durable SPEC paths and intended product-behavior changes in the handoff, leaving implementation-time reconciliation to the owner.
