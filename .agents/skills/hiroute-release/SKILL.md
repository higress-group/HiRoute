---
name: hiroute-release
description: Plan, qualify and publish HiRoute product releases. Use for release preparation or handoff that needs patch or minor acceptance, package evidence, native smoke and publication coordination.
---

# HiRoute release

Use the [hosted release guide](../../../docs/release-builds.md#hosted-build-and-publication)
for versioning, build, signing and publication commands. In the release Issue or
PR, record the intended version, latest recorded stable source, candidate SHA,
assets to ship and remaining gates. Compare the complete release diff with that
stable source.

## Choose acceptance scope

- **Patch within the same major and minor.** Verify each fix through its
  production entry and run the affected regressions. Use the required native
  package gates for install or upgrade smoke where they already cover the path.
  Add installed checks for affected paths that still lack evidence. Reuse
  unaffected suite results and finish when the scoped checks and release gates
  pass.
- **Minor or major.** Cover the accumulated changes and new user journeys.
  Smoke each shipped platform family through a representative installed flow.
  Cover clean installation and upgrade from a supported released predecessor
  where applicable, including saved-state continuity and a real request through
  the configured Gateway. Exercise shared behavior on a representative consumer
  and native differences on their own platforms.
- **Higher-impact changes in either tier.** Expand the affected scope for
  storage, recovery, installer/updater, security, native dependencies and
  published interfaces. Use [compatibility maintenance](../hiroute-compatibility/SKILL.md)
  for supported predecessors and [validation](../hiroute-validation/SKILL.md)
  for scenario selection and execution.

Both tiers require the existing [package qualification and native gates](../../../docs/release-builds.md#build-and-publish-the-frozen-artifacts)
and the [website review](../../../apps/website/docs/release-reviews/README.md)
for the release. Follow the applicable release policy for any scoped deferral.

## Reuse evidence and freeze once

- Carry forward a result with its original SHA and coverage after checking the
  intervening diff and confirming its behavior, inputs and test environment
  still apply. Run outstanding checks and package-bound gates on the frozen
  candidate. Bind installed evidence to its source, platform and exact package
  digest.
- Run independent hosted build and validation jobs in parallel. Start
  standard-user Windows acceptance when the matching installer is ready.
  Reuse successful producer attempts on partial retries. When a package digest
  changes, recheck the gates bound to that package. Keep the frozen candidate
  for later catalog or release-note edits.

## Omarchy graphical smoke

Use [Omarchy in Actions](../../../docs/release-builds.md#prepare-omarchy-packages-in-actions)
for both architecture builds and signing, and x86_64 KVM upgrade and Gateway
coverage. Reserve the existing hardware-accelerated local ARM64 Omarchy guest
for one brief graphical smoke after hosted verification of the signed final
candidate. Verify and install or update that package in the standard locations.
Open from the Omarchy launcher and confirm the native window and authenticated
console are usable. Add a representative affected visual interaction when
relevant. Record the tested package digest, guest platform and result, then
shut down the test guest. Run deeper change-specific acceptance in hosted or
other capable test environments before the smoke. Keep the local smoke pending
if the required local session is unavailable.

## Publish and record

Follow the [public release notes guidance](../../../docs/release-builds.md#write-public-release-notes).
Keep build and acceptance evidence in the release Issue or PR. Publish the
qualified assets at their frozen source within the user's release authorization.
Verify GitHub assets and public catalog availability, then follow the
[release snapshot and development version workflow](../../../docs/release-contracts.md#publication-and-retry).
