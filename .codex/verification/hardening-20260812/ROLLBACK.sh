#!/usr/bin/env bash
set -eu
printf 'branch=fix/a-group-hardening\nfield=version_sync_0_3_8\nstate=BASELINE\nvalue=desktop-version-0.3.7\n' > "$(dirname "$0")/rollback-fixture.txt"
printf 'ROLLBACK_RESULT=restored\n'
