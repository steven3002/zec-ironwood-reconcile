# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

**No release has been tagged.** The crate version is `0.1.0` and every entry below sits under
*Unreleased*. Two schemas carry their own versions, independent of the crate's: the evidence
manifest schema and the report schema. Those are contracts with anyone holding a published
artifact, and their history is recorded separately at the end of this file.

---

## [Unreleased]

### Added

- **`capture`**, collects a bounded block interval from a Zebra RPC endpoint. Read-only:
  it calls `getinfo`, `getblockchaininfo` and `getblock` and nothing else. Rate limited,
  with bounded retry. Cookie authentication by default. Preflight guards confirm the node
  serves the requested network, that its declared NU6.3 activation height matches this
  build's constant, that the tip is far enough beyond the interval, and that the node reports
  usable pool values at the anchor; after the interval completes the end block is re-read and
  the capture refused if its hash changed. `--resume` continues an interrupted run through
  atomic writes, so a file that exists is complete. `--archive` packs the bundle and writes
  `<archive>.sha256` in the format `sha256sum -c` accepts.
- **`reconcile`**, reconstructs Orchard and Ironwood pool changes from an evidence bundle
  and compares them against the node's reported figures at every height.
- **`verify`**, verifies an archive offline and reproduces a report hash. Calls the same
  reconciliation function `reconcile` does, so the two cannot disagree by construction.
- **`inspect`**, displays bundle metadata.
- **Evidence bundle format**, specified in `EVIDENCE_FORMAT.md` precisely enough for an
  independent implementation. Blocks stored as raw consensus hex rather than a node's decoded
  JSON. Deterministic archive packing: sorted entries, zeroed mtime/uid/gid, fixed mode, so
  two operators packing the same bundle produce identical archives.
- **Hardened archive extraction**, refusing traversal and absolute paths, links, special
  entry types, and archives exceeding entry-count, per-entry-size, total-size and path-depth
  bounds. Path safety is validated before an entry's type is classified.
- **Canonical reports**, RFC 8785 JSON with a SHA-256 digest, pinned by a committed golden
  file, plus a Markdown rendering produced from the report structure that recomputes nothing.
- **Layering enforced by test.** `tests/layering.rs` fails the build if the offline
  verification path reaches the network layer, or if the HTTP client is named outside
  `rpc/client.rs`.
- **Syscall-traced offline verification.** `scripts/check-offline-verify.sh` traces the
  release binary across eight scenarios and fails on any network syscall, with `capture`
  traced as a positive control. Runs in CI.
- **Auditable release builds.** `scripts/check-auditable-build.sh` builds with
  `cargo-auditable` and reads the embedded dependency tree back out of the binary, checking an
  ordinary build has no tree first as a control. Runs in CI.
- **`tests/completeness.rs`**, guards every set this crate declares twice, once as the
  definition and once as an enumeration.
- Committed evidence bundles from testnet as fixtures: one spanning the NU6.3 activation
  boundary, one covering the first real Ironwood inflow. Both are re-checked on every build.
- **Mainnet evidence.** Three bundles captured from a live Zebra 6.2.3 mainnet node on
  2026-08-07, each reconciling with every accounting check passing and no diverging height:

  | Bundle id | Interval | Report hash |
  | --- | --- | --- |
  | `mainnet-3428141-3428146` | 3428142–3428146 | `0a2ca229afb716ca77e3857c5f0a0700a8d36ee2a99b9235fec58cdb1fdc78db` |
  | `mainnet-3428143-3428147` | 3428144–3428147 | `64d975d67b5c97251e13ee8e3c3dd9f21273d2d379157236094aac6a0dbdb157` |
  | `mainnet-3439599-3439699` | 3439600–3439699 | `ea003acbe31b1ed33d73e46575234034fdf4496eef895d4829096d4f2fa17527` |

  The first spans the NU6.3 activation height, 3,428,143, and is the only bundle on either
  network whose 23 checks all reach an affirmative verdict over pools that move. It is
  committed under `tests/fixtures/bundles/mainnet-activation-boundary/`; the other two are
  release artifacts, with the reasoning in `tests/fixtures/PROVENANCE.md`.
- **`tests/mainnet_evidence.rs`**, pinning the committed mainnet bundle's published report
  hash as a test literal alongside the testnet one. Every other determinism assertion compares
  the tool against itself on one machine, which cannot distinguish reproducible from
  consistently wrong. The testnet literal does not cover mainnet evidence: the report carries
  the network, the interval and every verdict, so a defect reachable only from mainnet data
  would leave it intact.

### Changed

- **Report schema 1.1.0 → 1.2.0.** Added `reconciled_by_version`. See the schema history
  below.
- **`turnstile_observed` renamed to `pool_flows_observed`**, with a ninth limitation added
  stating that the two figures are not a balance. The old name embedded a disproved model in
  a hashed artifact and invited a reader to treat the figures as one.
- **The Orchard withdrawal-only rule is checked per transaction, not per block.** ZIP 258
  constrains every transaction. A block with a +100 and a −200 Orchard transaction nets to
  −100 and passed a per-block test while the first transaction broke consensus. The offending
  txid and index are now named.
- **The anchor and reported end balances are read from the evidence, not from the manifest.**
  A manifest is written by whoever produced the bundle; taking the anchor from it meant
  deriving a result from a figure its own author supplied, and a fabricated manifest would
  have produced a fully passing report.
- **Pool-state records are bound to the block they describe**, using the block hash this
  crate computes from the block's own header. A bundle with shuffled pool files was previously
  accepted with exit 0, because digests cannot help when whoever produced a bundle can reseal
  it.
- **Branch identifiers come from the upgrade table in `zcash_protocol`** rather than a
  hand-modelled NU6.3 boundary, which had reported NU6.2 for every earlier height and made any
  pre-activation block unparseable.
- **Pool responses are projected onto an allow-list of fields** rather than stored verbatim.
  `getblock` verbosity 1 carries `confirmations`, the distance to the tip, so a verbatim
  response made the same block produce different bytes on every capture and broke
  reproduction.
- **`--archive` refuses to overwrite an existing file** unless `--overwrite` is given, and
  refuses before the node is read. It had silently overwritten, which could replace a
  published archive and its digest without a word.
- **The authentication hint is conditional.** Every transport failure had been told to check
  `--rpc-cookie-file`, including a refused connection and a name-resolution failure. The hint
  now appears only for a hang-up on an already-accepted connection, or an HTTP 401/403.
- **`ironwood_anchor_zero` reads the boundary balance from wherever the bundle establishes
  it**, the anchor or a height inside the interval. Tying it to the anchor made it mutually
  exclusive with `no_ironwood_before_activation`, so no single bundle could affirm both halves
  of the boundary claim.
- **`orchard_withdrawal_only` is not applicable below activation** rather than passing. A
  passing verdict asserted that a post-activation consensus rule held across heights where it
  did not exist.
- **The `interval_precedes_activation` advisory is computed from the interval's end height**,
  not from the anchor. It had told an interval running 4,133,999–4,134,010 that no Ironwood
  value could exist in it, false, and emitted on precisely the most important interval the
  tool supports.
- **Nothing infers from the node's `monitored` flag.** Zebra computes it as
  `chainValueZat != 0` in one shared constructor, so it restates the balance. Reading it as a
  tracking signal made two checks downgrade every correct comparison against a pool that
  legitimately holds nothing, that is, every capture of the activation boundary. The flag is
  still stored, because it is part of the response.

### Fixed

- **A missing height exited 10**, a filesystem error naming an absolute path, instead of
  reporting unusable evidence. It now exits 4, naming the relative path.
- **Directory entries in archives were refused**, so the tool could only read archives
  produced by its own packer. GNU `tar` emits them. They are now skipped, while a directory
  entry carrying a traversing path is still refused.
- **A corrupt or non-archive input was reported as a filesystem error**, telling an operator
  to check their disk when they should have been re-downloading.
- **Unlisted check identifiers kept their evaluation order** when sorted, because the sort was
  stable and unlisted identifiers all mapped to the same rank. A canonical report hash would
  have depended on the order checks happened to be evaluated in.
- **The offline-verification layering scan did not cover `commands/reconcile.rs`**, which sits
  on the verification path because `verify` reaches the accounting path through it. The scan
  hand-listed `commands/verify.rs` only. It now enumerates `commands/` and exempts
  `capture.rs` by name, asserting the exemption still refers to a file that exists.

### Documentation

- Corrected the stated reason for encoding money as strings. The original, that a zatoshi
  amount exceeds 2^53, is **false**: `MAX_MONEY` is 2,100,000,000,000,000 against RFC 8785's
  exact-integer limit of 9,007,199,254,740,991. The decision is unchanged and now rests on
  reasons that hold: accumulation runs in `i128` where intermediates are bounded by neither,
  every figure in a bundle is untrusted input, and a numeric encoding would round such a value
  silently.
- Corrected the claim that the two comparison axes give two independent confirmations per
  height. Zebra derives `valueDeltaZat(h)` by subtracting consecutive `chainValueZat` values,
  so given a correct anchor the balance axis is entailed by the delta axis. Both are still
  computed, one localises a divergence, the other catches a misaligned anchor, but a reader
  should count one independent comparison per height. No check changed: both axes still
  compare this crate's reconstruction against the node.
- Narrowed the independence claim. It is not that this tool and Zebra share no code: Zebra
  depends on `zcash_primitives` at the same version this tool links, and most of the two
  locked dependency graphs coincide. The claim that survives is that Zebra reads transactions
  and computes value balances with its own decoder in `zebra-chain`, reaching
  `zcash_primitives` only for transaction identifiers and signature hashes.
- Recorded that `--tip-distance 100` is a conservative choice rather than a derived one. No
  specification states a depth beyond which a Zcash reorganisation cannot occur, and no source
  for one was found.
- Added `ACCOUNTING_MODEL.md`, `ARCHITECTURE.md`, `LIMITATIONS.md`, `SECURITY.md`,
  `CONTRIBUTING.md`, `REPRODUCING.md` and this file.
- Recorded that Ironwood is funded by **two different mechanisms**, one observed on each
  network. Testnet 4,134,683 funded it by issuance in a lone coinbase transaction with Orchard
  unmoved; mainnet 3,428,144 funded it by a movement out of Orchard, with the 20,000 zatoshi
  difference appearing in the transparent pool. §6.1 of `ACCOUNTING_MODEL.md` previously
  generalised from the testnet case that Ironwood inflow "routinely exceeds" Orchard outflow;
  mainnet shows the opposite ordering, and at 3,428,146 shows an excess inflow sourced from
  the transparent pool rather than from issuance. Neither ordering identifies a source, and
  nothing is inferred from the relationship between the two figures. No check changed: the
  inequality was never asserted, which is why the mainnet data contradicted no verdict.
- `REPRODUCING.md` now walks through the mainnet boundary bundle, and states in a table what
  has been run on which platform. The mainnet hashes have been reproduced only on two hosts
  the publisher operates, running the same OS and architecture with the same toolchain and
  producing byte-identical binaries; that is recorded as a change of host and not as a
  cross-platform result. The cross-platform CI job is marked as configured but never executed.

---

## Schema history

These version independently of the crate. A reader must reject a major version it does not
implement rather than attempt a partial interpretation.

### Report schema

| Version | Change |
| --- | --- |
| `1.2.0` | Added `reconciled_by_version`, the build that reconciled the bundle and decided every verdict. The report previously carried only `tool_version`, taken from the manifest, the *capturing* build, a field the bundle's author writes. Check semantics decide the report hash, so two builds reconcile one bundle to two hashes while both reports name the same version, and a verifier comparing hashes could not tell a difference in evidence from a difference in builds. Compiled in via `CARGO_PKG_VERSION` so it cannot be supplied by input. Golden file regenerated |
| `1.1.0` | Renamed `turnstile_observed` to `pool_flows_observed` and added a ninth limitation stating that Orchard outflow and Ironwood inflow are separate observations rather than a balance, because Ironwood receives newly issued value directly from coinbase transactions |
| `1.0.0` | Initial canonical report schema |

### Manifest schema

| Version | Change |
| --- | --- |
| `1.1.0` | Added `end.tracking`, recording the node's `monitored` value for each reconstructed pool at the end height. **The field names are misnomers**, held for schema compatibility: on Zebra this is `chainValueZat != 0`, not a statement about which pools the node tracks. Nothing in the reconciliation reads it |
| `1.0.0` | Initial manifest schema |

---

## Not yet done

Recorded here rather than omitted, so that a reader of this file is not left to infer status
from what the entries above happen to mention.

- No **release has been tagged**, so the three mainnet archives are not yet attached to
  anything a third party can download.
- No **independent party** has reproduced a result.
- No **mainnet report hash** has been reproduced outside the two hosts the publisher operates,
  which differ in neither operating system nor architecture.
- **macOS and `aarch64`** have never run this suite. The CI job covering `windows-latest` and
  `macos-latest` is configured and has never executed.
