---
name: issue-spec-apply
description: Implement a selected issue-spec change, directly or with managed PROCESS coordination. Not a trigger for ordinary code edits or skill maintenance.
license: MIT
metadata:
  requires: "Requires issue-spec CLI."
  author: issue-spec
  version: "1.0"
  generatedBy: "issue-spec"
---

# Issue Spec Apply

Implement the authorized change directly in the current session. Use issue-spec-workflow for shared artifact rules and bounded reads. Design/TASK/PROCESS never requires a second code writer. Read the configured project guidance for ownership, validation and stage handoff. If Implement is selected, persist it, perform its first QUESTION pass, then author PROCESS only for actual managed coordination.

For selected issue-spec steps, the built-in phase sequence and canonical artifact carriers are authoritative; never reorder/omit steps or move open decisions. This governs workflow structure, not user authorization or unrelated tasks.

Every new typed ID MUST be `<TYPE>-<issue><three-digit sequence>`: Issue 1 starts with `QUESTION-1001`, Issue 44 with `QUESTION-44001`. Allocate 001-999 only within the target Issue and type after reading that Issue's typed comments, and never renumber a legacy ID. New writes reject wrong Issue prefixes; `--allow-legacy-id` is only for intentional legacy-compatible creates.

## Implementation and Handoff

Read the complete Design, applicable answers, selected TASKs and required SPECs/dependencies. Confirm base/worktree, ownership, shared interfaces and required checks. Implement and self-test within that scope; recover derivable facts and pause only work affected by missing or conflicting requirements. Keep durable specifications current, without turning temporary development stages into product requirements.

For repository durable specs, follow [Durable contract maintenance](../issue-spec-workflow/references/durable-maintenance.md) to reconcile the requested increment with the complete current contract before handoff.

For user-facing critical paths, inspect added scans, subprocesses, recomputation and synchronous refreshes. Reuse results within the operation while their inputs remain valid; recheck mutable authorization and external state at the boundary that requires them. Use a focused regression to show that growing unrelated state does not add heavyweight work, and measure through the point when the user can actually proceed.

When adding or changing an execution script, exercise its actual launch entry, including setup and exit. If the changed behavior depends on an external system, verify the smallest authorized real end-to-end path early, before expanding test coverage; record missing prerequisites as gaps.

Run affected checks and explicit gates. After they pass, expand testing only for a change, failure or concrete unresolved risk. Evidence names the revision actually checked; after a repair rerun affected checks and preserve reused evidence at its original revision. Report the candidate, results, gaps and remaining review needs. Self-tests do not constitute independent review or approval. Do not dispatch a reviewer, publish, push or merge as a workflow side effect.

Delegation remains optional and requires actual authorization. Each assigned owner implements and tests its bounded package; any designated convergence owner handles shared changes and integration. Do not manufacture phase artifacts, PROCESS or role receipts to record ordinary single-owner work.

For requested implementation review or publication, read [Implementation Review and Publication](../issue-spec-workflow/references/implementation-review.md) completely. Do not load it for Proposal/Design authoring.

For selected managed PROCESS work, use the assignment below with its actual owner, which may be the current agent. Preserve exact base, ownership, DCO, tests, generators, dependency order, managed worktree isolation, and bounded handoff. These controls are implementation safety only: they do not create review, verification, rationale evidence, receipt, coverage, finalization, or delivery-acceptance authority.

## Implementation Role Packet

Apply this packet only when managed PROCESS execution is selected. A delegated owner receives the bounded assignment; a direct owner may execute it in its assigned worktree.

1. Accept only the sealed implementation assignment for the exact PROCESS, base revision, worktree, write ownership, focused tests, generators, result schema, and design_context. Do not load proposal bodies, the complete DAG, link matrices, human merge policy, provider routing, or unrelated artifacts.
2. Require design_context.read_mode=complete-issue-body and conflict_policy=design-authoritative-stop. Read the complete Design with issue-spec read issue --repo higress-group/HiRoute --issue <design_context.source_url> without comments, timeline, history, or gates. Recover derivable metadata. Stop and report any conflict in scope, authority, ownership, or contracts; routine private choices are not conflicts.
3. Work only in the assigned worktree and owned paths. Preserve the named invariant, decisions, must_preserve, must_not, and minimum_verification exactly. Do not collect or pass runtime session IDs.
4. Implement the invariant, run assigned generators, finish exactly one DCO commit when required, and leave the tree clean. Collect zero or more line-rationale drafts only for non-obvious decisions: repository-relative path, stable symbol plus changed-line anchor, and concise why/tradeoff/risk without secret, raw payload, or credential. Do not guess a provider diff position or create filler. If cohesion fails, stop with stable-interface split options and acceptance consequences.
5. Complete the assigned behavior and fix introduced failures. Run every assigned generator and focused test; expand or repeat only for changes, failures, concrete risks, or required gates. Return the exact result commit, changed paths, command outcomes, decisions, risks, and line-rationale drafts. Do not create a role receipt, decision file, or evidence carrier. Provider access and final diff positions are not worker responsibilities.
6. An amendment invalidates the returned revision; rerun affected and explicitly required checks. Reused results keep their original revision and require justified applicability. Leave shared integration and lifecycle completion to the designated convergence owner. Do not impersonate another writer when publishing rationale.

## PROCESS Write Ownership

- A bare repository-relative ownership path is one exact file.
- A directory subtree requires an explicit trailing /** declaration, for example internal/templates/**.
- Legacy bare directory declarations remain readable, but workspace prepare may reject them; correct the PROCESS or pass an explicit recursive ownership value before allocation.

## Project Workflow

- Workflow Source: `builtin`
- Workflow Schema: `issue-spec`
- Workflow Config: `issue-spec/config.yaml`
- Read the workflow config once for project context, rules and references. Load only guidance.shared and the current phase's guidance.propose, guidance.apply or guidance.review. Paths are relative to the repository root. Project files supplement this shared workflow; do not edit generated skills to customize it.
- Workflow Diagnostics:

Project workflow templates are declarative only. Active proposal, design, implement, SPEC, TASK, PROCESS, and QUESTION artifacts remain in the selected issue backend's issue-native storage; historical REVIEW and VERIFY artifacts are audit-only. Repository-mode durable specs are materialized and checked on the implementation branch.

The built-in phase sequence and canonical artifact carriers are authoritative. Project workflow context, rules, and artifact instructions may constrain work only within an existing step; they MUST NOT reorder or omit an enabled step or move a genuine unresolved decision out of its blocking typed QUESTION carrier. Keep the enabled phase order: persist the phase issue body, perform its first QUESTION discovery/create pass, then author the selected next typed children. Issue-body prose never carries an open decision.
