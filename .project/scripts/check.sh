#!/usr/bin/env bash
# The one check an agent runs before calling work done. It builds everything,
# runs the whole test suite and the no-host check, and refuses a change that
# only reformats a file. It exits non-zero on the first failure.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 1

fail() {
    echo "CHECK FAILED: $1" >&2
    exit 1
}

# The real-data checks read Last order, the Skills checkout's scripts and
# skills, and the Spumcake-Render checkout, from a machine's own settings
# file, `spumcake-local.env`, found in this repository's folder or any folder
# above it and never committed. Each line is NAME=path; the names are
# LAST_ORDER, SPUMCAKE_SKILLS_SCRIPTS, SPUMCAKE_SKILL_FOLDERS (folders joined
# by `:`), and SPUMCAKE_RENDER. A name whose path is missing is left out, and
# the checks that read it say they were skipped.
root="$PWD"
settings=""
dir="$root"
while [ "$dir" != "/" ]; do
    if [ -f "$dir/spumcake-local.env" ]; then
        settings="$dir/spumcake-local.env"
        break
    fi
    dir="$(dirname "$dir")"
done
unset LAST_ORDER SPUMCAKE_LAST_ORDER SPUMCAKE_SKILLS_SCRIPTS SPUMCAKE_SKILL_FOLDERS SPUMCAKE_RENDER
if [ -n "$settings" ]; then
    while IFS='=' read -r name value; do
        case "$name" in
            LAST_ORDER|SPUMCAKE_SKILLS_SCRIPTS|SPUMCAKE_RENDER) [ -d "$value" ] && export "$name=$value" ;;
            SPUMCAKE_SKILL_FOLDERS) export "$name=$value" ;;
        esac
    done < <(grep -E '^[A-Z_]+=' "$settings")
fi
# Storyboard's checks read the older name until its branch is merged.
[ -n "${LAST_ORDER:-}" ] && export SPUMCAKE_LAST_ORDER="$LAST_ORDER"

echo "== real data"
for name in LAST_ORDER SPUMCAKE_SKILLS_SCRIPTS SPUMCAKE_SKILL_FOLDERS SPUMCAKE_RENDER; do
    if [ -n "${!name:-}" ]; then
        echo "$name: ${!name}"
    else
        echo "$name: not set, so the checks that read it are skipped"
    fi
done

# Where Cargo builds, which several worktrees may share.
target=$(cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])') || fail "cargo metadata"

# A clean tree that already passed with the same real data is not checked
# again: its stamp names the tree and what the real-data checks read.
real_data_state() {
    for name in LAST_ORDER SPUMCAKE_SKILLS_SCRIPTS SPUMCAKE_SKILL_FOLDERS SPUMCAKE_RENDER; do
        echo "$name=${!name:-}"
    done
    for folder in "${LAST_ORDER:-}" "${SPUMCAKE_SKILLS_SCRIPTS:-}" "${SPUMCAKE_RENDER:-}"; do
        [ -n "$folder" ] || continue
        git -C "$folder" rev-parse HEAD 2>/dev/null
        git -C "$folder" status --porcelain 2>/dev/null
    done
}
stamp=""
if [ "${1:-}" != "--again" ] && [ -z "$(git status --porcelain)" ]; then
    key="$(git rev-parse 'HEAD^{tree}')-$(real_data_state | sha256sum | cut -c1-16)"
    stamp="$target/checks-passed/$key"
    if [ -f "$stamp" ]; then
        cat "$stamp"
        echo "This tree already passed with the same real data, so it is not checked again (--again checks it anyway)."
        echo "All checks passed."
        exit 0
    fi
fi

echo "== build"
cargo build --workspace 2>&1 | tail -3
[ "${PIPESTATUS[0]}" -eq 0 ] || fail "cargo build"

echo "== tests (the whole workspace)"
mkdir -p target/tmp
out=$(TMPDIR="$PWD/target/tmp" cargo test --workspace 2>&1)
status=$?
echo "$out" | grep -E '^test result|FAILED|panicked' | grep -v 'ok\. 0 passed; 0 failed' || true
[ "$status" -eq 0 ] || fail "cargo test (run it to see the failures)"
passed=$(echo "$out" | awk '/^test result/ {n += $4} END {print n}')
echo "$passed tests passed"

echo "== no-host check"
"$target/debug/storyboard-client" >/dev/null 2>&1 || fail "storyboard-client"

if [ -n "${SPUMCAKE_RENDER:-}" ]; then
    echo "== Spumcake-Render ($(git -C "$SPUMCAKE_RENDER" rev-parse --short HEAD))"
    render_key="$(git -C "$SPUMCAKE_RENDER" rev-parse 'HEAD^{tree}')-$(git -C "$SPUMCAKE_RENDER" status --porcelain | sha256sum | cut -c1-16)-$(echo "${LAST_ORDER:-}" | sha256sum | cut -c1-16)"
    render_stamp="$target/checks-passed/render-$render_key"
    if [ -f "$render_stamp" ] && [ -z "$(git -C "$SPUMCAKE_RENDER" status --porcelain)" ]; then
        echo "unchanged since it passed: $(cat "$render_stamp")"
    else
        render_out=$(cd "$SPUMCAKE_RENDER" && cargo test --workspace 2>&1)
        render_status=$?
        echo "$render_out" | grep -E 'FAILED|panicked' || true
        [ "$render_status" -eq 0 ] || fail "Spumcake-Render's cargo test (run it there to see the failures)"
        render_passed=$(echo "$render_out" | awk '/^test result/ {n += $4} END {print n}')
        echo "$render_passed tests passed"
        mkdir -p "$target/checks-passed"
        echo "$render_passed tests passed on $(date -I)" > "$render_stamp"
    fi
fi

echo "== reformatted files"
# A changed Rust file that equals the last commit's version run through
# rustfmt was reformatted, not edited. Each is formatted inside a copy of its
# folder, so the modules it declares are found.
edition=$(grep -m1 -E '^edition' Cargo.toml | cut -d'"' -f2)
scratch=$(mktemp -d -p target/tmp)
noise=""
for file in $(git diff --name-only HEAD -- '*.rs'); do
    [ -f "$file" ] || continue
    git cat-file -e "HEAD:$file" 2>/dev/null || continue
    rm -rf "${scratch:?}"/*
    git archive HEAD "$(dirname "$file")" | tar -x -C "$scratch"
    rustfmt --edition "${edition:-2024}" "$scratch/$file" >/dev/null 2>&1 || continue
    if cmp -s "$scratch/$file" "$file"; then
        noise="$noise $file"
    fi
done
rm -rf "$scratch"
[ -z "$noise" ] || fail "reformatted only, restore with git checkout --$noise"

echo "== changed since the last commit"
git status --short
if [ -n "$stamp" ]; then
    mkdir -p "$target/checks-passed"
    echo "Passed on $(date -I) at $(git rev-parse --short HEAD): $passed tests." > "$stamp"
fi
echo "All checks passed."
