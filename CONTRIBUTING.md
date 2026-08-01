# Contributing

Thank you for looking at this. The most valuable contribution to a tool like this one is not
a feature, it is a demonstration that a result is wrong, or a reproduction confirming that
one is right. See [`REPRODUCING.md`](REPRODUCING.md) for the latter.

---

## Prerequisites

The toolchain is pinned in `rust-toolchain.toml` (currently **1.96.0**), so `rustup` selects
it automatically. Edition 2024.

A release build needs a **C toolchain**: `zcash_script` pulls `secp256k1-sys`, which is a C
library.

`Cargo.lock` is committed and must stay committed. Reproducible builds are the point of the
project, and an unpinned graph would make a published report hash a function of when the
binary was built.

---

## The commands that must pass

All four must be clean before a change is proposed. CI runs them and treats each as required.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo deny check advisories licenses bans sources
```

`cargo deny` needs a separate install:

```sh
cargo install cargo-deny --locked
```

### Release builds

Release binaries embed their dependency tree, so a shipped artifact can be scanned without
this repository:

```sh
cargo install cargo-auditable rust-audit-info --locked
cargo auditable build --release
./scripts/check-auditable-build.sh
```

The script builds twice on purpose. An ordinary release build is checked to have **no**
embedded tree before the auditable build is checked to have one, a reader that succeeded
unconditionally would otherwise be indistinguishable from success.

### The offline-verification trace

```sh
cargo build --release
cargo test --test capture      # produces target/offline-check/evidence.tar.zst
./scripts/check-offline-verify.sh
```

This traces the release binary's syscalls and fails if `verify` or `inspect` opens a socket.
It needs `strace` and no privileges.

---

## Lints are load-bearing, not stylistic

The crate denies, for production code: `unsafe_code`, `arithmetic_side_effects`, the
truncating and sign-changing cast lints, `indexing_slicing`, `unwrap_used`, `expect_used`,
and `float_arithmetic`.

**Do not add an exception to make a change compile.** Each of these encodes a property the
project claims: that there is no unsafe code, that arithmetic cannot wrap silently, that no
monetary value can be built from a float. A carve-out weakens the claim everywhere, not just
at the site of the exception.

There is precedent. A helper computing blocks per second was rejected by
`float_arithmetic`, and it was **removed rather than exempted**: "this crate contains no
floating-point arithmetic anywhere" is a stronger and more defensible claim than one with a
carve-out, and a consumer can derive a rate at the point of display, where it cannot reach a
hashed artifact.

Test code may relax these with a file-level `#![allow(...)]`, and does.

---

## Architectural rules are enforced by test

`tests/layering.rs` reads the source and fails the build if a layer imports a layer it may
not reach, or if anything on the offline verification path names the network layer or the
HTTP client. The rule table in that file mirrors [`ARCHITECTURE.md`](ARCHITECTURE.md).

**A change to that table is an architectural decision, not a test fix.** If a change requires
editing it, that is the thing to discuss in the pull request.

`tests/completeness.rs` guards the sets this crate declares twice, once as the definition
and once as an enumeration. Adding a check identifier without listing it in the presentation
order, or an error variant without adding it to the variant helper, fails the build with the
offender named.

---

## Fixture provenance

Every test vector records where its bytes came from, in
[`tests/fixtures/PROVENANCE.md`](tests/fixtures/PROVENANCE.md). Fixtures are classified as
**captured** or **synthetic**.

**A fixture without recorded provenance is not evidence and may not be cited in support of
any claim about behaviour against real chain data.**

The distinction is not bookkeeping. A synthetic fixture is built from the same understanding
of the format as the code that reads it, so it cannot demonstrate that extraction from a real
Ironwood bundle is correct, only a captured one breaks that circularity. A synthetic fixture
can demonstrate that malformed input is rejected, and that is what they are used for.

When adding a captured fixture, record the network, height, block hash, node implementation
and version, the RPC call, the capture time, the file digest, and the **cross-check**: what
the node independently said the block's pool deltas were. The cross-check is what makes a
captured fixture useful, because it is what the test asserts the reconstruction agrees with.

---

## Changing a schema

Two schemas are versioned, and both are contracts with people who hold published artifacts.

| Schema | Constant | Current |
| --- | --- | --- |
| Manifest | `evidence::manifest::SCHEMA_VERSION` | `1.1.0` |
| Report | `report::schema::REPORT_SCHEMA_VERSION` | `1.2.0` |

**A change to the canonical serialization requires a version increment and a regenerated
golden file.** Both, not either.

- Increment the version. A change that alters the meaning or serialization of existing fields
  is a **major** increment; an additive change an older reader can safely ignore is a
  **minor** one.
- Regenerate `tests/fixtures/golden-report.json` and its `.sha256`.

A silent change means a previously published report hash is no longer reproducible by this
build, which breaks the one property the project exists to provide. The golden file is a
regression guard, not evidence: it proves the serialization has not changed, and says nothing
about whether the figures in it came from real chain data. They did not.

Check identifiers and error identifiers are stable strings embedded in reports. Changing one
is a breaking change to the report schema. Improving the *wording* of a message is not,
because reports carry the identifier and never the rendered message, that separation is
deliberate and worth preserving.

---

## Tests

New behaviour needs a test that fails without it. Where a test guards against a specific
defect, **mutation-check it**: break the thing it guards and confirm the test fails, naming
the offender. Several tests in this crate carry a comment recording that this was done, and
one of them was found to be reading zero variants from real source while passing its own
synthetic case.

Prefer a test that builds the real artifact, a real malicious archive, a real HTTP server, a
real captured block, over one that asserts against a substituted implementation.

A test whose positive control is missing is the failure mode to watch for. A run in which the
measurement silently recorded nothing must not be indistinguishable from success.

---

## Commit and pull request conventions

Commit messages describe what changed and why. There is no enforced format.

In a pull request, state what evidence stands behind the change: the tests that fail without
it, whether they were mutation-checked, and whether anything was verified against a live node.

**Do not describe an unfinished feature as delivered**, in a commit message, a document, or a
status table. A gate is open until its evidence exists, and passing unit tests are not that
evidence.

---

## Licensing

Contributions are dual-licensed under MIT and Apache-2.0, matching the project. See
[`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).

`deny.toml` allows permissive licenses only. An unexpected license appearing in the graph is
a design signal rather than a compliance failure: it usually means a dependency brought in
something the project has no use for.
