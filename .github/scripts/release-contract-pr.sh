#!/usr/bin/env bash
set -euo pipefail

: "${RELEASE_TAG:?published release tag required}"
: "${RELEASE_REVISION:?verified source revision required}"
: "${WEBSITE_REVISION:?verified release-manifest revision required}"
: "${GH_REPO:?repository required}"
set --
if [[ -n ${SUPERSEDE_UNPROMOTED_REVISION:-} ]]; then
  [[ "$SUPERSEDE_UNPROMOTED_REVISION" =~ ^[0-9a-f]{40}$ ]] || { echo 'Invalid prior revision' >&2; exit 1; }
  set -- --supersede-unpromoted-revision "$SUPERSEDE_UNPROMOTED_REVISION"
fi

root=$(git rev-parse --show-toplevel)
scratch=$(mktemp -d)
worktree="$scratch/pr"
cleanup() {
  git -C "$root" worktree remove --force "$worktree" >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT

# Resolve and validate the published source before any branch/PR write.
git show "${WEBSITE_REVISION}:apps/website/data/releases.json" > "$scratch/releases.json"
gh release view "$RELEASE_TAG" --repo "$GH_REPO" \
  --json tagName,isDraft,isPrerelease,publishedAt,assets > "$scratch/release.json"
python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["tagName"] == sys.argv[2], "release tag mismatch"' \
  "$scratch/release.json" "$RELEASE_TAG"
python3 "$root/scripts/release-contracts.py" record --repo "$root" \
  --directory "$scratch/verified" --manifest "$scratch/releases.json" \
  --release-json "$scratch/release.json" --revision "$RELEASE_REVISION" \
  --repository "$GH_REPO" > /dev/null

branch="chore/release-contracts-${RELEASE_TAG}"
if [[ $# -gt 0 ]]; then
  branch="${branch}-${RELEASE_REVISION:0:12}"
fi
git fetch origin main --tags
# Fast path after the generated PR has already merged.
git worktree add --detach "$worktree" origin/main
python3 "$root/scripts/release-contracts.py" record --repo "$root" \
  --directory "$worktree/contracts/releases" --manifest "$scratch/releases.json" \
  "$@" \
  --release-json "$scratch/release.json" --revision "$RELEASE_REVISION" \
  --repository "$GH_REPO" > /dev/null
python3 "$root/scripts/product-version.py" advance --repo "$worktree" > /dev/null
if [[ -z $(git -C "$worktree" status --porcelain) ]]; then
  echo 'Published contract snapshot and next development version are already recorded on main.'
  exit 0
fi
git worktree remove --force "$worktree"

existing=$(gh pr list --repo "$GH_REPO" --head "$branch" --base main \
  --state all --json state,url --jq '.[0] // {}')
state=$(python3 -c 'import json,sys; print(json.load(sys.stdin).get("state", ""))' <<< "$existing")
if [[ "$state" == CLOSED || "$state" == MERGED ]]; then
  echo 'A prior snapshot PR is closed but main lacks this exact record; resolve that discrepancy before retrying.' >&2
  exit 1
fi

git config user.name 'github-actions[bot]'
git config user.email '41898282+github-actions[bot]@users.noreply.github.com'
python3 "$root/scripts/product-version.py" paths > "$scratch/version-paths"
if git ls-remote --exit-code --heads origin "$branch" > "$scratch/remote-ref"; then
  git fetch origin "$branch:refs/remotes/origin/$branch"
  git worktree add --detach "$worktree" "origin/$branch"
  python3 "$root/scripts/product-version.py" scope --repo "$worktree" --base origin/main > /dev/null
  if ! git -C "$worktree" merge-base --is-ancestor origin/main HEAD; then
    # Scope proved these files differ from the common ancestor only in owned
    # version fields. Remove that projection before merging so main's newer
    # version, dependencies and generated contracts merge normally. Advance it
    # again below against the merged release ledger, without rewriting history.
    ancestor=$(git -C "$worktree" merge-base origin/main HEAD)
    while IFS= read -r path; do
      git -C "$worktree" restore --source "$ancestor" --staged --worktree -- "$path"
    done < "$scratch/version-paths"
    if ! git -C "$worktree" diff --cached --quiet; then
      git -C "$worktree" commit -s -m "chore(release): refresh version projection before merging main"
    fi
    git -C "$worktree" merge --no-edit --signoff origin/main
  fi
else
  status=$?
  [[ "$status" == 2 ]] || exit "$status"
  git worktree add --detach "$worktree" origin/main
fi

python3 "$root/scripts/release-contracts.py" record --repo "$root" \
  --directory "$worktree/contracts/releases" --manifest "$scratch/releases.json" \
  "$@" \
  --release-json "$scratch/release.json" --revision "$RELEASE_REVISION" \
  --repository "$GH_REPO" > /dev/null
python3 "$root/scripts/release-contracts.py" check --repo "$root" \
  --directory "$worktree/contracts/releases" --base origin/main > /dev/null
python3 "$root/scripts/product-version.py" advance --repo "$worktree" > /dev/null
python3 "$root/scripts/product-version.py" check --repo "$worktree" > /dev/null
git -C "$worktree" add -- contracts/releases
while IFS= read -r path; do
  git -C "$worktree" add -- "$path"
done < "$scratch/version-paths"
if ! git -C "$worktree" diff --cached --quiet; then
  git -C "$worktree" commit -s -m "chore(release): record $RELEASE_TAG and prepare development version"
fi
gh auth setup-git
git -C "$worktree" push origin "HEAD:refs/heads/$branch"

if [[ "$state" == OPEN ]]; then
  python3 -c 'import json,sys; print(json.load(sys.stdin)["url"])' <<< "$existing"
else
  cat > "$scratch/body.md" <<EOF
Record the published $RELEASE_TAG contract inventory from source $RELEASE_REVISION,
including storage schema, frozen contract descriptors and installer identities.

The release publication and package-verification jobs succeeded before this update.
Merging this PR advances the recorded stable baseline when this is a newer stable release;
it also prepares the next patch development version when main has not already advanced,
keeping all shipped components and lockfiles consistent without lowering a newer planned version.
The Local Control schema and its contract digest follow the same product version.
Previous snapshots remain immutable, including revision-bound archives when an explicitly
authorized unpromoted release is replaced. This inventory does not claim migration acceptance.

Validation: deterministic snapshot generation, frozen-source/index verification and product version consistency.
EOF
  gh pr create --repo "$GH_REPO" --base main --head "$branch" \
    --title "chore(release): record contracts for $RELEASE_TAG" --body-file "$scratch/body.md"
fi
