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

Transaction deserialization uses `zcash_primitives`. Everything downstream — the sign
convention, the per-transaction deltas, the per-block aggregation, the interval
accumulation, the anchor application, and every check — is implemented in this crate. No
Zebra crate appears in this binary's dependency graph.

The claim worth making is narrower than "different codebase", and is checkable. Zebra does
depend on `zcash_primitives` — at the same version this tool links — but it does not use it
to read transactions or to compute pool balances. Zebra deserializes transactions with its
own implementation in `zebra-chain`, and derives each pool's value balance from the fields
that implementation produced. `zcash_primitives` is reached from Zebra only for transaction
identifiers and signature hashes, neither of which contributes to a verdict here. So the two
`valueBalance` figures being compared are decoded from the same consensus bytes by two
independently written decoders.

That independence is not total, and overstating it would be the wrong argument. The two
share a large upstream graph, including the cryptographic crates `halo2_proofs`,
`pasta_curves` and `reddsa`. Those crates do not decode a value balance — both sides read it
as a little-endian `i64` in their own code — but a defect inside them would be invisible to
both.

Zebra's reported balances are the hypothesis under test, never an input to the calculation.
Agreement means two independent decoders over the same public bytes reached the same result;
disagreement means one of them contains a defect, and locates where.

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
  --from-height 3428143 --to-height 3429143 \
  --output ./evidence/mainnet-3428142-3429143 \
  --archive ./evidence/mainnet-3428142-3429143.tar.zst
```

Cookie authentication is the default path; Zebra writes the cookie into its cache directory
at startup, and `--rpc-cookie-file` is only needed when the node uses a custom location.
`--archive` also writes `<archive>.sha256` in the format `sha256sum -c` accepts.

Capture only reads. It calls `getinfo`, `getblockchaininfo`, and `getblock`, and nothing
else — no wallet method, no key request, no broadcast, no node configuration change.

Before retrieving any block it confirms the node serves the requested network, that the
node's own NU6.3 activation height matches the one compiled into this build, that the tip is
far enough beyond the requested interval to survive a reorganisation, and that the node
reports usable pool values at the anchor. After the interval completes it re-reads the end
block and refuses the capture if the hash changed. `--resume` continues an interrupted run;
files are written by atomic rename, so a file that exists is complete.

The endpoint is transported over plain HTTP only, because Zebra's RPC port does not offer
TLS. Credentials never reach a bundle, a log line, or an error message.

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
| Transaction parsing | Implemented; **verified against a real Ironwood bundle on testnet** |
| Reconciliation and checks | Implemented and tested |
| Reports and determinism | Implemented and tested |
| Archive packaging and hardened extraction | Implemented and tested |
| `inspect` | Implemented and tested |
| `verify` | Implemented; reproduces a report hash offline |
| RPC client and `capture` | Implemented; exercised against a live Zebra 6.2.3 node |
| `reconcile` | Implemented; exercised against real Ironwood-era testnet blocks |
| Published mainnet evidence | Not started |

No component is described as delivered until its tests pass against real chain data.

The end-to-end path is demonstrated on testnet. At height 4,134,683 — where value first
entered the Ironwood pool, 683 blocks after NU6.3 activated — the tool reconstructs an
inflow of **125,000,000 zatoshi** from the block's own version 6 transaction bytes, which is
the figure Zebra independently reports for that height. Every height in the interval agrees
on both comparison axes, and the archive verifies offline to the same report hash. The
bundle is committed as a fixture with its provenance in
[`tests/fixtures/PROVENANCE.md`](tests/fixtures/PROVENANCE.md), so that result is re-checked
on every build.

**Still open.** Nothing has been demonstrated on mainnet, no evidence bundle has been
published, and no independent party has reproduced a result.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
