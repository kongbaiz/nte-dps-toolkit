# Tauri migration phase 18: complete Software Update transactions

## Scope

This phase completes the Software Update category in Console Settings. It keeps
the existing signed manifest, component comparison, package verification,
transactional installer, Mods Plugin rollback, and updater health protocol in
Rust. React only renders the versioned projection and sends typed component
intents.

## Connected behavior

- `SETTINGS_CONTRACT_VERSION = 4` projects every available application and Mods
  Plugin component with version, publication time, release notes, decimal-string
  artifact size, active download progress, prepared component, and stable
  install blocking reason.
- Startup and manual checks reuse the official signed manifest, installed app
  and Mods Plugin version comparison, WinHTTP system proxy/PAC/direct fallback,
  and bounded manifest download.
- The automatic-download preference starts preparation of the first verified
  component returned by the signed manifest. Manual and automatic downloads use
  the same Rust operation and a 100 ms minimum progress publication interval.
- Application packages retain declared-size, SHA-256, archive-layout, updater,
  and transaction-file validation. Installation is enabled only after capture
  stops; the external updater is launched before the Tauri process exits.
- Mods Plugin packages retain size, hash, single-file archive, backup, atomic
  replacement, installed-version state, deployment refresh, and rollback
  semantics. Installation is disabled while the game process is running.
- Startup records the updater health marker and schedules completed transaction
  cleanup through the existing storage implementation.
- Network, manifest, signature, file, updater, process-probe, and deployment
  details stay in Rust logs. The WebView receives stable status and error keys.

## Automated coverage

- Rust Settings projection covers available, downloading, progress, prepared,
  installability, and generation updates.
- TypeScript rejects duplicate components, invalid component identifiers,
  unsafe byte representations, progress greater than the declared total, and
  inconsistent prepared/install state.
- Typed client tests cover component download and prepared-update installation
  command names and camel-case arguments.

## Manual acceptance gate

1. Check with direct networking, a Windows proxy/PAC, and no network; confirm
   checking, current, available, not-configured, and error states settle through
   the mounted Settings channel.
2. Use a manifest containing application-only, Mods-Plugin-only, and both
   components; compare versions, release notes, publication time, and size.
3. Download each component manually and with automatic download enabled. Confirm
   byte progress is monotonic and the install button appears only after package
   verification and preparation finish.
4. Corrupt the declared size, SHA-256, and archive structure independently.
   Confirm preparation fails, no install button appears, and retry remains
   available without exposing internal paths or transport details.
5. Keep capture starting/running/stopping and confirm application installation
   stays disabled. Stop capture, install, confirm the external updater replaces
   the application, restarts it, writes the health marker, and cleans completed
   transaction files.
6. Keep the game process running and confirm Mods Plugin installation stays
   disabled. Close it, install, confirm the installed version advances and all
   managed game copies refresh. Force replacement or deployment failure and
   confirm the previous plugin and component-version state are restored.
7. Repeat in English, Japanese, and Simplified Chinese at 820 px and 1440 px,
   100%/125%/150% DPI, and verify long release notes wrap without clipping the
   progress or action controls.
