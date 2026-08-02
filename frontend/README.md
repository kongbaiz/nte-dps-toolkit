# NTE DPS TOOL frontend

Vite + React + TypeScript + Tailwind CSS + shadcn/ui frontend for the Tauri
desktop application.

This is the production desktop UI. Rust remains the authoritative owner of
domain and window state, while React consumes typed commands and ordered,
throttled snapshot Channels through the Tauri adapter.

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

Run these commands from this directory. See `../AGENTS.md` for the current
architecture boundaries and manual Windows validation checklist.
