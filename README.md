# zec-ironwood-reconcile

A local tool for independently reproducing Orchard and Ironwood value-pool changes over a
selected Zcash block interval.

It captures public block data from a Zebra node, reconstructs transaction-level pool deltas
from a declared anchor, compares the calculated ending balances against Zebra's aggregate
reporting, and produces a hashed evidence bundle that anyone can verify offline.

Reconciliation is deterministic: the same evidence yields the same report hash on any
machine, so a result can be published as a single 64-character figure and checked by a
third party without trusting the publisher.

## Installation

```sh
cargo install --path .
```

Or build in place:

```sh
cargo build --release
```

The toolchain is pinned in `rust-toolchain.toml`; no other configuration is required.

## Quick start

Verify a published archive against its report hash, with no node and no network access:

```sh
zec-ironwood-reconcile verify mainnet-3428141-3428146.tar.zst \
  --expected-report-hash e10929941f3cdf05217f939b0e2f8c93efccfc3bb6b81805c75f26f29eb0b66e
```

Or reproduce a committed bundle from a clone alone:

```sh
cargo build --release
./target/release/zec-ironwood-reconcile \
  reconcile tests/fixtures/bundles/mainnet-activation-boundary --output ./out
cat ./out/report.sha256
```

[`REPRODUCING.md`](REPRODUCING.md) is the full procedure, including a PowerShell path for
Windows, and states what a reproduction must report for it to count.

## The claim

Given a declared starting anchor and a continuous sequence of public Zcash blocks, the tool
independently reconstructs the Orchard and Ironwood value-pool changes over that interval
from transaction-level public data, and compares the calculated ending balances against
Zebra's reported balances.

That is the entire claim. The tool does **not**:

- prove total Zcash supply from genesis;
- verify every Zcash consensus rule;
- prove the Ironwood circuit is sound;
- prove Orchard was never exploited;
- replace Zebra consensus validation;
- certify that Zcash's circulating supply is correct;
- provide a formal security audit.

[`LIMITATIONS.md`](LIMITATIONS.md) states the boundary of what a report may be cited for.

## Commands

```
zec-ironwood-reconcile capture     # collect a bounded interval from a Zebra RPC endpoint
zec-ironwood-reconcile reconcile   # reconstruct pool changes from an evidence bundle
zec-ironwood-reconcile verify      # verify an archive offline against a published hash
zec-ironwood-reconcile inspect     # display bundle metadata
```

### Offline verification

Verification requires no node, no internet access, no database, no hosted service, no
wallet, and no infrastructure belonging to this project.

That is enforced rather than asserted. The module dependency rules forbid the verification
path from reaching the network layer, and `tests/layering.rs` fails the build if it does.
`scripts/check-offline-verify.sh` then traces the release binary's syscalls and fails if
`verify` or `inspect` issues any network call, with `capture` traced alongside as a positive
control. Both run in CI.

### Capturing

```sh
zec-ironwood-reconcile capture \
  --rpc-url http://127.0.0.1:8232 \
  --rpc-cookie-file ~/.cache/zebra/.cookie \
  --network mainnet \
  --from-height 3428144 --to-height 3428147 \
  --output ./evidence/mainnet-3428143-3428147 \
  --archive ./evidence/mainnet-3428143-3428147.tar.zst
```

Cookie authentication is the default path; Zebra writes the cookie into its cache directory
at startup, and `--rpc-cookie-file` is only needed when the node uses a custom location.
`--archive` also writes `<archive>.sha256` in the format `sha256sum -c` accepts.

Capture only reads. It calls `getinfo`, `getblockchaininfo` and `getblock`, and nothing
else: no wallet method, no key request, no broadcast, no node configuration change.

Before retrieving any block it confirms the node serves the requested network, that the
node's own NU6.3 activation height matches the one compiled into this build, that the tip is
far enough beyond the requested interval to survive a reorganisation, and that the node
reports usable pool values at the anchor. After the interval completes it re-reads the end
block and refuses the capture if the hash changed. `--resume` continues an interrupted run;
files are written by atomic rename, so a file that exists is complete.

The endpoint is transported over plain HTTP only, because Zebra's RPC port does not offer
TLS. Credentials never reach a bundle, a log line, or an error message.

## Mainnet evidence

Three bundles captured from a live Zebra 6.2.3 mainnet node on 2026-08-07. Each contains the
raw consensus bytes of every block in its interval, the node's own reported pool figures at
each height, and a manifest hashing all of it.

| Bundle id | Interval | Report hash | Archive SHA-256 |
| --- | --- | --- | --- |
| `mainnet-3428141-3428146` | 3428142–3428146 | `e10929941f3cdf05217f939b0e2f8c93efccfc3bb6b81805c75f26f29eb0b66e` | `da2db22303bab2dd33ee0262524c46c7bb834a499ac0dc69c738d2073d1ef177` |
| `mainnet-3428143-3428147` | 3428144–3428147 | `67774f62162ee588282c1e99efc66653d0e50923b008b0d28514b1de2b43575e` | `fa94524b64a36891dd6eb9aabfcd3379e87ae17d16faa0a9227a2516a9f91dcc` |
| `mainnet-3439599-3439699` | 3439600–3439699 | `84c797f3cf03ffc039ae5c5751586a1d16d0048d679f9a9fdb99ac62c02a5d0a` | `f63bcdbdfaf34eeca6e2a3382fd25c8e8ab9e817af761278b2dbbe92291ee886` |

All three reconcile with every accounting check passing and no diverging height. The archives
and their digests are published as release artifacts. `mainnet-3428141-3428146` is also
committed under [`tests/fixtures/bundles/`](tests/fixtures/bundles/), with its report hash
pinned as a test literal, so the result is re-checked on every build rather than resting on a
manual run.

### What the mainnet data shows

`mainnet-3428141-3428146` spans the NU6.3 activation height, 3,428,143, and reaches an
affirmative verdict on **all 23 checks**: none reports `NotApplicable`, none reports a
warning. The testnet boundary bundle does too, but with both pools motionless at every
height, it affirms the boundary rules without demonstrating value movement. The mainnet
bundle does both, with real value moving in Orchard or Ironwood at four of its five heights.

Value first entered the Ironwood pool at 3,428,144, the first block after activation, and it
arrived by a different route than on testnet:

| | testnet 4,134,683 | mainnet 3,428,144 |
| --- | --- | --- |
| Blocks after activation | 683 | 1 |
| Transactions in the block | 1, a coinbase | 3 |
| Ironwood delta | +125,000,000 | +1,000,000 |
| Orchard delta | 0 | −1,020,000 |
| Where the value came from | issuance | the Orchard pool |

On mainnet the Orchard outflow exceeds the Ironwood inflow by 20,000 zatoshi, and the
transparent pool's delta rises by exactly that amount: +137,520,000 at 3,428,144 against
+137,500,000 the block before. That is what a transaction fee looks like in a pool view; the
matching figure is the measurement, and reading a fee into it is an interpretation rather
than something the node states.

Testnet had only ever shown Ironwood funded by issuance. Both mechanisms now have captured
evidence behind them, and neither is treated as a rule: the report states the Orchard outflow
and the Ironwood inflow as two separate observations rather than as a balance, for the
reasons given in [`ACCOUNTING_MODEL.md`](ACCOUNTING_MODEL.md).

`mainnet-3439599-3439699` covers 100 blocks roughly 11,500 past activation and is the only
evidence covering Ironwood *outflows*: 25 of its heights carry a negative Ironwood delta, and
its anchor holds a non-zero Ironwood balance, neither of which any boundary interval
contains.

## Independence from the node

Transaction deserialization uses `zcash_primitives`. Everything downstream, the sign
convention, the per-transaction deltas, the per-block aggregation, the interval accumulation,
the anchor application, and every check, is implemented in this crate. No Zebra crate
appears in this binary's dependency graph.

The claim worth making is narrower than "different codebase", and is checkable. Zebra does
depend on `zcash_primitives`, at the same version this tool links, but it does not use it to
read transactions or to compute pool balances. Zebra deserializes transactions with its own
implementation in `zebra-chain`, and derives each pool's value balance from the fields that
implementation produced. `zcash_primitives` is reached from Zebra only for transaction
identifiers and signature hashes, neither of which contributes to a verdict here. So the two
`valueBalance` figures being compared are decoded from the same consensus bytes by two
independently written decoders.

That independence is not total, and overstating it would be the wrong argument. The two share
a large upstream graph, including the cryptographic crates `halo2_proofs`, `pasta_curves` and
`reddsa`. Those crates do not decode a value balance, both sides read it as a little-endian
`i64` in their own code, but a defect inside them would be invisible to both.

Zebra's reported balances are the hypothesis under test, never an input to the calculation.
Agreement means two independent decoders over the same public bytes reached the same result;
disagreement means one of them contains a defect, and locates where.

## Verification coverage

Reconciliation is deterministic by construction, and the property is tested rather than
assumed: each committed bundle's report hash is pinned as a literal, so a platform that
computes a different figure fails the build instead of agreeing with itself.

| | Verified |
| --- | --- |
| Transaction parsing against real Ironwood bundles | testnet and mainnet |
| `capture` against a live Zebra 6.2.3 node | testnet and mainnet |
| Cross-platform determinism, testnet report hash | Ubuntu, WSL2, native Windows, all `x86_64` |
| Cross-platform determinism, mainnet report hashes | two Ubuntu `x86_64` hosts |
| Offline verification opens no socket | syscall-traced in CI |

The mainnet hashes have so far been reproduced only on hosts operated by the publisher, which
differ in neither operating system nor architecture. macOS and `aarch64` are covered by a CI
job that has not yet executed, so they are untested in fact. No independent party has yet
reproduced a published result.

[`REPRODUCING.md`](REPRODUCING.md) records exactly what has been run where, and no component
is described here as delivered until its tests pass against real chain data.

## Exit codes

| Code | Meaning |
| ---- | ------- |
| 0 | Reconciliation completed and all required checks passed |
| 1 | Reconciliation completed but one or more checks failed |
| 2 | Invalid command-line input |
| 3 | Evidence manifest invalid |
| 4 | Evidence file missing or hash mismatch |
| 5 | Unsupported transaction encoding |
| 6 | Block continuity failure |
| 7 | RPC capture incomplete |
| 8 | Network or activation-context mismatch |
| 9 | Internal processing error |
| 10 | Output path or filesystem error |

Exit code `0` is never returned when a required accounting comparison fails.

## Documentation

| Document | Contents |
| --- | --- |
| [`ACCOUNTING_MODEL.md`](ACCOUNTING_MODEL.md) | The arithmetic contract: units, the sign convention with its ZIP citations, extraction, aggregation, the comparison against the node, and the activation-specific rules |
| [`EVIDENCE_FORMAT.md`](EVIDENCE_FORMAT.md) | The bundle layout, manifest schema, hashing and validation, specified for an independent implementation |
| [`LIMITATIONS.md`](LIMITATIONS.md) | The boundary of what the output may be cited for |
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | The layering, the dependency rules and how they are enforced by test, and the dependency choices |
| [`SECURITY.md`](SECURITY.md) | Reporting channel, the archive threat model, credential handling, and what the tool does not defend against |
| [`REPRODUCING.md`](REPRODUCING.md) | How to verify a published bundle, and what a reproduction must report to count |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | Build, test, lint and supply-chain commands, and the rules a change must respect |
| [`CHANGELOG.md`](CHANGELOG.md) | Change history, including the manifest and report schema versions |

## Building from source

```sh
cargo build --release
cargo test
```

The crate forbids `unsafe` code and denies clippy's arithmetic-side-effects, float-arithmetic,
indexing, `unwrap` and `expect` lints in production code. `cargo deny` covers advisories,
licences, bans and sources. All of it runs in CI.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
