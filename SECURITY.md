# Security

## Reporting a vulnerability

Report suspected vulnerabilities by opening a **private security advisory** through GitHub's
"Report a vulnerability" form on this repository's Security tab. That channel stays private
until a fix is published.

If that form is not available to you, open a public issue containing **no details**, just a
request for a private channel, and one will be arranged before anything further is said.

Please do not put vulnerability details in a public issue. Public issues are the right place
for everything else, including incorrect results, which are the more likely failure mode of a
tool like this one.

A useful report includes the command, the tool version (`zec-ironwood-reconcile --version`),
the report schema version if a report was produced, and, where it can be shared, the
evidence bundle or archive that triggers the behaviour. An archive is the most useful thing
to attach, because verification is fully offline and reproducible from it alone.

Expect an acknowledgement within seven days. This is a small project with no funded security
response; that is the honest commitment rather than a service-level target.

---

## What the threat model is

The tool has exactly two inputs that are not under the operator's control, and they carry
very different levels of trust.

| Input | Trust | Why |
| --- | --- | --- |
| An **evidence archive or bundle** | **Untrusted** | The whole point of the format is that a stranger can publish one and anyone can verify it. It may be authored by an adversary |
| A **node's RPC responses** | Semi-trusted | The operator chose the endpoint, but the node is also the implementation being cross-checked, so its figures are the hypothesis under test rather than an input to the calculation |

Everything below follows from the first row. An evidence archive is the crate's largest
untrusted-input surface, and verification necessarily runs *before* its contents are known to
be honest.

---

## The archive threat model

### What the hardened extractor refuses

Extraction refuses, rather than accommodates:

- **paths that are absolute or contain `..`**, the vulnerability class commonly called Zip
  Slip, in which an entry named `../../etc/thing` writes outside the destination directory.
  A path is also refused if it is empty, contains a backslash, or contains a null byte;
- **symbolic and hard links**, which could redirect a later write outside the destination
  even when every path in the archive looks benign;
- **device, FIFO, socket and other special entry types**, which have no meaning in an
  evidence bundle;
- **archives exceeding the configured bounds** on entry count, per-entry size, total
  decompressed size, and path depth, which is what bounds a decompression bomb;
- **entries whose actual byte count differs from the size declared in their header**, so a
  declared size can never be used to reserve resources that the entry does not go on to
  consume.

Two ordering properties matter as much as the list:

**Path safety is validated before the entry type is classified.** A hostile path carried by
an entry type that would otherwise be skipped is still refused. Directory entries *are*
skipped, GNU `tar` emits them, and an extractor that rejected every non-regular entry could
only read archives produced by its own packer, but a directory entry carrying a traversing
path is refused like any other, and there is a test for exactly that case.

**Every refusal is checked before the entry's contents are written.** No partially written
hostile entry is ever left on disk.

Extraction happens into a directory the process created, obtained from `tempfile`, rather
than into a caller-supplied location.

`tests/archive_security.rs` holds 22 tests, each building a real malicious archive rather
than asserting against a mocked reader.

### What a manifest cannot do

The manifest is untrusted input in the same way. Structural validation runs **first and
completely**, before any listed file is opened: schema version, bundle identity, interval
consistency, digest format, path safety, sortedness and duplicates. A crafted manifest
therefore cannot induce a reader to open a path outside the bundle.

Directory traversal during validation never follows symbolic links, so a link cannot cause a
reader to leave the bundle directory or to loop.

### What digests do and do not prove

A bundle's digests prove that its files have not changed since the manifest was written. They
prove **nothing about whether the bundle is honest**, because whoever produced it can reseal
it. Two consequences are built into the reconciliation path:

- the arithmetic reads the anchor and the reported ending balances from the **evidence
  files**, never from the manifest, so a fabricated manifest cannot produce a passing report
  by agreeing with itself;
- each pool-state record is **bound to the block it describes**, using the block hash this
  crate computes from the block's own header. A bundle with shuffled pool files is refused
  even though every digest is correct.

---

## Credential handling

Credentials are relevant to `capture` only. `reconcile`, `verify` and `inspect` never
authenticate to anything.

**A captured bundle is meant to be published.** The RPC password, the endpoint that may embed
it, and the cookie contents must therefore appear in no artifact, no error message, and no
log line. Three mechanisms enforce that:

- the secret type has **no `Display` implementation**, and its `Debug` prints a placeholder,
  so a credential cannot be formatted into a string by accident;
- a scrubbing pass is applied to **every message originating outside this crate**, because a
  transport error can quote a URL that carries credentials;
- the manifest records only `rpc_url_redacted: true`, the endpoint itself is never written.

The one place a secret is rendered is the function that builds the HTTP authorization header,
whose result goes straight into a request and is never retained.

Cookie authentication is the default path, matching how Zebra is normally deployed. The
cookie file's presence on disk is **not** treated as evidence that a node is running: Zebra
does not delete it on clean shutdown, contrary to some documentation.

### Transport

`https://` is **refused with a reason** rather than silently downgraded. Zebra's RPC port
serves plain HTTP and offers no TLS, so the HTTP client is built with default features
disabled and no TLS stack is linked at all. Linking a TLS implementation that could never be
used would add roughly thirty crates to the audit surface for nothing.

The consequence, stated plainly: **the RPC connection is unauthenticated and unencrypted in
transit.** The supported deployment is a node reachable without an intermediary, normally
loopback. Anyone tunnelling the RPC port across an untrusted network must provide their own
transport security, such as an SSH tunnel or a VPN.

---

## Verification opens no socket

Offline verification requires no node, no internet access, no database, no hosted service and
no infrastructure belonging to this project. That is enforced rather than asserted, by two
independent mechanisms that run in CI:

- **Static.** The module dependency rules forbid the verification path from reaching the
  network layer, and `tests/layering.rs` reads the source and fails the build if it does. The
  HTTP client may be named in exactly one file. The scan covers every file under `commands/`
  except `capture.rs`, with an assertion that the exemption still refers to a file that
  exists, so a command added later is covered by default rather than escaping the rule.
- **Empirical.** `scripts/check-offline-verify.sh` traces the release binary's syscalls and
  fails if `verify` or `inspect` issues any network call. Eight scenarios are traced,
  including tampered, truncated, non-archive and missing-file inputs, because a single happy
  path proves little about code a different input would reach. `capture` is traced alongside
  as a **positive control**: without it, a run in which tracing silently recorded nothing
  would be indistinguishable from success.

Syscall tracing is used in preference to a network namespace because it answers a stronger
question. A namespace shows the process *could not* have reached anything; a trace shows it
*never asked to*.

---

## Memory safety and arithmetic

`unsafe_code` is set to `forbid` crate-wide, so this crate contains no unsafe blocks. That
says nothing about its dependencies, which are not audited for unsafe code.

Arithmetic is checked. `clippy::arithmetic_side_effects`, `indexing_slicing`, the cast lints,
`unwrap_used` and `expect_used` are all set to `deny` for production code, and the release
profile sets `overflow-checks = true`. `clippy::float_arithmetic` is denied with no
exceptions anywhere in the crate.

---

## What this tool deliberately does not defend against

Listed because a security document that implies more coverage than exists is worse than none.

- **A wrong anchor.** A bundle in which the anchor balance and every per-height reported
  balance are shifted by the same constant reconciles clean. The tool reconstructs changes
  from a declared anchor and does not derive supply from genesis. Detecting this requires
  checking the anchor against a source outside the bundle.
- **A dishonest node.** The node's figures are the hypothesis under test, not a trusted
  input, but a node that reported *internally consistent* wrong figures would produce
  agreement with a reconstruction from blocks it also served. Independent reproduction against
  a different node is what addresses this, and it is exactly what a published archive plus a
  published hash is for.
- **Cryptographic defects in shared dependencies.** Independence from the implementation
  being cross-checked is real but partial: this crate and Zebra share most of their locked
  dependency graph, including `zcash_primitives` itself and the cryptographic crates
  `halo2_proofs`, `pasta_curves` and `reddsa`. A defect inside those would be invisible to
  both. See [`LIMITATIONS.md`](LIMITATIONS.md) §2.
- **Invalid proofs.** `valueBalance` is read as a cleartext field. No zero-knowledge proof is
  verified.
- **The supply chain of the toolchain and dependencies.** `cargo deny` checks advisories,
  licenses, bans and sources in CI, and release binaries embed their dependency tree so a
  shipped artifact can be scanned directly. Neither is a substitute for auditing the
  dependencies themselves, which has not been done.
- **A hostile local environment.** A compromised machine can alter the binary, the bundle, or
  the reported result. Reproducibility on a *second* machine is the mitigation, and it has
  not yet been demonstrated.
- **Denial of service against a node.** `capture` is rate limited and read-only, and it calls
  only `getinfo`, `getblockchaininfo` and `getblock`. It is not designed to protect a node
  from an operator who raises the rate.

---

## Status of this document

No third party has audited this code, and the tool has not been reviewed for security by
anyone outside the project. Nothing here has been demonstrated on mainnet, no release has
been tagged, and no independent party has reproduced a result.
