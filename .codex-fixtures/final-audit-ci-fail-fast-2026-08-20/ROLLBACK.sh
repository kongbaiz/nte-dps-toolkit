#!/usr/bin/env bash
set -euo pipefail

artifact_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
default_root="$(cd "$artifact_dir/../.." && pwd -P)"
target_root="${1:-$default_root}"
target_root="$(cd "$target_root" && pwd -P)"

if [[ ! -d "$target_root/.github/workflows" || ! -d "$target_root/scripts" ]]; then
  printf 'ROLLBACK_ERROR invalid target root: %s\n' "$target_root" >&2
  exit 2
fi

cp "$artifact_dir/build.baseline.yml" "$target_root/.github/workflows/build.yml"
rm -f "$target_root/scripts/test_ci_powershell_fail_fast.ps1"

expected="$(awk '{print $1}' "$artifact_dir/ORIGINAL_SHA256.txt")"
actual="$(sha256sum "$target_root/.github/workflows/build.yml" | awk '{print $1}')"
if [[ "$actual" != "$expected" ]]; then
  printf 'ROLLBACK_ERROR hash mismatch expected=%s actual=%s\n' "$expected" "$actual" >&2
  exit 3
fi

printf 'ROLLBACK_OK target=%s build_sha256=%s gate_script=absent\n' "$target_root" "$actual"
