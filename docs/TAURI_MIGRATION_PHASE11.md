# Tauri migration phase 11: read-only Mod Studio workspace

## Scope

The accepted HUD remains unchanged. This slice starts the next migration
stage with one new stable `console` window and one page: Mod Studio.

The page is deliberately read-only:

- list `.nte` documents from the existing software-side
  `plugins/nte-mods` workspace;
- show enabled state, line count, and byte count without sending every source
  body in the list response;
- load only the selected source through a separate typed command;
- allow text selection only inside the source preview;
- show explicit loading, empty, workspace-error, and document-error states;
- refresh the workspace without checking for a game installation;
- keep create, edit, save, enable/disable, deployment, runtime logs, and hot
  reload in later independently validated slices.

The existing egui Mod Studio remains the behavior reference.

After the first visual review, the temporary standalone dark-card layout was
replaced with the existing Console and Mod Studio information architecture:

- light Console surface with the grouped left navigation and active Mod Studio
  row;
- page title plus the game-client, refresh, folder, loader-status, and enable
  toolbar positions;
- Explorer, document chrome, getting-started row, capability breadcrumb,
  source editor, status bar, and runtime-console regions;
- line numbers and dependency-free C++ presentation highlighting in the
  read-only source;
- unavailable write, deployment, and loader controls remain visibly disabled
  until their Rust commands are migrated.

This is a visual-alignment correction inside Stage A, not an early
implementation of Stage B or Stage C behavior.

## Rust boundary

The root crate adds a frontend-neutral read model under
`src/core/mod_studio.rs`. Tauri enables the lightweight `desktop` feature,
while the CLI-only feature does not compile the Mod Studio storage module.
Tauri, WebView, React, egui, and window dependencies therefore remain absent
from the CLI graph.

The read path is:

```text
plugins/nte-mods
  -> storage::mod_scripts
  -> core::mod_studio read model
  -> blocking Tauri command worker
  -> versioned serde DTO
  -> runtime-validated TypeScript client
  -> Mod Studio selector and page
```

The index response contains summaries only. A detail command validates the Mod
ID before looking up its source. The workspace is capped at 256 documents and
each source keeps the existing 16 KiB storage limit. Absolute local paths and
filesystem error details remain in Rust logs rather than command responses.

In debug builds, an empty executable-side workspace falls back to the
repository `plugins` directory so the page can be validated with the bundled
development scripts. Release builds continue to use the portable directory
beside the executable.

## Window and capability boundary

- stable label: `console`;
- normal decorated window, 1180 x 760 logical pixels;
- minimum size: 820 x 560 logical pixels;
- no transparency, passthrough, always-on-top, or HUD permissions;
- its capability grants only `core:default`;
- `hud-spike` keeps its separate window-scoped drag permission.

Both labels are centralized in Rust and TypeScript contract modules. React
components do not assemble command names or window labels.

## Frontend

The page uses the existing design tokens and shadcn source components. Alert,
Skeleton, and Empty were added through the already-installed shadcn CLI; no
Node dependency was added.

The Console uses a window-local light token scope so the already accepted dark
HUD tokens remain unchanged. The read-only source renderer produces React text
spans rather than HTML injection and recognizes comments, directives, NTE
macros, keywords, types, strings, numbers, namespaces, and function calls.

The view model:

- keeps the current selection when it still exists after refresh;
- selects the first sorted document otherwise;
- ignores late detail results after the user selects another document;
- scopes a detail failure to the source pane;
- treats an empty workspace as a first-class state.

Monaco is not introduced in this slice. Its package size, worker setup,
license, and Vite integration remain an explicit dependency decision for the
editing slice.

## Automated validation

Run:

```powershell
cargo fmt --check
cargo check
cargo test
cargo check --bin nte-core --no-default-features --features cli
cargo clippy --bin nte-core --no-default-features --features cli -- -D warnings
cargo tree -e normal --no-default-features --features cli

cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings

pnpm --dir frontend lint
pnpm --dir frontend typecheck
pnpm --dir frontend test
pnpm --dir frontend build
```

Focused tests cover bounded workspace projection, invalid IDs, missing
documents, path redaction, DTO versions, duplicate IDs, typed command names,
selection preservation, stale detail rejection, loading/empty/error
projection, stable window routing, source line numbering, representative NTE
declarations, multiline comments, and strings containing comment markers.

## Manual validation gate

Run:

```powershell
pnpm --dir frontend tauri:dev
```

Verify only the new `console` window:

1. Mod Studio opens without requiring either game client or the game-side
   loader.
2. The page follows the existing Console baseline: grouped left sidebar, light
   Mod toolbar, Explorer, editor chrome, blue status bar, and runtime console.
3. The Explorer lists the `.nte` documents from `plugins/nte-mods` in stable
   order with enabled markers and source sizes.
4. Selecting different documents updates the source pane; rapidly alternating
   selections never shows a late source under the wrong filename.
5. Refresh keeps the selected document when it still exists and chooses the
   first document when it was removed.
6. Source code can be selected and copied; title, Explorer rows, badges, and
   surrounding chrome do not become selectable.
7. Line numbers and syntax colors remain aligned while vertically and
   horizontally scrolling long source files.
8. Loading uses skeletons, an empty workspace uses the empty state, and a
   malformed workspace uses the localized error with a retry action.
9. The layout remains usable at the 820 x 560 minimum and at 100%, 125%, and
   150% DPI.
10. Disabled create, save, revert, folder, loader, and enable controls do not
    imply that a write or deployment was completed.
11. The accepted `hud-spike` window still has the same transparency,
    passthrough, Home hotkey, width, height, position, and always-on-top
    behavior.
