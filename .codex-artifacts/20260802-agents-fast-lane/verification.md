# AGENTS fast-lane verification

## Baseline

Command: `Get-Content .codex-artifacts/20260802-agents-fast-lane/original/AGENTS.md -Raw` plus literal rule probes.

```text
FAST_LANE_HEADING=0
FULL_MATRIX_DEFAULT=1
FAST_REPLY_RULE=0
```

Exit status: `0`

## Modified

Command: `Get-Content AGENTS.md -Raw` plus literal rule probes.

```text
FAST_LANE_HEADING=1
FULL_MATRIX_DEFAULT=0
FAST_REPLY_RULE=1
MARKDOWN_FENCES=20
```

Exit status: `0`

## Structural checks

Commands: `git diff --check -- AGENTS.md` and `git apply --check --reverse --ignore-space-change --ignore-whitespace change.patch`

```text
DIFF_CHECK_EXIT=0
MARKDOWN_STRUCTURE_EXIT=0
PATCH_REVERSE_CHECK_EXIT=0
ROLLBACK_RESTORED=1
ROLLBACK_HASH_MATCH=1
MODIFIED_HASH_MATCH=1
```

Exit status: `0`
