#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
patch_file="$script_dir/DIFF_FILE"
target="${1:?usage: ROLLBACK.sh <repository-copy>}"
expected_head="86d21c622910f0007bb3ef9d02a98574d39708c0"

test -s "$patch_file"
git -C "$target" rev-parse --is-inside-work-tree >/dev/null
actual_head="$(git -C "$target" rev-parse HEAD)"
test "$actual_head" = "$expected_head"

git -C "$target" apply --reverse --check "$patch_file"
git -C "$target" apply --reverse "$patch_file"

test -z "$(git -C "$target" status --porcelain=v1 --untracked-files=all)"
test "$(git -C "$target" rev-parse HEAD)" = "$expected_head"

echo "ROLLBACK_OK head=$actual_head status=clean restored=baseline"