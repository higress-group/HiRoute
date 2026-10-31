---
name: hiroute-public-pr
description: Prepare and audit a HiRoute task branch for a public GitHub pull request, including every outgoing commit and its messages. Use before any public push, including validation pushes, and for requested PR delivery; not for unrelated implementation or automatic merging.
---

# HiRoute public pull requests

HiRoute develops directly on `higress-group/HiRoute:main`. Start from current
GitHub `main` and preserve its contributors' changes. Read the project's public
content rules in `issue-spec/config.yaml` and its shared guidance before GitHub
writes. Keep current specifications and lasting guides in the repository;
record design, decisions, progress and validation summaries in the Issue or PR.
Drafts and raw evidence stay in ignored local artifacts.

## Prepare the candidate

Fetch GitHub and record the exact base and candidate. Inspect the checkout and
preserve unrelated work. Use `<type>/<concise-kebab-case-topic>` for branches
created by this skill, such as `feat/`, `fix/`, `doc/`, `chore/`, `test/`, or `ci/`.
The user can request another name. Keep task history scoped to the selected
GitHub base and change; never merge unrelated history or publish branches/tags in bulk.

Keep changes within the authorized scope. If publication preparation would
change product behavior, a contract, default, routing, authentication or bundled
component, describe the concrete conflict and seek the unresolved decision;
continue independent work. This is not an extra approval for routine fixes.

Read [the public surface](references/public-surface.v1.json). Its explicit
path rules include agent instructions, shared skills, project policy and durable
specs. New documents must be current specifications or lasting guides, rather
than process diaries. Classify genuinely unknown paths with the user before
publication and record the narrowest agreed rule. Retiring a document also
removes its required `include_docs` entry; the base manifest can classify only
its deletion, never changed or new content.

## Check before every public push

Run from the repository root:

```sh
python3 .agents/skills/hiroute-public-pr/scripts/public_surface.py \
  audit-range --base origin/main --head HEAD
python3 .agents/skills/hiroute-public-pr/scripts/public_surface.py \
  audit-text --file /path/to/prepared-pr-body.md
```

The range audit inspects every outgoing commit, including merge-side history,
changed file contents, intermediate paths, commit messages and author sign-offs.
An added-then-deleted file is still public history. Findings identify the rule,
path and revision without printing matched sensitive values. Optional personal
literal deny patterns can be supplied with `--deny-file PATH` (a JSON string
array kept outside the project). Public developers do not need that file.

The generic scan is an aid, not proof that everything is publishable. Review the
whole outgoing history and prepared title/body for credentials, private
infrastructure, personal configuration, raw logs and inaccessible context.
Verify attachments separately. Keep only sanitized examples and self-contained
requirements. If a sensitive intermediate commit exists, build a clean task
history before its first push; deleting it at the tip is insufficient. Recheck
any repaired candidate before validation or PR pushes.

Every outgoing commit requires `Signed-off-by` matching its author. Use
`git commit -s` and preserve attribution for reused contributions. Rewrite only
an authorized task branch when necessary; never rewrite public `main` or another
owner's branch. For a sign-off-only repair, prove the tree unchanged and use
`--force-with-lease` against the observed remote head if already pushed.

## Verify and deliver

Follow `AGENTS.md`, the [code map](../../../docs/code-map/README.md) and
[HiRoute validation](../hiroute-validation/SKILL.md). Verify references
against the public checkout so private files cannot satisfy missing dependencies.
Choose checks from changed behavior, then bind results to the exact candidate.
A source build does not prove installation or an actual user journey.

Push the checked task branch within the user's authorization, including when
SSH validation requires it. Create an English PR to `main` with the concrete
problem, resulting behavior, Issue link and relevant public validation. Inspect
only the CI/checks selected for the authorized handoff; do not add continuous
monitoring or unrelated investigations. Keep raw logs and publication bookkeeping
local. Update the selected issue-spec task with the real PR and state.

Follow the project's [Issue-closing policy](../../../issue-spec/project/workflow.md#pr-links-and-completed-issues):
put a separate `Closes #NUMBER` in the PR body for every fully delivered existing
Issue, including the completed Proposal/Design/Implement chain. A simple Issue
needs only its own reference; partial work and wider programs keep `Refs` links.
For phase chains, `issue-spec pr link-issues` can add the closing block with only
the delivered stages. Preserve the block through later PR-body edits. Before
handoff, read `baseRefName` and `closingIssuesReferences` with `gh pr view` and
confirm the default-branch target and exact intended Issue set. Repair omissions
or unintended closing links before human merge; ordinary links are insufficient.

Attach the created PR to the current Codex task when that capability is available.
PR review and merge belong to the human unless explicitly authorized. A passing
check or completed task does not grant merge or release authority.

For changes to this skill, run `python3
.agents/skills/hiroute-public-pr/scripts/test_public_surface.py` and the available
skill validator.
