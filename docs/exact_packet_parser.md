# Exact packet parser rewrite candidate

## Status

The worktree now routes both real live capture and PCAP import through the exact
runtime adapter and the shared reducer. It no longer falls back to legacy packet
damage when the exact profile is absent or decoding fails. Inventory, equipment,
packet diagnostics and existing combat-clock paths remain independent.

This is a source implementation in the isolated worktree, not an installed
release. Automatic class/channel/version discovery is still NOT certified.
A matching explicit profile is required; absence produces a visible warning and
no packet damage, rather than plausible legacy results.

## Modules

- `src/engine/settlement/wire.rs`: bounded native v6 request and settlement RPC
  grammars, complete payload consumption, full transmitted actor references,
  request health snapshots, server damage components and current HP.
- `src/engine/settlement/transport.rs`: explicit-profile packet/Bunch parsing,
  bounded fragment reassembly and actor content/RPC framing. It does not search
  arbitrary offsets for a plausible damage pattern.
- `src/engine/settlement.rs`: connection/generation-local exact message ledger,
  source/target joins, catalog consensus, deduplication and conflict quarantine.
- `examples/replay_settlement.rs`: offline raw-PCAPNG or bounded-RPC acceptance
  harness; native CombatEvidence is never an input.
- `src/engine/settlement/tests.rs`: 15 synthetic behavior tests. They are not
  substitutes for the separately evaluated real captures.

## Accounting contract

Requests alone never create damage. Settlements provide amount, per-component
display category, source and target references, and current HP. Request metadata
only enriches the exact matching message/source/target, supplying the configured
statistical skill and request-time HP snapshots. Raw Float32 bits are retained.
Critical/element/activation identity are not guessed.

All transmitted identity fields are retained, including tails that distinguish
same-species targets. Target array order is not used to match request targets.
Same-frame or same-amount components are not merged.

`Change` replaces the projection for one `MessageKey`; it is NOT an append-only
hit event. A late request enriches the existing projection. A conflicting
settlement retracts and quarantines that message. A conflicting request removes
request-derived enrichment but preserves the observed settlement damage.
Identical retransmissions have no effect. A bounded ledger fails explicitly at
capacity rather than evicting dedup identities and silently recounting traffic.

The caller must give each connection/generation its own decoder and ledger.
The message key alone is not a cross-connection or cross-generation identity.

## Replaying

```text
cargo run --no-default-features --features cli --example replay_settlement -- PCAPNG PROFILE_JSON CATALOG_JSON OUTPUT_JSON
cargo run --no-default-features --features cli --example replay_settlement -- RPC_JSON CATALOG_JSON OUTPUT_JSON
```

The explicit capture profile supplies the local/server endpoints, observed
component prefix, channel and build-specific RPC field indices. The decoder does
not select a schema based on parse success. The catalog input is the established
asset relationship catalog. Unresolved GE candidates are retained as unresolved,
not dropped to manufacture a unique skill mapping.

The PCAP harness validates the input through the existing import preflight and
supports the observed Ethernet/IPv4/UDP capture form. Unsupported IP fragments,
link types, new dynamic-actor spawn headers, nonempty recovery/extra-damage
arrays, malformed streams and incomplete fragments return explicit errors.
It does not claim complete decoding of replication property blocks.

## Initial standalone-core acceptance

Five separate capture/build profiles were replayed directly from raw PCAPNG by
the Rust executable; the Python decoder was not part of this replay path.

|Capture|Request RPCs|Settlement RPCs|Damage components|
|---|---:|---:|---:|
|2026-09-24 00:22|211|211|236|
|2026-09-24 00:35|379|379|373|
|2026-09-24 11:23|435|435|428|
|2026-09-24 14:10|48|45|91|
|2026-09-24 14:29|44|44|64|
|Total|1117|1114|1192|

All 1192 components match the previously sealed packet predictions and native
comparison references for amount/category, complete source/target references,
statistical skill, current HP and maximum HP. The 14:10 capture includes three
additional bounded request envelopes with opaque inner prediction-key payloads;
they produce no settlement damage, and are not declared semantically decoded.

The final candidate output was replayed after the last semantic change and
read back. Its output exactly matches the fully evaluated version for all five
samples. Production promotion remains false: supplied profile qualification is
not the same as automatic class/channel bootstrap for arbitrary captures.

Initial standalone-core validation (superseded by migration results below):

- CLI `cargo check`: passed.
- CLI/desktop clippy with `--all-targets -- -D warnings`: passed.
- `cargo fmt --check`, `git diff --check`: passed.
- New parser tests: 15 are included and passed in the final full runs.
- Final CLI full test invocation: 742 passed, 1 failed, 8 ignored; exit 101.
- Final desktop full test invocation: 824 passed, 1 failed, 12 ignored; exit 101.
- Final Tauri `cargo check --manifest-path src-tauri/Cargo.toml`: passed.
  No Tauri runtime/UI test was performed.

Both full test failures are
`storage::i18n::tests::japanese_locale_covers_every_simplified_chinese_key`:
the Japanese locale lacks three existing preference labels. The same focused
test was run against the original unchanged checkout and failed identically.
No unrelated locale data or test assertions were changed to hide the failure.

## Application integration completed

- `EngineEvent::ExactSettlement` carries connection/generation-qualified
  replace/retract projections through the one shared reducer.
- `Hit.exact` persists complete source/target identity, message/timestamp strings,
  component identity and raw HP snapshots. Conflict quarantine survives history
  serialization/restoration. Old records remain readable without this field.
- Late metadata changes skill/HP projection in place. Identical updates are
  no-ops. Source, amount, category, target and first-settlement time are frozen.
- Round cut/reset retires old message identities, so old enrichment cannot become
  new-round damage. Half membership is frozen at first settlement. New damage
  first observed after a cut belongs to the new projection; requests alone do
  not create a damage record.
- New rows bypass legacy request accounting, source reassignment, snapshot
  deduplication, HP-residual corrections and inferred overkill. Old helpers
  remain for legacy/reference tests and unrelated shared functionality, not as
  a fallback for the new packet path.
- New exact category-22 records preserve the server-credited character in
  personal totals; historical legacy rows retain their older shared-category
  accounting. The parser does not infer which teammates filled the stagger bar.
- Known current HP is retained when maximum HP is missing. Unavailable HP,
  overkill and max-HP reduction are null in public DTOs, not invented zeroes.
  Main DPS detail contract is v8; CLI battle read contract is v6. The frontend
  shows unknown HP instead of a fabricated zero/death state.
- Outgoing activity/idle revision is updated for new exact settlements only,
  not duplicate or late enrichment events. Packet hit counters use newly
  decoded settlement components, not request-side prediction values.
- `serde_json` now enables its existing `float_roundtrip` feature. The real
  application replay exposed non-bit-preserving float readback with the default
  configuration; the fix was validated by JSON readback, not by relaxing the
  comparison. No dependency versions or lockfiles were upgraded.

## Explicit configuration

Set `NTE_EXACT_PACKET_CONFIG` before launching CLI/Tauri. Its JSON fields are
`rpc` (the existing Profile), `local_ip`, `local_port`, `server_ip`, `server_port`,
and `catalog_path` (the verified asset relationship catalog). The endpoint and
RPC indices must match the capture/build. Do not copy a previous session's
profile and claim it was automatically verified.

```text
cargo run --no-default-features --features cli --example replay_application -- PCAPNG OUTPUT_JSON
```

This exercises the application's actual `import_pcapng -> EngineEvent ->
apply_engine_event -> CombatState` path, not the standalone decoder example.
The five real captures produce 1192 retained hits with zero legacy Hit,
HitFollowUp or HitDamageCorrection events. Field comparison covers amount,
category, full references, main skill, current HP and maximum HP.

No game-runtime capture, installed-package deployment or visual UI smoke is
claimed. Dynamic-actor bootstrap, unsupported versions and unsupported payload
branches remain explicit availability boundaries, not guessed fallbacks.

## Migration verification and known baseline failures

Formal five-capture replay, JSON readback, exact-history roundtrip, quarantine,
late enrichment, multi-target identity, generation isolation, round cut/reset,
recorded-half updates and idle/no-op revisions have targeted coverage.
Frontend tests: 294 passed. Tauri tests: 270 passed. Architecture and runtime
safety gates passed. Rust and TypeScript checks/clippy/lint passed as recorded in
the task's tool outputs.

The complete Rust suite still has the pre-existing Japanese locale missing-key
failure documented above. The global contract parity script also fails on the
unrelated pre-existing `MOD_MARKET_SCHEMA_VERSION` manifest (4 versus source 5);
the same script fails in the original checkout. The changed CLI version and the
Rust/TypeScript detail versions are checked separately. Neither unrelated defect
was hidden by changing its expected result.

Final migration full-suite results: CLI 756 passed / 1 baseline failure / 8 ignored; desktop 838 passed / 1 baseline failure / 12 ignored; external resources 832 passed / 1 baseline failure / 12 ignored. No failures were weakened or hidden.
