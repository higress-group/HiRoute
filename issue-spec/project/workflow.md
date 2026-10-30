# HiRoute workflow supplement

Read with the generated issue-spec skills. Shared artifact rules, bounded reads and QUESTION commands remain in the generated resources; this file owns HiRoute's project policy.

- Use GitHub `higress-group/HiRoute:main` and the repository-selected issue-spec backend. A normal change can remain a simple Issue; use Proposal/Design/TASK/PROCESS only when their planning or coordination is useful.
- The current session owns authorized planning, implementation and self-tests. Use `hiroute-implementation-writer` for an assigned change. Do not create other sessions or reviewers as a workflow side effect.
- A coordinator is optional, only when the user explicitly designates one for cross-change scheduling and convergence. Its Codex-specific dispatch capability is excluded from Claude; shared planning and implementation do not depend on loading that skill.
- Keep each change within its declared ownership. A designated convergence owner finalizes shared manifests, lockfiles and generated registries; ordinary single-change work needs no separate coordinator.
- The human and GitHub own review, approval and merge. Report the actual candidate, evidence and remaining gaps. If the user selected a coordinated stage, use its agreed handoff and stage boundary; otherwise hand off the current change directly.
- Use current Issues/Designs and answers, repository specs, code and tests as requirements. Current requirements must be self-contained.
- Follow `AGENTS.md` for repository boundaries and task routing; use [HiRoute validation](../../.agents/skills/hiroute-validation/SKILL.md) for checks, execution, failure repair and evidence. Follow config rules for public content and record placement. Public records use English.
- HTML projections remain disabled by `html_review.enabled: false`; load their recipes only when enabled.
- Ordinary GitHub user discussions can use `issue-spec-github`. Selected issue-spec planning and typed artifacts use issue-spec's discussion commands. The repository marker explicitly selects GitHub; an authentication failure must not silently select another backend.
- Capture concrete CLI/workflow friction with [workflow improvement](../../.agents/skills/hiroute-workflow-improvement/SKILL.md): route by the owning layer, deduplicate and label suggestion Issues for periodic analysis, then assess authorized fixes during later relevant work using durable Issue context; missing or obsolete context can leave benefits unverified without blocking iteration. Continue the original task; feedback does not add a phase or authorize automatic implementation.

## PR links and completed Issues

For a delivery PR to `main`, include a separate `Closes #NUMBER` line for every existing Issue whose scope the PR fully delivers. For a phased change, include its Proposal, Design and Implement Issues when all are delivered; do not close only the final phase or create missing phases to fill a template. A bounded simple Issue needs only its own closing reference.

Use ordinary `Refs #NUMBER` links for partial delivery, dependencies, wider programs and outstanding work. Record any deliberately deferred release acceptance in its existing follow-up Issue, preserving the actual validation limits. A merge does not itself prove an unexecuted test passed.

For an issue-spec phase chain, the existing helper can add the closing block after PR creation:

```sh
issue-spec pr link-issues --repo higress-group/HiRoute --pr PR_NUMBER \
  --proposal PROPOSAL_NUMBER --design DESIGN_NUMBER --implement IMPLEMENT_NUMBER
```

Pass only stages that exist and are fully delivered. For a simple Issue, put `Closes #NUMBER` directly in the PR body. Keep these closing lines, including any helper-managed block, when editing or rewriting the description; `Implements`, `Related to` and links alone do not close Issues.

Before handoff, read back the PR with `gh pr view PR_NUMBER --repo higress-group/HiRoute --json baseRefName,closingIssuesReferences`. Verify the target is `main` and every intended Issue appears in `closingIssuesReferences`; repair missing or unintended links before human merge. GitHub performs closure when the PR merges into the default branch. Already merged PRs require explicitly authorized Issue reconciliation using `issue-spec issue close` and a link to the merged delivery; no background closing bot or retired finalization gate is introduced.
