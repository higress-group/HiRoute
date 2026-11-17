---
name: hiroute-workflow-improvement
description: Route and classify reproducible feedback about HiRoute project skills, the issue-spec CLI, and workflows. Use for project reporting rules and later adoption checks.
---

# HiRoute Workflow Improvement

Use this skill for HiRoute's reporting rules, classification, and adoption checks.
Record observations from project skills and workflows through the
[shared issue-spec feedback guide](../issue-spec-workflow/references/workflow-feedback.md).

Capture observations while doing the original task, such as repeated manual repair,
ambiguous guidance, unnecessary steps or missing command capability. Do not launch
a separate broad audit or require a feedback report for every task.
Continue the original task wherever possible. Feedback is not a new planning,
review or delivery gate, and creating an Issue does not authorize its implementation.

HiRoute's repository policy authorizes filing, recording verified occurrences, and
applying the feedback classification labels for evidence from its work. The
destination's published feedback policy and backend permissions govern escalation.
Honor a user's narrower scope, such as read-only or no publication.
Do not infer this standing authorization when copying this skill to another project.
Do not start agents, recurring jobs, workflow upgrades or automatic rule rewrites.

## Establish the cause and destination

Read [instruction ownership](../../README.md), the current project's
`issue-spec/config.yaml` rules and shared guidance. Inspect the relevant installed
command's exact `--help`, or the instruction and its owning source. Record the CLI
revision from `issue-spec version --json`, the generated workflow source revision
from `../issue-spec-workflow/release.json`, and the project commit when applicable.
Different installed and generated revisions alone are not a defect.

| Finding | Destination | Category label |
| --- | --- | --- |
| CLI behavior, command UX, unsupported capability, incorrect output or error | `higress-group/issue-spec` | `improvement/cli` |
| Shared workflow or generated instruction gap whose correction remains useful after removing HiRoute names, paths and policy | `higress-group/issue-spec` | `improvement/workflow-generic` |
| HiRoute policy, project supplement, local skill, validation or repository-specific convention | `higress-group/HiRoute` | `improvement/workflow-project` |

Use the smallest source that actually owns the correction. A stale local recipe or
an agent ignoring correct instructions is not automatically a CLI defect. Correct
routine command use first. File only when a concrete usability or instruction gap
remains; transient network/authentication errors alone do not establish one.

For workflow findings, explain whether the fix survives removal of project policy.
If ownership remains uncertain, gather a bounded additional read; keep a local draft
and state the uncertainty instead of guessing the destination. Split genuinely
independent CLI and workflow causes into linked reports, not copies of one symptom.
Never edit generated `issue-spec-*` files to implement upstream suggestions.

## File and record feedback

Use the [report and analysis guide](references/feedback.md) for issue body fields,
classification labels, and interval reviews.

1. Follow the shared guide to verify the destination Issue repository and profile,
   and search open and closed discussions for the same cause. HiRoute and issue-spec
   use GitHub. Read plausible matches and disclose the coverage of a bounded search.
2. Reuse the Issue for the same unresolved cause. Respect closed and wontfix
   decisions. Link a distinct regression after a fix to its predecessor and use a
   separate Issue for an unrelated cause.
3. For a new Issue, prepare a self-contained English body with sanitized
   reproduction, expected and observed behavior, impact, owning layer, suggested
   correction and acceptance. Include the minimal scenario and prerequisites
   needed by a future session.
   Distinguish measured facts from hypotheses. A design gap can use an observed
   task sequence rather than a failing command. Do not manufacture a bug to fill
   a category or attach private logs, credentials or local absolute paths.
4. On GitHub, provision missing labels first, then create the ordinary feedback Issue
   with `gh issue create --body-file ... --label ...` so creation includes its
   classification. Follow the configured backend's Issue operations for a
   non-GitHub target. Apply `improvement/feedback` and exactly one category to both
   new and reused Issues. Preserve unrelated labels and resolve conflicting
   classification with the Issue owner.
5. Use the shared guide to preview and record the first observation and each
   independent recurrence with `issue-spec feedback record`. Reuse the original
   event identity when replaying a report from a skill or workflow. Read the
   confirmed count and escalation decision from the command result.
6. Read back the Issue URL, evidence, and labels. Reconcile an uncertain create
   against existing Issues. Keep the Issue URL and retry the missing operation
   when classification or evidence recording only partially succeeded.

Report the Issue URL, category, recording status, confirmed count, escalation
result, and any incomplete labeling.

## Review an interval

When asked, collect the umbrella label from both repositories for the requested
interval, including closed Issues. Use creation time for new reports and dated
occurrence comments for recurrences; updated time is only a discovery aid. Group
by underlying cause and layer, link representative evidence, distinguish unique
Issues from occurrences, and prioritize by observed impact and recurrence.
Propose common fixes upstream and project-specific fixes locally. Summarizing
feedback does not authorize implementation, closure, relabeling history or a schedule.

## Bring improvements back into HiRoute

Resume this loop only when a later related task, an authorized adoption, or a
requested periodic review makes a feedback item relevant. Do not keep the current
task open, wait for an upstream release, poll, or scan the entire backlog on every
iteration. A future session can recover the scenario from its Issue and linked
durable evidence using the [resumption guide](references/feedback.md#resume-after-a-gap).

When a relevant feedback item has a proposed or released fix, assess its current
applicability before planning adoption. Read its actual resolution,
linked implementation and available revision; an Issue closure alone is not proof
that a fix exists, is installed or addresses HiRoute's scenario. Batch related
candidates for the user's periodic review instead of repeatedly interrupting work.

- **CLI correction:** identify the tested fix/release and the currently installed
  revision. Propose adoption when needed. Install or change a global CLI only within
  the user's authorized scope; a feedback-filing policy does not authorize upgrades.
- **Shared workflow correction:** use the reviewed CLI revision and the workflow
  preview/apply procedure in [instruction ownership](../../README.md). Review the
  generated diff and project supplements together; preserve the manifest and links.
  Do not hand-patch generated instructions or update them merely to clear a warning.
- **Project correction:** change the owning HiRoute supplement, skill or validation
  path through the normal authorized change and PR process. Avoid copying generic
  upstream rules into local policy when the shared source now supplies them.

Adoption is separate implementation work: propose it when not yet authorized, and
proceed without asking again when the current task already includes it. Keep
feedback Issues linked to the corresponding fix and adoption PR/revision.

When a comparable scenario is available within the current scope, validate an
adopted improvement against the recorded symptom and applicable project checks.
Recheck current requirements and environment first; historical evidence explains
the report but does not override today's contract. Record actual results and source
versions on the feedback Issue or adoption PR.

If the original context is missing, too costly to reconstruct, obsolete or no longer
relevant, record that limitation and leave the benefit unverified or the scenario
no longer applicable. State a useful future trigger only when known, such as the
next task using the affected command. Do not invent a deadline, assign the original
session ongoing ownership, or reconstruct an old environment solely to finish the
feedback loop. This does not waive checks required by an actual adoption change.

Do not claim time savings without measurements, count an upstream release as local
adoption, infer success from silence, or automatically close the report. New relevant
evidence can resume the same thread later; a remaining cause returns there without
requiring continuity of agent, conversation or worktree.
