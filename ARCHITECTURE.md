# Architecture

How `zec-ironwood-reconcile` is arranged, and why. This is a public summary of the design;
the arithmetic it implements is specified in [`ACCOUNTING_MODEL.md`](ACCOUNTING_MODEL.md).

---

## 1. Governing principle

**A pure core with I/O only at the edges.**

Every accounting decision lives in code that touches no network, no clock, and no filesystem.
`reconcile` and `verify` execute the *same* pipeline over the same bytes; only the source of
the bytes differs.

```
        edges (I/O)                      pure core (no I/O, no clock, no paths)
   ┌───────────────────┐        ┌──────────────────────────────────────────────┐
   │ rpc/ → capture/   │─bytes─▶│  parse/ → reconcile/ → checks/ → report/     │─▶ canonical
   │ evidence/ (fs)    │        │              deterministic                   │   JSON + hash
   └───────────────────┘        └──────────────────────────────────────────────┘
```

Three properties follow from the shape rather than from discipline:

- offline verification is **structural**, not a feature somebody has to remember to preserve;
- report-hash determinism is enforced by what the core is able to reach;
- the core is unit-testable without a node.

`verify` calls the same reconciliation function `reconcile` does, so a hash produced by one
and reproduced by the other cannot differ by construction.

---

## 2. Layers

| Layer | Responsibility |
| --- | --- |
| `domain/` | The vocabulary of the problem: money, pools, heights, networks, node-reported pool state |
| `parse/` | Raw consensus bytes to signed per-transaction pool deltas |
| `reconcile/` | Deltas to an interval result: aggregation, accumulation, continuity, binding |
| `checks/` | Verdicts, each with a stable identifier |
| `report/` | Result to artifacts: the canonical schema and its Markdown rendering |
| `evidence/` | The on-disk bundle: manifest, layout, hashing, validation, archive |
| `rpc/` | Transport. Knows nothing about accounting |
| `capture/` | Orchestrates `rpc` and `evidence`. The only networked command |
| `commands/` | Thin orchestration per CLI verb |
| `cli/` | Argument definitions and the exit-code table |
| `canonical.rs` | RFC 8785 serialization and SHA-256. The sole producer of hashed bytes |
| `error.rs` | The typed error model and its stable identifiers |

`domain/` is deliberately not a utility drawer. It contains the vocabulary of the problem and
nothing else.

`canonical.rs` is a top-level module rather than living under `report/`, because manifest
digests and report digests must use identical canonicalization. Two implementations would be
free to diverge, and a divergence would stay invisible until an external reproduction failed.

---

## 3. Dependency rules, and how they are enforced

| Layer | May **never** import |
| --- | --- |
| `domain` | every other layer |
| `parse` | `rpc`, `capture`, `evidence`, `report`, `commands` |
| `reconcile` | `rpc`, `capture`, `evidence`, `commands` |
| `checks` | `rpc`, `capture`, `commands` |
| `report` | `rpc`, `capture`, `commands` |
| `rpc` | `parse`, `reconcile`, `checks`, `report`, `commands` |
| `evidence` | `rpc`, `parse`, `reconcile`, `commands` |
| `capture` | `reconcile`, `checks`, `report` |
| `commands` |, orchestrates every layer by design |

The critical rule is that **`reconcile/` and everything downstream of it cannot reach
`rpc/`**. That is the mechanical guarantee behind offline verification.

These are not enforced by review. `tests/layering.rs` reads the source and fails the build on
a violation, with six tests:

- no layer imports a layer it is forbidden to reach;
- the offline verification path cannot reach the network layer;
- only the transport module names the HTTP client;
- test modules are last in every file;
- no top-level module outside the layers reaches the network;
- the rule table covers every layer that exists.

The scan is deliberately blunt and textual. A textual scan cannot be defeated by an unusual
import form, and the crate is small enough that the cost is a rare explicit exception rather
than constant friction. Test modules are excluded, a test may reach for whatever it needs to
build a fixture, which is why one of the tests asserts that no shipped code appears *after*
a test module, where the scan could not see it.

### Two rules about the rules

The last two tests exist because of a defect class this project has hit twice: **a set
declared twice, once as the definition and once as an enumeration of it.** The compiler
checks a `match` for exhaustiveness; it does not check an array or a hand-written list.

- `canonical.rs` and `error.rs` sit in no layer directory, so no rule in the table covered
  them. They are named explicitly, and a new top-level module must now be classified rather
  than slip through.
- The offline-verification scan originally hand-listed `commands/verify.rs`. But `verify`
  reaches the accounting path by calling into `commands/reconcile.rs`, which was on the
  verification path and in no scanned set. The scan now enumerates everything under
  `commands/` and exempts `capture.rs` **by name**, asserting that the exemption still refers
  to a file that exists, so a command added later is covered by default.

`tests/completeness.rs` applies the same treatment to the crate's other twice-declared sets:
the check identifier list, the error-variant test helper, the pool enumeration, the generated
path list, and the published limitations. Each scanner is itself pinned by a test on input
whose answer is known, because a source scan that silently matched nothing would make every
test built on it vacuous.

---

## 4. Why the accounting path cannot reach the network

Stated as three independent supports, because the property is the tool's central promise.

**Structural.** The dependency rules above forbid it, and `tests/layering.rs` enforces them
mechanically. The HTTP client may be named in exactly one file, `rpc/client.rs`.

**Static.** The release dependency graph contains exactly two network-related crates: the
HTTP client and its sans-I/O protocol parser. Only one of them can open a socket, and the
verification path is forbidden from naming it. No input can reach code that is not linked
into the path.

**Empirical.** `scripts/check-offline-verify.sh` traces the release binary's syscalls across
eight scenarios, valid, tampered, truncated, non-archive, missing-file, wrong-expected-hash,
and `inspect`, and fails if any of them issues a network call. `capture` is traced alongside
as a positive control, so that a run in which tracing recorded nothing cannot pass as success.

Both `tests/layering.rs` and the trace script run in CI. The layering test was
mutation-checked: injecting a network import into a file on the verification path fails it and
names the file.

An honest note on how this claim aged. Before the RPC client existed, "verification performs
no network access" was guaranteed for free by the dependency graph containing no HTTP client
at all. Adding `capture` ended that, and the guarantee was rebuilt deliberately as the rules
and checks described here rather than left to rest on a property that had quietly stopped
holding.

---

## 5. Determinism

A report is a pure function of the evidence bundle and the reconciling build's version.
Nothing that varies between machines or runs may enter it: no wall-clock timestamps, no
filesystem paths, no hostnames, no durations, no locale-dependent formatting, no unordered
collections.

Three mechanisms hold that:

- the report builder reads no clock, no environment and no filesystem;
- checks are sorted into a canonical order by position in the identifier list, so report
  output never depends on the order checks happened to be evaluated in. An identifier absent
  from that list sorts last with a tiebreak on the identifier itself, and is separately
  surfaced rather than merely survived;
- measurements that legitimately vary, timings, throughput, are carried in a separate
  structure that never enters the hashed artifact, with a test asserting its field names
  never appear in canonical report bytes.

The Markdown rendering is produced **from** the report structure and recomputes nothing, so
it cannot become a second accounting path that disagrees with the first.

A committed golden file pins the canonical serialization byte for byte. If the serialization
changes deliberately, the golden file is regenerated **and** the report schema version is
incremented, a silent change would mean a previously published hash is no longer
reproducible.

Two version fields appear in every report, and the distinction matters. `tool_version` is
what the bundle's manifest claims captured it: a claim by the bundle's author. `reconciled_by_version`
is the build that decided every verdict and produced the hash, compiled in so it cannot be
supplied by input. Without the second, a verifier comparing hashes could not tell a difference
in evidence from a difference in builds.

---

## 6. Dependencies and why each is there

| Crate | Purpose | Justification |
| --- | --- | --- |
| `clap` | CLI | Derive API; standard |
| `serde`, `serde_json` | Serialization | Required |
| `serde_jcs` | RFC 8785 canonical JSON | The determinism contract depends on byte-stable serialization across machines and library versions |
| `thiserror` | Typed errors | Stable error identities the report schema depends on |
| `sha2`, `hex` | Hashing and encoding | Evidence integrity |
| `tar`, `zstd` | Archive | Bundle packaging |
| `tempfile` | Temporary directories | Hardened extraction into a directory this process controls |
| `time` | RFC 3339 timestamps | Manifest metadata only; never the hashed report |
| `zcash_primitives` | Transaction and block deserialization | Exposes `ironwood_bundle()` publicly, so no hand-rolled parser is needed |
| `zcash_protocol` | `BranchId`, the upgrade table | Branch-identifier validation at every height |
| `orchard` | `Bundle::value_balance()` | Value-balance access |
| `ureq` | Blocking HTTP | The transport for `capture` |
| `base64` | HTTP Basic credentials | Already present transitively; declared directly so the authentication encoding is not another crate's implementation detail |

**Deliberately excluded.**

- `tokio` and `reqwest`. The workload is a sequential, rate-limited job with no concurrency to
  exploit, and `reqwest` spawns a `tokio` thread even in blocking mode. `deny.toml` bans
  `tokio` outright, so its reintroduction fails CI rather than passing review.
- **TLS.** `ureq` is built with `default-features = false`. Zebra's RPC port serves plain
  HTTP, so a TLS stack would be roughly thirty crates of audit surface that could never be
  used. `https://` is refused with a reason rather than silently downgraded. The whole
  transport adds 8 crates to the graph, and no async runtime.
- `anyhow`. It erases the typed error identities the report schema and the exit-code table
  depend on.
- `flate2`. `zstd` alone suffices.

**Not excluded, and worth naming.** `orchard` pulls the halo2 proving crates unconditionally,
so declining to verify proofs does not keep them out of the tree. They are permissively
licensed and immaterial to the accounting, but they are there. A native C toolchain is
required for a release build, because `zcash_script` pulls a C library through `secp256k1-sys`.

Supply chain: `cargo deny` enforces advisories, licenses, bans and sources in CI. Release
binaries are built with `cargo-auditable`, which embeds the dependency tree in the binary so a
shipped artifact can be scanned without this repository; CI builds one and reads the tree back
out, with an ordinary build checked first as a control.

---

## 7. Errors and exit codes

`error.rs` defines the error model with 22 variants, each carrying a **stable identifier**.
Reports embed the identifier, never the rendered message, so wording can be improved without
altering a published report or its hash.

`cli/exit.rs` is the sole authority mapping outcomes to the 11 documented exit codes. Both the
identifier mapping and the exit-code mapping are exhaustive matches, so the compiler catches a
new variant before any test does.

Exit code 0 is returned only when every required check passed. A completed run whose
accounting comparison failed exits 1, a failed comparison can never be mistaken for success
by a calling script.

The full table is in [`README.md`](README.md).

---

## 8. What this architecture deliberately does not do

No plugin system, no trait abstraction over "node implementations", no configuration
framework, no async runtime, no database, no caching layer, no generic pipeline abstraction.

There is one transport trait with one production implementation, and it exists so that guards
and failure paths are testable without a node, not as an abstraction over node
implementations, which the design rejects.

The tool captures a bounded interval, sums integers, compares, and writes two files.
Structure exists to keep the accounting path pure and auditable, and nowhere else.
