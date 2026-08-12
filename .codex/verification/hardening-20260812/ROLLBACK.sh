#!/usr/bin/env bash
set -euo pipefail
target="${1:?target path required}"
printf '%s\n' \
  'branch=fix/a-group-hardening' \
  'field=take_battle_preserving_inventory' \
  'state=BASELINE' \
  'value=inventory-preserved' > "$target"
printf '%s\n' 'ROLLBACK_RESULT=restored'
