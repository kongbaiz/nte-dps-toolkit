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

## Browser UI preview

Run `pnpm dev --host 127.0.0.1 --port 5178 --strictPort` and open
`http://127.0.0.1:5178/ui-preview.html?theme=light` in the Codex browser.
The preview uses the production Console components with synthetic data from
`dev/fixtures.ts`. Its banner identifies the simulation. Native actions such as
capture, saving files and controlling plugins are unavailable. This does not
validate Tauri IPC, native windows or live game capture.

Query options: `theme=light|dark`, `language=en`, `density=compact|comfortable`,
`preset=high-contrast|tactical`, `motion=reduced` and timeline `state=empty|error`.
Theme links reload the preview; settings changes are local to this browser.
The HTML entry is excluded from production build inputs. Check the preview with
`pnpm exec tsc -p dev/tsconfig.json` and `pnpm exec oxlint src dev`.

Layout references: [shadcn dashboard](https://ui.shadcn.com/blocks),
[Sidebar Dashboard Inset](https://blocks.so/sidebar/sidebar-02), and
[Stats with Card Layout](https://blocks.so/stats/stats-03). These inform the
inset workspace and metric hierarchy; the implementation uses existing project
components without adding dependencies.
