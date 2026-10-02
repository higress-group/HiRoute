#!/usr/bin/env bash
set -euo pipefail

: "${RELEASE_TAG:?published release tag required}"
: "${RELEASE_REVISION:?verified source revision required}"
: "${WEBSITE_REVISION:?verified release-manifest revision required}"
: "${GH_REPO:?repository required}"

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
git fetch origin main --tags
# Fast path after the generated PR has already merged.
git worktree add --detach "$worktree" origin/main
python3 "$root/scripts/release-contracts.py" record --repo "$root" \
  --directory "$worktree/contracts/releases" --manifest "$scratch/releases.json" \
  --release-json "$scratch/release.json" --revision "$RELEASE_REVISION" \
  --repository "$GH_REPO" > /dev/null
if [[ -z $(git -C "$worktree" status --porcelain -- contracts/releases) ]]; then
  echo 'Published contract snapshot is already recorded on main.'
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
if git ls-remote --exit-code --heads origin "$branch" > "$scratch/remote-ref"; then
  git fetch origin "$branch:refs/remotes/origin/$branch"
  changed=$(git diff --name-only "origin/main...origin/$branch")
  if [[ -n "$changed" ]] && ! python3 -c \
    'import sys; raise SystemExit(any(not p.startswith("contracts/releases/") for p in sys.stdin.read().splitlines()))' <<< "$changed"; then
    echo 'Existing automation branch contains unrelated changes.' >&2
    exit 1
  fi
  git worktree add --detach "$worktree" "origin/$branch"
  git -C "$worktree" merge --no-edit --signoff origin/main
else
  status=$?
  [[ "$status" == 2 ]] || exit "$status"
  git worktree add --detach "$worktree" origin/main
fi

python3 "$root/scripts/release-contracts.py" record --repo "$root" \
  --directory "$worktree/contracts/releases" --manifest "$scratch/releases.json" \
  --release-json "$scratch/release.json" --revision "$RELEASE_REVISION" \
  --repository "$GH_REPO" > /dev/null
python3 "$root/scripts/release-contracts.py" check --repo "$root" \
  --directory "$worktree/contracts/releases" --base origin/main > /dev/null
git -C "$worktree" add -- contracts/releases
if ! git -C "$worktree" diff --cached --quiet; then
  git -C "$worktree" commit -s -m "chore(release): record contracts for $RELEASE_TAG"
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
previous snapshots remain immutable. This inventory does not claim migration acceptance.

Validation: deterministic snapshot generation and frozen-source/index verification.
EOF
  gh pr create --repo "$GH_REPO" --base main --head "$branch" \
    --title "chore(release): record contracts for $RELEASE_TAG" --body-file "$scratch/body.md"
fi
