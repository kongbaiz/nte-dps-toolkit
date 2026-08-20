#!/usr/bin/env bash
set -euo pipefail

artifact_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
default_root="$(cd "$artifact_dir/../.." && pwd -P)"
target_root="${1:-$default_root}"
target_root="$(cd "$target_root" && pwd -P)"

if [[ ! -d "$target_root/src/engine" ]]; then
  printf 'ROLLBACK_ERROR invalid target root: %s\n' "$target_root" >&2
  exit 2
fi

cp "$artifact_dir/model.baseline.rs" "$target_root/src/engine/model.rs"

expected="$(awk '{print $1}' "$artifact_dir/ORIGINAL_SHA256.txt")"
actual="$(sha256sum "$target_root/src/engine/model.rs" | awk '{print $1}')"
if [[ "$actual" != "$expected" ]]; then
  printf 'ROLLBACK_ERROR hash mismatch expected=%s actual=%s\n' "$expected" "$actual" >&2
  exit 3
fi

printf 'ROLLBACK_OK target=%s model_sha256=%s dense_candidate_gate=restored-baseline\n' "$target_root" "$actual"
