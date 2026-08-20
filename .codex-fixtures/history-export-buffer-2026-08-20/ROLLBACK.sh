#!/usr/bin/env bash
set -euo pipefail
target="${1:?target copy path is required}"
baseline="${2:?baseline path is required}"
cp -- "$baseline" "$target"
