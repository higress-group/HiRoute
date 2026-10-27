# Durable contract maintenance

Use during implementation when the selected workflow maintains repository durable specs. The current implementation owner reconciles the affected contracts on the implementation branch and includes them in the same reviewable change. Proposal SPECs define the requested increment; durable specs define the complete current contract after that change. Proposal-only work does not require publishing future behavior as a current durable contract.

## Reconcile the contract

Read the affected existing durable requirements together with the confirmed change requirements, current Design and effective decisions when present. Identify what is added, changed, explicitly removed or renamed. Preserve still-applicable behavior and scenarios, including meaningful failure, authorization, data lifecycle and compatibility constraints. Check code and executable tests against that intent; incomplete implementation is a gap to fix or report, not permission to weaken the contract. Resolve consequential conflicts before affected work.

Organize by lasting capability and contract ownership, reusing existing files and requirements. Proposal SPECs and durable requirements need not map one-to-one: one change may affect multiple capability files, and successive changes may update the same requirement. Keep the resulting behavior self-contained, avoid duplicate normative definitions, and retain source references separately for traceability. Do not copy change-relative wording, implementation schedules, TASK verification procedures or execution evidence into the durable contract. Preserve unrelated content; this does not authorize a repository-wide rewrite.

## Select the appropriate projection

When using the CLI's durable-spec preview/apply/check protocol for a selected Proposal, record explicit Durable Intent operations on its source SPECs for the affected paths and requirements. A MODIFIED operation replaces the entire target Requirement block, including its scenarios; it does not merge individual sentences or preserve omitted scenarios inside that block. Supply the complete intended replacement, retaining every still-applicable contract condition.

Capability identity comes from the declared path slug, not the document's display title. Requirement operations preserve existing readable titles and independent sections after Requirements (such as Implementation references); added requirements stay inside Requirements. Keep canonical Purpose and Requirements sections and Requirement headings. If preview reports unsupported or ambiguous structure, diagnose it before writing rather than changing the title or dropping other sections just to satisfy the tool. When multiple source SPECs affect the same Requirement, consolidate the final contract under one owning operation; duplicate target operations are rejected.

- Use `current-spec` only when the source SPEC already expresses the complete intended durable Requirement and scenarios, with the matching target title and no temporary change context. It is suitable for a self-contained new requirement or a complete replacement, not an incremental note about an existing requirement.
- Use `inline` when the durable Requirement needs consolidation with existing behavior or a different capability-oriented formulation. Author the complete resulting requirement and scenarios in the projection rather than expanding every Proposal SPEC into a full capability document.
- Use `SECTION_MODIFIED` only for an explicitly authorized update to an existing `Purpose` or independent trailing `Implementation references` section. Declare the exact baseline body in `before` and the complete replacement in `projection: {"source":"inline","text":"..."}`. Exclude surrounding LF delimiters, preserve internal whitespace, and keep both bodies nonempty and at most 4096 bytes without document-level Markdown headings. The original heading and surrounding delimiters remain unchanged. Duplicate section targets across source SPECs and changes that swallow neighboring headings are rejected. All other sections remain protected by the full-file check.
- Use explicit REMOVED or RENAMED operations for those intentions. Review omitted old behavior as a possible regression unless a confirmed decision authorizes its removal; absence from the Proposal is not such a decision.

Preview the declared operations against the intended baseline, inspect the complete resulting contract, apply, and check the actual candidate. If the content needs correction, revise its declared projection and regenerate; do not hand-edit the result while leaving an inconsistent projection. These commands check structure, source/baseline consistency and equality to the declared result. They do not prove semantic completeness, preservation of old behavior or implementation correctness.

For a simple-Issue change without Proposal SPECs, maintain affected durable files through the normal repository edit and review path. Do not manufacture Proposal/SPEC artifacts just to invoke projection commands. The same semantic reconciliation applies.

## Review with the implementation

Before handoff, inspect the durable diff for missing old constraints, duplicate or conflicting definitions, unauthorized scope and temporary validation details. Report updated capability paths, changed behaviors and unresolved implementation/spec mismatches with the normal implementation handoff. Preserve the project's required checks and human review; do not add a separate archive phase, receipt, reviewer or per-SPEC approval gate.
