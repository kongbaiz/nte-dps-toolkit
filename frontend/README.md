# NTE DPS TOOL frontend

Vite + React + TypeScript + Tailwind CSS + shadcn/ui frontend for the Tauri
desktop application.

The current scope is migration phase 1 only: the `hud-spike` technical
validation window. Rust remains the authoritative owner of window state and
publishes an ordered, throttled snapshot Channel.

## Commands

```powershell
pnpm install --frozen-lockfile
pnpm format:check
pnpm lint
pnpm typecheck
pnpm test
pnpm build
pnpm tauri:dev
```

Run these commands from this directory. See
`../docs/TAURI_MIGRATION_PHASE1.md` for the architecture and manual Windows
validation checklist.
