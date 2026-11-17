# HiRoute feedback

The [shared issue-spec feedback guide](../../.agents/skills/issue-spec-workflow/references/workflow-feedback.md)
loads this supplement through `guidance.shared` from both the `issue-spec-feedback`
skill and the issue-spec workflow. Use it to verify the destination, search for
existing Issues, record and deduplicate occurrences, and check escalation.

Capture reproducible friction during the original task. Examples include repeated
manual repair, ambiguous skill instructions, unnecessary workflow steps, and
missing CLI capabilities. Check the command's exact `--help` or the instruction's
owning source and correct routine usage first. Report a concrete gap that remains
after checking use, version differences and transient failures. Continue the
original task where possible.

## Route and classify

| Cause | Issue repository | Category label | Color for a missing label |
| --- | --- | --- | --- |
| issue-spec CLI behavior, capability, command usability, output or errors | `higress-group/issue-spec` | `improvement/cli` | `0052cc` |
| A reusable issue-spec workflow or generated instruction gap | `higress-group/issue-spec` | `improvement/workflow-generic` | `1d76db` |
| HiRoute-specific guidance, skill, validation, release policy or repository convention | `higress-group/HiRoute` | `improvement/workflow-project` | `0e8a16` |

Both repositories use the `github` profile on `github.com`. A generic workflow fix
must still make sense after removing HiRoute's names, paths and policy. Use the
actual owning repository for an independently maintained skill. If a bounded read
cannot resolve ownership, keep a local draft. Link reports with genuinely
independent causes and reuse the matching Issue for the same unresolved cause.

Apply `improvement/feedback` (color `5319e7`) and exactly one category when creating
or reusing a HiRoute feedback Issue in either repository. Keep unrelated labels and
coordinate any conflicting classification with the Issue owner. The umbrella label
denotes evidence-backed skill, CLI or workflow feedback for interval review.

HiRoute authorizes filing, adding the classification labels, and recording verified
occurrences from its work within the user's scope. Honor narrower restrictions such
as read-only work. The destination policy and backend permissions govern escalation.
Feedback collection leaves implementation, upgrades, automation and additional
reviews to their own authorization.

## Publish and record

Follow the [public content rules](../config.yaml) and write a self-contained English
Issue body. Include the observation date and timezone, origin project, relevant skill
name and repository-relative path, applicable source revisions, owning layer and
routing reason. Give the smallest sanitized scenario, expected and observed behavior,
concrete impact, suggested improvement, acceptance and any prerequisites for
resuming later. Link related Issues and independently verified earlier observations.
Distinguish measured facts from hypotheses and omit private logs, credentials and
local absolute paths. Use `issue-spec version --json` for the installed CLI and
[release.json](../../.agents/skills/issue-spec-workflow/release.json) for the generated
workflow revision. Explain when a relevant revision is unavailable.

For ordinary GitHub feedback in these repositories, follow the
[GitHub skill](../../.agents/skills/issue-spec-github/SKILL.md). Confirm the required
labels exist and provision missing labels if permitted. Set both labels at creation.

```sh
gh issue create --repo OWNER/REPO --title 'Observed problem' --body-file report.md \
  --label improvement/feedback --label CATEGORY
```

Use the Issue backend's supported operations for a non-GitHub destination.
Reconcile uncertain creates before retrying and repair only the missing labels
or record on an existing Issue.

Use the shared guide to preview and submit `issue-spec feedback record` for the
first observation and each independent recurrence. Keep the original event
identity on retries or when reporting the same observation from both entry points.
Read back the Issue and report its URL, category, record status, confirmed count,
escalation result and any incomplete labeling.

## Review an interval

When requested, query both verified repositories for `improvement/feedback`,
including closed Issues. Use creation time for new Issues and search updated older
Issues for dated occurrence records. Use the requested timezone and explicit dates.
Count independent events by observation date and event identity. Page through the
results and disclose gaps in coverage. Group by underlying cause and owning layer,
give Issue and occurrence counts, link representative evidence and prioritize by
observed impact and recurrence. Include relevant fix and adoption updates, with
the observed benefit or missing verification.

## Assess an improvement in later work

Use the Issue and linked fix or adoption record when related work, an authorized
adoption or a requested review supplies context. Confirm the current requirements,
applicable versions and actual resolution. Keep the scenario and the essential
sanitized evidence in the Issue or PR for future sessions.

- For a CLI fix, compare the tested release with the installed revision. Change
  the global CLI when adoption is authorized.
- For a shared workflow fix, use the reviewed CLI and the preview/apply procedure
  in [instruction ownership](../../.agents/README.md). Review generated files,
  manifest, Claude links and project supplements together.
- For a HiRoute fix, update the owning supplement, skill or validation path using
  the project's implementation and PR workflow.

Link the fix and adoption revision. When feasible, rerun the smallest comparable
scenario and applicable project checks. Record the versions, observed outcome and
any missing baseline, changed requirement or unavailable prerequisite. Resume
when later work provides comparable evidence. Keep verification and adoption
results distinct from recurrence counts and write follow-ups when there is new
evidence or a material context change.
