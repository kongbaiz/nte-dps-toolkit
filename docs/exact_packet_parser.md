# Exact packet parser rewrite candidate

## Status

The worktree now routes both real live capture and PCAP import through the exact
runtime adapter and the shared reducer. It no longer falls back to legacy packet
damage when configuration is absent or decoding fails. Inventory, equipment,
packet diagnostics and existing combat-clock paths remain independent.

Normal CLI/Tauri startup now includes a bounded midstream attachment path for
the two reviewed PlayerController wire layouts. It requires no environment
variable or per-session endpoint file. This is a current-branch source candidate,
not a packaged or deployed build; it does not claim support for arbitrary future
versions or unreviewed channel/class layouts.

## Default desktop and CLI attachment

- `automatic.rs` uses the reviewed channel-3 layouts (field upper bound 213 with
  settlement index 139, or 219 with index 142; both use request index 100).
  These come from the existing five-capture profiles and matching class schema,
  not a scan for plausible damage bytes. The full-login sample independently
  contains the channel-3 PlayerController archetype export/open record.
- Local direction uses the selected NIC address, or the existing private/public
  distinction for offline replay; ambiguous direction remains unavailable.
  Endpoints and the component prefix come from this capture, never a prior run.
- Before exposing any damage, exactly one layout must join a request and a
  settlement by channel/message/raw timestamp bits and full source/target
  references, with a known player involved. A successful body parse alone,
  similar amount, or nearby time cannot qualify the lane. Buffered observations
  retain their original capture timestamps.
- Maximum eight connection/prefix lanes; each probe retains at most 128 messages
  and 4,096 rows. The losing layout is released after qualification. The current
  `skill_attribution_catalog.json` is embedded with all 952 GE entries (including
  unresolved entries) and 100 skill roots from the existing resource refresh; it contains
  no machine paths, endpoints or capture payloads. Catalog cloning occurs once
  per qualified flow, not per packet. CLI includes it through the core manifest.
  Its asset-build qualification metadata is preserved; resource availability is
  not a claim of runtime activation identity or universal version coverage.
- Handshakes discard unqualified probes. An ambiguous handshake after an
  accounted flow stops that tuple visibly while retaining its dedup identities;
  restarting capture is required for that case. A new endpoint/prefix gets a
  separate flow. Incomplete fragments are isolated and
  reported; later complete messages remain usable. Conflicting/unsupported
  confirmed traffic stops that flow visibly without falling back to estimates.
- The existing `ExactSettlement -> reducer -> LiveCaptureService -> AppState ->
  MainDpsSnapshot -> typed React client` path remains the sole accounting path.
  The waiting issue clears on qualification; incomplete/unavailable warnings
  are not hidden by a later generic status message.

The ignored Tauri acceptance test
`automatic_midstream_capture_reaches_main_dps_display_contract` accepts
`NTE_TEST_AUTO_CAPTURE` and `NTE_TEST_EXPECTED_DAMAGE`. It verifies the real
default import/service/revision/main-display DTO path, without an explicit
packet profile. `NTE_TEST_MAIN_SNAPSHOT_OUTPUT` optionally writes that DTO for
the frontend contract test (`NTE_TEST_MAIN_SNAPSHOT_INPUT`). Real capture files
are not committed. Browser/native screen inspection is a separate acceptance
step and is not implied by this DTO test.

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
link types, unqualified nonempty extra-damage arrays, malformed streams and
incomplete fragments return explicit errors in the standalone harness. Recovery
arrays are decoded as HP-only records and never converted to damage.
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
  no-ops. Source, raw settlement amount, category, target and first-settlement
  time are frozen; validated derived scaling can be enriched or retracted.
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

## Mechanism labels and validated HP scaling

Exact presentation also loads the existing GE-index mapping and semantic names;
a GE no longer needs a `DT_SkillDamageData` row or a fabricated GA owner just to
have a useful label. `display_only` semantics are allowed only for registered GE
names and cannot set an owner, GA, damage coefficient or accounting rule. BlackBird
GE 6094 uses its dedicated reaction label; Parry and Zankou's reaction preserve
their existing labels. A known GE without a verified human/skill label stays
identifiable as `GE <index>`, with unresolved skill attribution retained.

Lacrimosa's registered 200% maximum-HP rule is accounted separately from raw
server damage. A proof requires a matching request/effect/owner, the full target
identity, a frozen preceding server HP snapshot, one damage component, and exact
f32 agreement with `(previousHP - damage) / previousMaxHP * nextMaxHP`. Client
predicted damage and stale request HP are not used as the preceding HP. The
result is explicitly tagged `ruleVerifiedScaling`, not a raw server damage field.

Raw `damage` stays immutable. Additional observed HP loss occupies the existing
`follow_up_damage` contribution; upper-limit reduction occupies
`max_hp_reduction` and is exposed as known only with the proof. These are distinct
quantities. The existing opt-in upper-limit-in-total setting remains unchanged.
Repeated target entries/components, retransmissions and late requests cannot
apply the reduction twice. Late enrichment/retraction updates global/character
totals, the timeline and the original abyss half through bounded deltas, without
inventing another hit or activity timestamp. Conflicting predecessor evidence
invalidates dependent derived contributions. Capture gaps clear HP continuity;
old history without proofs remains readable and unknown rather than fabricated.

The 2026-09-25 01:04 sample retained 836 raw damage records. Its raw outgoing
amount remained 6,853,491; 23 validated scaling contributions added 233,351.125
actual HP loss, yielding 7,086,842.125, while separately recording 392,530 in
maximum-HP reduction. The two ordinary repeated-target entries in the same
Nightmare messages did not receive a second scaling contribution. This is
real-file replay evidence, not an assertion of a new live desktop run.

## Explicit configuration (diagnostics override)

CLI/Tauri first uses an explicit `NTE_EXACT_PACKET_CONFIG` override. Without
that override, it reads `exact-packet.json` beside its executable (not from the
current working directory). A relative `catalog_path` is resolved beside that
configuration file. A missing default file selects the automatic path described
above and reports that it is waiting for matching traffic. An invalid default file or
explicit override is an error, never a reason to try another profile or the
legacy parser. The configuration is read once per decoder/session; changing it
requires restarting capture. Its JSON fields are
`rpc` (the existing Profile), `local_ip`, `local_port`, `server_ip`, `server_port`,
and `catalog_path` (the verified asset relationship catalog). The endpoint and
RPC indices must match the capture/build. Do not copy a previous session's
profile and claim it was automatically verified.

The explicit override is for diagnostics and qualified non-default profiles;
ordinary users do not need to create it. An explicit invalid override never
silently falls back to automatic mode. Generic class/channel discovery outside
the reviewed layouts remains unsupported.

### Full-login transport regression

The 2026-09-24 full-login capture provided a captured channel-3 actor-open
record and its PlayerController archetype export, unlike the earlier midstream
sample. Its observed data prefix was 28, not the older profile's 20. These are
session-specific observations, not new global defaults.

The explicit-profile decoder now separates the non-data component branch from
sequenced traffic, consumes the verified dynamic-actor spawn header, and ignores
byte-identical reliable Bunch retransmissions before fragment reassembly. The
duplicate window is per direction, limited to 256 Bunches, and cleared with the
decoder. Conflicting same-sequence content and genuinely missing/discontinuous
fragments remain errors. It never deduplicates by damage amount or timestamp.

The full 12,037-packet file was replayed through the application import and
shared reducer with the explicitly supplied matching session configuration:
180 retained hits, outgoing damage 1,534,882, incoming damage 0. The import copy
only lowers the interface's declared snaplen to 65,535; all captured packets
were at most 1,064 bytes, and packet bytes/timestamps were verified unchanged.
The original capture remains unchanged. This is real-file replay evidence,
not installed-desktop live acceptance or automatic profile discovery.

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


## Mixed-source child attribution and mechanism labels

The settlement message source identifies the initiator, not every child attacker.
For an exact message/timestamp key, equal request/settlement target counts and a
complete ordinal match of full target identities bind each child to its own
request source and GE. The first request source must match the message source.
A mismatched/incomplete repeated-target layout stays unresolved; amounts and
client predictions are never matching keys. Unique reordered targets retain the
existing full-identity consensus path.

Persisted `source` stays the immutable message initiator; optional
`request_source` records a different, aligned per-target source. Old archives
without this additive field remain readable. Late known-player attribution can
move an existing outgoing hit between characters, preserving its amount, time,
ordinals and original Abyss half. Duplicate input is a no-op; conflicting requests
retract the enrichment. Rare source changes use the existing structural aggregate
rebuild, not a second appended hit; normal request-before-settlement capture does
not rebuild. No incoming/outgoing reclassification is introduced.

The paired capture confirms ten BlackBird coordinated attacks (61,273) previously
credited to Lacrimosa or Zankou. Display-only mechanism labels distinguish
BlackBird's coordinated attack and enhanced Nova settlement, Lacrimosa's enhanced
Discord bonus, and the boss projectile return. These labels do not invent GA
activation evidence. Boss/monster act IDs remain descriptive labels where formal
skill titles are unverified; projectile damage remains credited to the transmitted
player source, separately from its physical projectile origin. Max-HP scaling
still requires the registered rule and matching server HP witnesses.


## Shared break damage and explicit subtypes

Exact settlements use server display type 22 for shared break damage, regardless
of when request metadata arrives. Legacy hits retain the existing attack-type
rule. Both ordinary break and Daffodill's extra break remain in team totals and
shared-mechanic details, not personal damage rankings. Their transmitted source
is retained for traceability. Named subtypes require matching GE semantics;
other reaction categories cannot inherit those names from the trigger skill.

The paired 04:49 capture contains ordinary break (GE 749, 494,745), Daffodill
extra break (GE 2949, 153,848), and BlackBook arc follow-up (GE 3311, 6,773).
The last is ordinary type-0 damage triggered by break, not break damage itself.
Its label comes from the BlackBook fork item/upgrade tables and GE asset, not a
fabricated character GA. All 321 primary amounts/display types match the native
server-settlement records. Team total remains 3,365,909.03125; shared break is
648,593. The real replay/history/UI detail filter verifies both subtype names.


## Observed prefixes and recovery-only settlements

Automatic attachment must not assume the low two prefix bits are zero. The
05:33 Raw IP capture has prefix 0x19. It still must pass the same bounded
PlayerController grammar and exact bidirectional message/actor join; merely
accepting a prefix never enables damage. The primary connection now decodes all
44,724 packets without an error: 925 request messages, 921 settlement messages,
824 primary damage components and 57 HP-only recovery records.

The second tail array is `ClientRecoverDataArray` (`FClientRepRecoverData`):
full target reference followed by current-HP f32. The CN SDK container/type,
existing native capture array name, complete bit consumption, finite values and
consistent trailing message metadata establish the layout for this sample.
Recovery records are retained in the settlement dedup identity, create no hits,
and invalidate only the corresponding target's damage-only HP predecessor.

The first tail array, `ClientExtraDamageInfos`, is still unqualified on wire.
A nonempty instance yields a typed, visibly reported unsupported-RPC marker.
Only its already bounded RPC body is omitted, preserving neighboring RPCs,
reliable sequence state, qualification and dedup; HP continuity is invalidated.
Malformed framing, conflicting identities and resource-budget errors remain
fail-closed. This is not permission to scan past unknown bytes or manufacture
extra damage from HP changes. The 05:33 sample does not require this fallback.
