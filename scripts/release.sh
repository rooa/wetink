#!/usr/bin/env bash
# Cut a release: bump the version, test, commit, tag and push.
# The pushed tag triggers .github/workflows/release.yml, which builds every
# target, publishes the GitHub release and updates the Homebrew tap.
#
#   scripts/release.sh 0.2.0      # release v0.2.0
#   scripts/release.sh -n 0.2.0   # dry run: check and test, change nothing
set -euo pipefail
cd "$(dirname "$0")/.."

die() { echo "release: $*" >&2; exit 1; }

dry=0
if [[ ${1:-} == -n ]]; then dry=1; shift; fi
version=${1:-}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || die "usage: scripts/release.sh [-n] X.Y.Z"
tag="v$version"
name=$(sed -n 's/^name = "\(.*\)"/\1/p' Cargo.toml | head -1)
current=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
branch=$(git rev-parse --abbrev-ref HEAD)

[[ -z $(git status --porcelain --untracked-files=no) ]] || die "commit or stash your changes first"
git rev-parse -q --verify "refs/tags/$tag" > /dev/null && die "tag $tag already exists"
git fetch -q origin 2> /dev/null || die "cannot fetch origin ($(git remote get-url origin)); does the repository exist?"
if git rev-parse -q --verify "origin/$branch" > /dev/null; then
  git merge-base --is-ancestor "origin/$branch" HEAD || die "$branch is behind origin/$branch; pull first"
fi
git ls-remote --exit-code --tags origin "refs/tags/$tag" > /dev/null && die "tag $tag already exists on origin"

echo "== $name $current -> $version on $branch"
if [[ $version != "$current" ]]; then
  perl -0pi -e "s/^version = \"\Q$current\E\"/version = \"$version\"/m" Cargo.toml
fi
cargo test -q   # also updates Cargo.lock for the new version

if ((dry)); then
  git --no-pager diff --stat
  git checkout -q -- Cargo.toml Cargo.lock
  echo "== dry run OK: would commit, tag $tag and push $branch + $tag"
  exit 0
fi

if [[ $version != "$current" ]]; then
  git commit -q -m "Release $tag" Cargo.toml Cargo.lock
fi
git tag -a "$tag" -m "$name $version"
git push -q origin "$branch" "$tag"

echo "== pushed $tag; the release workflow is building it now"
echo "   watch:  gh run watch \$(gh run list --workflow release.yml --limit 1 --json databaseId -q '.[0].databaseId')"
echo "   then:   brew upgrade $name   (crates.io, if you use it: cargo publish)"
