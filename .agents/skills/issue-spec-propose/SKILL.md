---
name: issue-spec-propose
description: Author planning artifacts for a selected issue-spec change; not standalone research, design discussion, or skill maintenance.
license: MIT
metadata:
  requires: "Requires issue-spec CLI."
  author: issue-spec
  version: "1.0"
  generatedBy: "issue-spec"
---

# Issue Spec Propose

Use for /issue-spec:propose or issue-native planning in a selected issue-spec change, not ordinary research, design discussion, or editing these skills. Use issue-spec-workflow for shared reads, provider routing, and recovery.

For selected issue-spec steps, the built-in phase sequence and canonical artifact carriers are authoritative; never reorder/omit steps or move open decisions. This governs workflow structure, not user authorization or unrelated tasks.

Every new typed ID MUST be `<TYPE>-<issue><three-digit sequence>`: Issue 1 starts with `QUESTION-1001`, Issue 44 with `QUESTION-44001`. Allocate 001-999 only within the target Issue and type after reading that Issue's typed comments, and never renumber a legacy ID. New writes reject wrong Issue prefixes; `--allow-legacy-id` is only for intentional legacy-compatible creates.

Before authoring or revising Proposal/SPEC or Design/TASK, read [Requirement scope and verification ownership](../issue-spec-workflow/references/planning-scope.md).

1. Validate workflow config, search related issues, and open only selected discussions. If the issue is already in a later phase, continue that phase rather than duplicating it.
2. Keep unconfirmed investigation in a simple issue with issue-spec issue create simple; a proposal states a confirmed problem and intended change, so never promote an investigation issue into the proposal or attach SPEC/Design to it. Create phase issues with body files, starting with issue-spec issue create proposal --repo higress-group/HiRoute --body-file <file>. Follow `rules.language` and `rules.language_instructions`. For localized titles, pass an explicit `--title` for Proposal, Design, and Implement; the derived title retains an English stage prefix. Keep standardized stage prefixes and avoid style-only title rewrites.
3. Perform the Proposal's first QUESTION discovery/create pass. Record each genuine unresolved decision as a blocking typed QUESTION with issue-spec question create, attaching credible options; never leave an open decision as body or projection prose. Do not manufacture a question or reopen a settled choice; keep unresolved decisions distinct from evidence-dependent items. Resolve routine private details within the confirmed scope; self-contained contracts need not prescribe every helper or layout.
4. Generate canonical SPEC comments with issue-spec comment generate --type SPEC; add --example --id <SPEC-id> for input JSON. Use testable requirements and WHEN/THEN scenarios. --allow-noncanonical is for migration only.
5. Persist the authoritative self-contained Design, perform its first QUESTION discovery/create pass, then complete TASK planning.
6. Generate TASK comments with issue-spec comment generate --type TASK; add --example for input JSON. Execution Planning must identify Design-invariant cohesion and major entry points, bounded role-context pressure, stable interfaces, owned areas, shared touchpoints, dependencies, coupling, and acceptance consequences. File ownership and parallelism are scheduling context, not semantic PROCESS boundaries. TASKs organize the current owner’s work; delegation and PROCESS are optional.
7. Upsert each TASK with --covers-issue so it publishes its complete canonical SPEC coverage and verify planning relationships. Proposal, Design, Implement, TASK, and PROCESS remain optional aids and never certify delivery acceptance.

## Project Workflow

- Workflow Source: `builtin`
- Workflow Schema: `issue-spec`
- Workflow Config: `issue-spec/config.yaml`
- Read the workflow config once for project context, rules and references. Load only guidance.shared and the current phase's guidance.propose, guidance.apply or guidance.review. Paths are relative to the repository root. Project files supplement this shared workflow; do not edit generated skills to customize it.
- Workflow Diagnostics:

Project workflow templates are declarative only. Active proposal, design, implement, SPEC, TASK, PROCESS, and QUESTION artifacts remain in the selected issue backend's issue-native storage; historical REVIEW and VERIFY artifacts are audit-only. Repository-mode durable specs are materialized and checked on the implementation branch.

The built-in phase sequence and canonical artifact carriers are authoritative. Project workflow context, rules, and artifact instructions may constrain work only within an existing step; they MUST NOT reorder or omit an enabled step or move a genuine unresolved decision out of its blocking typed QUESTION carrier. Keep the enabled phase order: persist the phase issue body, perform its first QUESTION discovery/create pass, then author the selected next typed children. Issue-body prose never carries an open decision.
