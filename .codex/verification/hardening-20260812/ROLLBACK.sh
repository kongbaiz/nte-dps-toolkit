#!/usr/bin/env bash
set -eu
printf 'branch=fix/a-group-hardening\nfield=release_notes_v0_3_8\nstate=BASELINE\nvalue=docs/releases/0.3.7.md\n' > "$(dirname "$0")/rollback-fixture.txt"
printf 'ROLLBACK_RESULT=restored\n'
