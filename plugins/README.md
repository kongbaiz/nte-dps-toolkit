# Marketplace-managed Toolkit components

Plugin mode downloads all components from the signed Mod Market. Desktop CI no
longer embeds an optional Toolkit bundle. Paths are relative to the running
application executable, never its working directory or a user-selected path:

```text
mods/
  game/
    d3d12.dll
    plugins/<plugin-id>.dll
  tools/NTE-Loader.exe
  driver/uetools.sys
```

The home page displays the active data mode explicitly. The host control panel selects
proxy/Loader loading; host and plugin management exposes only status, load and
unload. Plugin-owned features keep their existing capability, idle and confirmation
gates. Host shutdown stops services; it does not claim the DLL was unmapped.

Loader uses the managed runtime and explicit managed driver path. Proxy loading
requires the game to be closed, discovers exactly one installation through the
existing registry probes, validates every candidate, stages complete files on the
destination volume and publishes the host last. Existing identical files are a
no-op; different files, foreign plugins and ambiguous installations fail closed.
It never overwrites old/unknown proxies, migrates loose files or deletes user files.
After successful deployment the user starts the game and checks real host status.

## Signed catalog supply contract

Catalog schema 6 requires each entry to declare `component`: `plugin`, `host`,
`loader` or `driver`. The latter three have fixed IDs `nte-host`, `nte-loader` and
`uetools-driver`; arbitrary paths and alternative IDs are rejected. Artifacts use
`<id>-<semver>.dll`, `.exe` or `.sys` under `https://dps.o-na-ni.com/mods/v3/packages/`. The signed catalog is
`https://dps.o-na-ni.com/mods/v3/catalog.json`; the legacy v1 address is retained.
Schema 5 remains accepted for plugin-only catalogs; it cannot supply host/tool/driver
files. Signatures, pinned public key and SHA-256/size verification are unchanged.
PE machine, optional header, DLL flag and subsystem must match the declared kind.
Each component is atomically replaced only after verification; failed downloads or
validation leave the existing component intact. Downloads do not start a host,
driver, game process or plugin. Schema 6 and all seven Release components were published at `/mods/v3/` on
2026-09-27. The previous `/mods/v1/` catalog and packages remain unchanged.
Latest publication: `7ef265cd5a32cb25efeb15d367337277cf95649b57c2b3dfced49f968d8cd977`;
source base: `1747d2ff7774396299f3f65f0dfb7822d10b8a38`, with the working-tree
changes recorded by SHA-256 `885274408389ace5f14d36611925013552d5d2865117440a7037e2afbd58d91e`.
The previous release and all previously published package URLs are retained.
The staged and public catalogs both passed the Rust signature/package verifier.
This confirms distribution integrity, not real-game or kernel-loader acceptance.

To prepare a later candidate, run `scripts/build_mod_market_release.py` with the
verified Release directory, a new output directory and its source commit. Sign
`payload.json` on the existing release host; never export its private key. Run
`staged_catalog_and_every_package_pass_local_verification` with
`NTE_MOD_MARKET_STAGED_DIRECTORY` before switching the server's `current` link.
Preserve older versioned routes and releases, then run
`official_catalog_and_every_package_pass_local_verification` against HTTPS.

UE Tools and Mod Loader source belong to the private UE Tools workspace. The old
native Mod plugin source has been removed; public CI does not build or package it
or Mod Loader. This does not remove existing files from users' game directories.

`mods-plugin.version`, `nte-mods.enabled`, `nte-mods/*.nte` and `examples/` are
legacy v7 compatibility assets still referenced by Rust code/tests. The example
`query_mod_events.py` requires an external old runtime exposing the v7 named pipe.
These scripts cannot be installed as Toolkit v1 plugins.

See the [legacy reference audit](../docs/LEGACY_MOD_REFERENCES.md) for remaining
runtime callers and the distinction between the old IPC and Toolkit v1.

## Native clock and Abyss stage adaptation

Timing follows the capture source: packet capture keeps wall time; the native
plugin supplies pause-adjusted time. `gameClock.status=awaiting_outgoing_damage`
with a nonempty, boundary-valid transition history means the provider is ready,
not that a removed pause setting must be enabled. Actual missing/invalid pause
observations still degrade visibly; selecting Plugin never fabricates pause data.

The live Capture pipe advertises `combat.abyss.v2` (and legacy v1) and sends
`event.combat.abyss` in the same bounded FIFO as hits. The new client explicitly
requests `abyssEvents:true, abyssSchema:3` with `combat.abyss.restart_scope.v1`
and checks the start receipt; old clients are not sent
unknown notification types. Each notification retains
provider/capture identity and the shared sequence; lifecycle rows do not consume
hit IDs. The Rust stream adapter routes them through the existing Abyss reducer,
including while the first clock baseline is pending, without reordering hits or
backfilling earlier unclassified damage. Overflow and malformed lifecycle data
are explicit failures. Older producers without this capability are rejected rather
than silently reporting an unknown half as if detection were working.

Sources are delivered local-player notifications, not UI buttons: the existing
Abyss data-layer half delegate; `HTPlayerState.ClientAbyssCloneResponse` after the
game decodes its parameters into the player's `AbyssGamePlayData`; and the current
clone state after replication. The generic `ClientSyncCloneCustomData` is not the
Abyss synchronization path. Every reflected member/type/size is checked.
A temporary data-layer `None` during restart is not treated as leaving Abyss.
At capture startup and at most once per second on the existing game-thread pulse,
the current clone state supplements missed pre-capture notifications with a
`location` event: a known Abyss floor does not imply a known half. The environment
snapshot exposes `abyssRuntime.status` and nullable half for read-only diagnosis.
Current-state reads resolve the player-state storage from the SDK response
implementation's parameter destination and `OnAbyssFightStageChanged` source;
no game RVA or player-field offset is hardcoded. Runtime reflection validates
`FAbyssGamePlayData`, and clone identity must match. `GetAbyssFirstDataLayerLoaded`
is only a loaded flag, not a half enum, and is not used to classify halves.
Unresolved bindings stay unknown. A read-only live check confirmed floor 11,
FirstHalf in the matching session; transitions/retries/exit still require their
own live acceptance rather than being implied by that snapshot.

## Runtime maintenance and prebattle skills

Challenge restart monitoring additionally requires `combat.rounds.v1` and an
acknowledged `roundEvents:true` start option. AdvVision/DiyBoss reset requests are
paired with delivered reset confirmations for the same frozen clone identity and
capture generation. The consumer archives and clears the old battle through the
existing core round transaction, not a React-local reset. Abyss repeats use the
explicit restart request plus stage response/read-back rather than only a rising
`IsReChallenge` edge. Current-half retries have their own `restart_half` event:
the other half is retained, not archived away with the entire floor. Whole-floor
resets still archive/clear both halves. Live mode-by-mode acceptance remains required.

The host reclaims stale `.loaded` shadow DLLs at startup only after their owner
process is confirmed exited. Live/unqueryable owners, locked files, unknown names
and reparse points are retained; original plugin files are never swept. Loaded
copies remain necessary for safe DLL updates and host import binding.

Combat exposes Toolkit commands 120/121 for prebattle skill-unlock status/request.
The desktop control requires confirmation, binds the current host identity and
uses advertised capabilities. It reuses the narrow native operation, does not
reset cooldowns or energy and never treats a queued request as confirmed success.
The updated host and Combat plugin must both be installed and loaded.
