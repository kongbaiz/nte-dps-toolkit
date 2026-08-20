#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
patch_file="$script_dir/DIFF_FILE"
target="${1:?usage: ROLLBACK.sh <repository-copy>}"
expected_head="12ef5e865bbfb843f1814fd9c9d4b470034c5841"

test -f "$patch_file"
git -C "$target" rev-parse --is-inside-work-tree >/dev/null
actual_head="$(git -C "$target" rev-parse HEAD)"
test "$actual_head" = "$expected_head"

git -C "$target" apply --reverse --check "$patch_file"
git -C "$target" apply --reverse "$patch_file"

test -z "$(git -C "$target" status --porcelain --untracked-files=all)"
grep -Fq 'packet_emission: PacketEmissionMode::FullDebug,' \
  "$target/src-tauri/src/state.rs"

echo "ROLLBACK_OK head=$actual_head status=clean restored=packet_emission:PacketEmissionMode::FullDebug"
