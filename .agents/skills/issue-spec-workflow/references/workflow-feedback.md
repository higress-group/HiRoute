# Workflow feedback

Use this reference when authorized work exposes a reproducible problem with a project skill, CLI command, or workflow. The standalone `issue-spec-feedback` skill also links to this reference during ordinary project tasks. Continue the original task and use an ordinary Issue and comments. Apply the current task's publication authorization and any standing authorization in project guidance.

## Find the project context

From the checkout root, read `.issue-spec/config.json` to find the project's Issue repository (`repo`), `profile`, `hostname`, and self-hosted `server_instance_id` when present. If the file is absent, use the project Issue target established by the task. The code remote or Source Binding can point to a different repository.

Run `issue-spec workflow which --repo "$project_repo" --json` with the project's Issue repository. Read the config at `.workflow.source.config_path` when present, including `openspec/config.yaml` if selected. Apply the relevant project rules and load the repository-relative files in `.workflow.config.guidance.shared`.

## Route and record

1. Choose the destination repository by root cause. Use the project's Issue target for locally owned skills, guidance, validation, and release problems. For CLI limitations, reusable workflow improvements, or externally maintained skills, use the owning upstream Issue target identified by the task or project guidance. Set `feedback_repo` and `feedback_profile` from that destination. Keep an internal draft while the destination is uncertain.
2. Run `issue-spec --profile "$feedback_profile" auth status --json` and require `.ok` and the intended `.auth.profile`. Confirm the expected GitHub host or the self-hosted `.auth.server_instance_id` for the destination. For GitHub Enterprise with the built-in `github` profile, pass the destination's `--hostname` to auth and each Issue command as well. Use the same verified profile and repository for every destination read and write, including a retry. User or project authorization and backend permissions govern writes.
3. Search open and closed discussions for the same cause with `issue-spec --profile "$feedback_profile" search issues --repo "$feedback_repo" --scope discussion --state all --query 'specific behavior'`. Read plausible matches with `issue-spec --profile "$feedback_profile" read issue --repo "$feedback_repo" --issue N --comments`. Reuse an existing Issue for the same unresolved cause and supplement a closed decision with authorized evidence when appropriate. Link a distinct regression after a fix to its predecessor.
4. For a new authorized report, use `issue-spec --profile "$feedback_profile" issue create simple --repo "$feedback_repo" --title 'Observed problem' --body-file report.md`. Include the dated observation, originating project and revisions, expected and actual result, impact, owning layer, suggested correction and acceptance. Follow the destination's classification policy. An initial taxonomy uses `improvement/feedback` and one of `improvement/cli`, `improvement/workflow-generic`, or `improvement/workflow-project`. If classification needs a separate authorized label action, retain the original Issue identity and retry that action.
5. Record each independent event with its original context. For a skill problem, include the skill name, repository-relative path, applicable revision, and the step and impact that can be verified. Use the provider's stable source instance identifier, source repository and original task or release identifier. For a retried release, preserve its original key even when the run or attempt changes. A maintainer or configured trusted reporter can map each verifiable historical event to its own record and link the original evidence. Count independent events across Issue bodies and ordinary comments. Treat discussion, handoffs and repair verification as context for those events.
6. Preview the record. Confirm the returned `url` identifies the intended Issue and the count is correct. Omit `--dry-run` when the write is authorized.

```sh
issue-spec --profile "$feedback_profile" feedback record --repo "$feedback_repo" --issue 42 --event-file occurrence.json --dry-run --json
```

```json
{
  "source_instance": "github:github.com",
  "source_repository": "example/project",
  "event_id": "release:v1.2.3@abc1234",
  "observed_at": "2026-11-16T10:30:00+08:00",
  "revision": "abc1234",
  "summary": "The catalog deployment ran before publication and its download check returned HTTP 404.",
  "evidence": ["https://github.com/example/project/actions/runs/123"]
}
```

Include only sanitized evidence. `revision` is optional. A preview reports the event key and projected confirmed count using the current identity. With a trusted reporter list, a contributor can record an event for confirmation. When the destination restricts trusted reporters, a configured reporter can confirm an earlier unconfirmed event by verifying the evidence and recording the same key. The backend supplies the comment author. A submitted field claiming confirmation or identity carries no authority.

An ambiguous submission is reconciled by rereading the Issue. Retry the same event file if needed. If the result shows a pending label action, an authorized caller can retry that action with the same event without increasing the count. Preserve the Issue URL in handoffs.

## Defaults and optional destination policy

Recording and preview work after ordinary initialization. The default policy counts distinct evidence from authenticated Issue authors and suggests `priority/high` at three confirmed occurrences.

To change the threshold or enable automatic escalation, publish a policy from an ordinary Issue in the **destination** repository. Its body contains a single JSON object, optionally fenced as JSON. To customize just the threshold, the entire body can be `{"threshold": 5}`.

Automatic escalation uses `"mode": "auto"` and a list of trusted reporters. For each trusted account, get the stable ID from `issue-spec --profile "$feedback_profile" auth status --json` (`.auth.user_id`). Write the IDs as strings.

```json
{
  "threshold": 3,
  "mode": "auto",
  "priority_label": "priority/high",
  "trusted_reporter_ids": ["123"],
  "higher_priority_labels": ["priority/critical"],
  "hold_labels": ["priority/deferred"]
}
```

An automatic assignment uses a predefined repository label. The priority label defaults to `priority/high`. Higher-priority and hold labels are optional. `issue-spec/feedback-hold` and `wontfix` are recognized holds. A closed or locked Issue is ineligible.

```sh
issue-spec --profile "$feedback_profile" feedback configure --repo "$feedback_repo" --issue 17 --dry-run --json
issue-spec --profile "$feedback_profile" feedback configure --repo "$feedback_repo" --issue 17
```

Publishing requires permission to manage repository label definitions. The command binds the exact policy body to the repository's `issue-spec/feedback-policy` label definition. Change the policy Issue body and publish again to activate a new revision. Remove the policy label definition to return to the built-in suggestion policy. GitHub enforces the caller's permission to assign a label. On self-hosted, a caller who can create Issues can request promotion. The server rechecks the published policy, persisted evidence and Issue eligibility before assigning the configured label. The ordinary issue-label API uses its triage authorization.

## Customize project guidance

Use an entry in `guidance.shared` in the selected workflow config to customize when to use feedback, project and upstream destinations, sanitization, or standing publication authorization. The standalone feedback skill and shared reference are available after ordinary init or a workflow update. Preview an existing checkout's update with `issue-spec workflow update --dry-run --repo "$project_repo"`.

For a requested interval review, include open and closed feedback Issues and compare their creation dates and dated evidence. Group by root cause and count distinct keys. Record the upstream fix, local adoption revision and comparable verification as they become available in relevant authorized work. Prefer the original Issue and its evidence links for handoff across sessions.
