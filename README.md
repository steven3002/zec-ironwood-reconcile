# zec-ironwood-reconcile

`zec-ironwood-reconcile` is a local tool for reproducing Orchard and Ironwood value-pool
changes over a selected Zcash block interval. It captures public block data from Zebra,
reconstructs transaction-level pool deltas from a declared anchor, compares the calculated
ending balances with Zebra's aggregate reporting, and produces a hashed evidence bundle
that can be verified offline.

The tool does not prove total Zcash supply, validate zero-knowledge proofs, determine
whether historical counterfeiting occurred, or replace full-node consensus validation.

## Status

**In development.** No release has been published and no evidence bundles exist yet. See
[Project status](#project-status) below for what currently works.

## The claim

Given a declared starting anchor and a continuous sequence of public Zcash blocks, the tool
independently reconstructs the Orchard and Ironwood value-pool changes over that interval
from transaction-level public data, and compares the calculated ending balances with
Zebra's reported balances.

That is the entire claim. In particular the tool does **not**:

- prove total Zcash supply from genesis;
- verify every Zcash consensus rule;
- prove the Ironwood circuit is sound;
- prove Orchard was never exploited;
- replace Zebra consensus validation;
- certify that Zcash's circulating supply is correct;
- provide a formal security audit.

## Why it is not a wrapper around a node

Transaction deserialization uses `zcash_primitives`, a different codebase from the Zebra
node whose figures are being checked. Everything downstream — the sign convention, the
per-transaction deltas, the per-block aggregation, the interval accumulation, the anchor
application, and every check — is implemented in this crate.

Zebra's reported balances are the hypothesis under test, never an input to the calculation.
Agreement means two independent implementations over the same public bytes reached the same
result; disagreement means one of them contains a defect, and locates where.

See [`ACCOUNTING_MODEL.md`](ACCOUNTING_MODEL.md) for the arithmetic and its specification
citations.

## Commands

```
zec-ironwood-reconcile capture     # collect a bounded interval from a Zebra RPC endpoint
zec-ironwood-reconcile reconcile   # reconstruct pool changes from an evidence bundle
zec-ironwood-reconcile verify      # verify an archive offline against a published hash
zec-ironwood-reconcile inspect     # display bundle metadata
```

Offline verification requires no node, no internet access, no database, no hosted service,
no wallet, and no infrastructure belonging to this project.

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

## Building

```sh
cargo build --release
cargo test
```

The pinned toolchain is declared in `rust-toolchain.toml`.

## Project status

| Component | State |
| --- | --- |
| Domain types, error model, exit codes, CLI surface | Implemented and tested |
| Evidence format, manifest, hashing, validation | Implemented and tested |
| Transaction parsing | Implemented; unverified against real Ironwood bundles |
| Reconciliation and checks | Implemented and tested |
| Reports and determinism | Not started |
| Archive, offline verify, inspect | Not started |
| RPC client and capture | Not started |
| Published mainnet evidence | Not started |

No component is described as delivered until its tests pass against real chain data. The
project publishes no evidence bundles yet.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
