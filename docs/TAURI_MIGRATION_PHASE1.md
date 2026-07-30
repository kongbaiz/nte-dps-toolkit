# Tauri migration phase 1: HUD technical validation

## Scope

This phase adds one isolated `hud-spike` window and leaves every existing egui
entry intact. After the first technical dashboard failed visual acceptance, the
window was reduced to a backing-less transparency and dragging baseline. It
validates the desktop boundary before any product page is migrated:

- Tauri 2 shell and WebView2 window;
- Vite + React + TypeScript frontend;
- Tailwind CSS and selected shadcn/ui primitives, without a full-window surface;
- typed request/response commands;
- ordered, change-driven Rust-to-React Channel snapshots with a 100 ms
  coalescing window and string 64-bit counters;
- explicit subscription cleanup;
- transparent, undecorated, resizable HUD window with haloed text only in the
  painted content area;
- a dedicated drag line that explicitly calls `startDragging()` with a
  window-scoped capability, separate from interactive controls;
- Rust-owned always-on-top and mouse-passthrough state;
- the saved passthrough hotkey (Home by default) restores edit mode even while
  the game is foreground;
- one shared simplified-Chinese source at `res/languages/zh-CN.json`.

The Channel carries a small aggregate technical snapshot. It does not carry
packets, hits, frames, `CombatState`, pointers, or window handles.

The reduced transparency and dragging baseline was accepted by the user on
2026-07-30. Phase 2 now extends the same snapshot with a read-only HUD
projection; see `docs/TAURI_MIGRATION_PHASE2.md`.

## Directory boundary

```text
src/                       existing Rust domain and system core
src-tauri/src/commands/    finite typed request/response adapters
src-tauri/src/channels/    ordered stream registration and cleanup
src-tauri/src/windows/     stable window labels and native window operations
src-tauri/src/state.rs     thread-safe adapter state and stream registry
frontend/src/lib/tauri/    runtime-validated TypeScript contract and client
frontend/src/features/     React projection for the one validation window
frontend/src/routes/       stable window-label registration
```

`src-tauri` links the root crate with `default-features = false`. Tauri,
WebView, and Node dependencies remain outside the root crate's CLI feature
graph.

## Added dependency rationale

| Dependency                                             | Purpose                                                                              | Maintenance/license                      | Rejected alternative                               |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------ | ---------------------------------------- | -------------------------------------------------- |
| Tauri 2                                                | Desktop shell, ordered Channel, and native window operations                         | Official Tauri ecosystem, MIT/Apache-2.0 | Reimplementing WebView2 and native window plumbing |
| React 19 + Vite 8 + TypeScript                         | Component UI, fast local builds, strict contracts                                    | Mainstream maintained projects, MIT      | Continuing to grow the egui presentation layer     |
| Tailwind CSS 4 + selected shadcn/ui/Base UI components | Local design tokens and accessible primitives whose source remains in the repository | Maintained upstreams, MIT                | Adding a large opaque component framework          |
| Vitest + Prettier + Oxlint                             | Contract tests, deterministic formatting, linting                                    | Maintained JavaScript tooling, MIT       | Untested handwritten IPC and ad-hoc formatting     |

The lockfiles pin the resolved dependency graph. No global state framework,
router framework, async Rust runtime, or second i18n store was added.

## Automated validation

From the repository root:

```powershell
cargo fmt --check
cargo check
cargo test

cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml

pnpm --dir frontend format:check
pnpm --dir frontend lint
pnpm --dir frontend typecheck
pnpm --dir frontend test
pnpm --dir frontend build

cargo check --bin nte-core --no-default-features --features cli
cargo tree -e normal --no-default-features --features cli
```

For the Tauri packaging path without producing installers:

```powershell
pnpm --dir frontend tauri:build -- --debug --no-bundle
```

## Manual Windows validation gate

Run:

```powershell
pnpm --dir frontend tauri:dev
```

Then verify this single window:

1. **Transparency:** text and local controls are visible while the rest of the
   WebView has no rectangular white/black background, titlebar residue, blur,
   gradient, or corner artifacts.
2. **Drag and resize:** drag the line with the grip icon, confirm the refresh
   button does not start dragging, resize rapidly from the left and right
   edges, and confirm the content-owned height stays locked.
3. **Ordered Channel:** the sequence increases and uptime advances without UI
   flicker; refresh advances state without resetting the stream.
4. **Always on top:** toggle it off/on and compare the HUD with another normal
   window.
5. **Mouse passthrough:** press Home to enable it, confirm clicks reach the
   window behind the HUD, press Home again while the game is foreground, and
   confirm the HUD is interactive again without an auxiliary window.
6. **DPI and monitors:** move the HUD between monitors with different scaling,
   checking sharpness, hit targets, drag behavior, and size.
7. **Language and error surface:** visible copy is simplified Chinese; if the
   Rust bridge reports an error, the page shows a retry action rather than a
   blank window.

The basic transparency and dragging observations passed. Keep recording any
GPU, WebView2, cross-DPI, multi-monitor, or passthrough discrepancy because the
final HUD replacement gate remains stricter than this phase-1 confirmation.

The equivalent HUD contract, staged delivery order, and HTML-in-Canvas hybrid
component boundary are documented in `docs/TAURI_HUD_MIGRATION_PLAN.md`.
