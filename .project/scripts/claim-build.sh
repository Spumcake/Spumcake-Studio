#!/usr/bin/env bash
# Claims the build folder for this worktree before any cargo command in a
# turn. Worktrees may share one build folder, and Cargo gives a workspace's
# own crates the same names in every worktree, so a build after another
# worktree's could link that worktree's crates. When the folder last built a
# different worktree, this removes the workspace's own crates from it, keeping
# every dependency, so the next build compiles this worktree's sources. Run it
# first in each turn; check.sh runs it itself.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 1
meta=$(cargo metadata --format-version 1 --no-deps 2>/dev/null) || { echo "claim-build: cargo metadata failed" >&2; exit 1; }
target=$(printf '%s' "$meta" | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
mine="$PWD"
last_file="$target/last-worktree"
last=$(cat "$last_file" 2>/dev/null || true)
if [ -n "$last" ] && [ "$last" != "$mine" ]; then
    members=$(printf '%s' "$meta" | python3 -c 'import json, sys; print(" ".join("-p " + p["name"] for p in json.load(sys.stdin)["packages"]))')
    echo "claim-build: the build folder last built $last; clearing this workspace's own crates"
    # shellcheck disable=SC2086
    cargo clean $members >/dev/null 2>&1 || true
fi
mkdir -p "$target"
printf '%s\n' "$mine" > "$last_file"
