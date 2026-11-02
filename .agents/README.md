# Agent instruction ownership

`AGENTS.md` owns repository rules; root `CLAUDE.md` is a relative symlink to it.
`.agents/skills/<name>/` is the canonical installed location for instructions, references and scripts. The `issue-spec-*` files listed in `issue-spec-workflow/release.json` are generated and hash-protected; their source is the issue-spec CLI. Project-owned `hiroute-*` skills remain maintained here.
Codex discovers that directory directly. `.claude/skills/` is a real directory
containing relative, per-skill symlinks to the shared subset. Read and edit the
canonical paths, including when following sibling-skill references.

`AGENTS.md` keeps global boundaries and task routing. The shared
`hiroute-validation` skill owns check selection, execution, failure repair,
retention and handoff guidance for ordinary changes as well as integration.
Its linked guides retain runner commands and report details. Keep integration
ownership in `hiroute-integrate`, rather than making it a prerequisite for testing.

`hiroute-ui-quality` owns shared interaction and visual-delivery guidance for the
product UI. Its [interaction reference](../docs/desktop-interactions.md) is the
single current source for equivalent controls, edit/confirmation state and
comparative evidence. Design, implementation and validation skills reference it
when their task changes UI; they do not maintain competing style or approval rules.

Optional local skills and their matching Claude links can be explicitly ignored.
They are not part of the published skillset or prerequisites for public worktrees.
Keep required public references within the tracked instruction set.

Add a Claude link only after checking the skill and its required references for
Codex host dependencies. Do not link the entire skills directory: new skills
need an explicit exposure decision. Link the complete skill folder, not just
`SKILL.md`, to preserve its supporting resources. Do not restore the obsolete
`.claude/commands/issue-spec/` workflows or generate copies over current policy.

HiRoute supplements live in `issue-spec/project/`, selected by the `guidance`
entries in `issue-spec/config.yaml`. Shared reads, artifact rules and QUESTION
commands come from issue-spec; project coordinator, stage review and validation
policy stay in the supplements and existing `hiroute-*` skills. Do not customize
the generated shared bodies. Use a reviewed issue-spec CLI revision, preview with
`issue-spec workflow update --repo higress-group/HiRoute --tools codex,claude --dry-run`,
then apply the same command without `--dry-run`. Review the Git diff, link exposure
and project behavior; a second preview must report zero changes. Conflicts require
reconciliation, not deleting the manifest or force-overwriting local edits.
Commit the manifest, generated files and links together so worktrees stay pinned.

The shared [workflow improvement skill](skills/hiroute-workflow-improvement/SKILL.md)
owns feedback routing, deduplication, independent recurrence counting, automatic
high-priority promotion from the third confirmed occurrence, stable analysis labels
and verification of adopted improvements when later work provides comparable
context. Issues preserve the resumption context across sessions; feedback does not
keep an iteration open. Its CLI and shared workflow suggestions go upstream;
HiRoute policy suggestions stay here. It has no Codex host dependency and is exposed
to Claude through its own relative link.
Generated workflow files and their manifest remain owned by issue-spec.

## Codex-only skills

| Skill | Why it is not exposed to Claude |
| --- | --- |
| `hiroute-process-coordinator` | Creates and coordinates independent Codex app threads. |
| `hiroute-cu` | Uses the Codex host's native CU runtime and review loop. |

These skills remain available to Codex. Claude must not bypass the exposure
boundary by loading them through another skill or repository instruction. Shared
UI quality checks can run in Claude; their Codex native-CU portion remains pending
until performed in a capable Codex session. A browser check does not satisfy it.

The validation and Proposal-owner skills (`hiroute-validation`,
`hiroute-implementation-writer`) and the issue-spec
workflow, apply, propose and design-author skills are shared. They run in the
current Codex or Claude session without creating other sessions. Their references
to central coordination describe an external responsibility, not a requirement
for Claude to load the Codex coordinator. Claude reports handoff needs to the user.

`agents/openai.yaml` remains Codex-specific metadata. The coordinator keeps
`allow_implicit_invocation: false` to avoid automatic selection of the dispatch
role. The Proposal owner uses `true`: an assigned change can select it without an
explicit skill mention. Claude uses its default automatic skill selection. Neither
setting grants phase, messaging or publication authority. Host permissions and
personal settings remain separate.

## Check changes

Verify that `CLAUDE.md` resolves to `AGENTS.md`, each Claude skill link is relative
and resolves to the matching canonical folder with a `SKILL.md`, and no Codex-only
skill or old command is exposed. Inspect the skill list in a fresh host session
when validating actual discovery; filesystem checks alone do not prove runtime
loading. Keep all links within the checkout so other worktrees use their own
instruction revision.
