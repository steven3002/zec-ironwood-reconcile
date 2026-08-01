# Reproducing a result

This is the instruction set for someone who has been handed a published evidence archive and
a published report hash, and who wants to establish for themselves whether the claim holds.

You need **no node, no internet access, no database, no hosted service, no wallet, and no
infrastructure belonging to this project**. You need the archive, its digest, the expected
report hash, and a machine that can build Rust.

**Nothing has been published yet.** No evidence bundle exists in public, no release has been
tagged, and no independent party has reproduced a result. This document specifies the
procedure so that it is fixed in advance of any publication, rather than written to fit
whatever a first reproduction happened to do.

---

## Before you start, which shell

Every command in this document is **POSIX shell**. It uses `sha256sum`, `uname` and
`tar --zstd`, none of which exist in Windows PowerShell, where the first line fails with
`A parameter cannot be found that matches parameter name 'eu'`.

| Platform | Use |
| --- | --- |
| Linux, macOS | Any terminal |
| Windows | **Git Bash**, installed with Git for Windows, so if `git` works you already have it. WSL also works |

### Save the commands to a file; do not paste them into the prompt

Any block beginning `set -eu` is a **script**, and pasting it into an interactive shell
closes the window. `set -e` exits the shell on the first command that returns non-zero, and a
shell prompt runs such commands constantly; `set -u` exits on the first unset variable. The
terminal shuts before anything useful happens.

Write it to a file and run the file:

```sh
cat > repro.sh <<'EOF'
# ... paste the block here ...
EOF

bash repro.sh
```

The heredoc is safe to paste, because everything between `<<'EOF'` and `EOF` is written to
the file rather than executed. The script then runs in its own shell, so a failure inside it
ends the script and leaves your terminal open with the output still on screen.

A PowerShell-native path exists and is described at the end, under *Reproducing without a
POSIX shell*. It skips the archive and reconciles the bundle directory directly, which needs
no `tar` and no `sha256sum`.

---

## What you should have been given

A publication is incomplete unless it includes all five:

| Item | Example |
| --- | --- |
| The archive | `testnet-4134682-4134686.tar.zst` |
| Its SHA-256 digest, in `sha256sum -c` format | `testnet-4134682-4134686.tar.zst.sha256` |
| The expected canonical report hash | `4a5d4d76…` |
| The exact verification command | see step 4 |
| The tool version and git commit that produced it | `0.1.0`, commit `abc1234` |

If the expected report hash is missing, you can still verify the archive's internal
consistency, but you cannot confirm you arrived at the same answer as the publisher. Ask for
it.

---

## Step 1, Check the archive is the one that was published

```sh
sha256sum -c testnet-4134682-4134686.tar.zst.sha256
```

```
testnet-4134682-4134686.tar.zst: OK
```

If this fails, stop. Everything below is meaningless on a different file. Obtain the archive
again, ideally from a different source than the first, and compare.

Note what this does and does not establish: it shows you hold the same bytes the digest
describes. It does not show the publisher was honest, that is what the rest of the procedure
is for.

## Step 2, Build the tool from source

Do not use a binary supplied by the same party that published the archive. The point of the
exercise is that two independent parties reach the same figure; a binary from the publisher
collapses the two.

```sh
git clone <repository-url> zec-ironwood-reconcile
cd zec-ironwood-reconcile
git checkout <the published commit>
cargo build --release
```

The toolchain is pinned in `rust-toolchain.toml` and `rustup` will select it. A release build
needs a C toolchain, because one dependency pulls a C library.

Record the version you built:

```sh
./target/release/zec-ironwood-reconcile --version
```

```
zec-ironwood-reconcile 0.1.0
```

If this does not match the published tool version, say so in your report. It does not
invalidate the reproduction, but check semantics decide the report hash, so a different
build may legitimately produce a different hash, and that is a finding rather than a failure.

## Step 3, Confirm the tests pass on your machine

```sh
cargo test
```

This is not ceremony. The suite includes committed evidence bundles captured from real
testnet chain data, so a passing run establishes that *your* build reconstructs the Ironwood
inflow the network reported, before you apply it to anything the publisher gave you.

## Step 4, Verify the archive offline

```sh
./target/release/zec-ironwood-reconcile verify \
  testnet-4134682-4134686.tar.zst \
  --expected-report-hash 4a5d4d7603618a80a8de29c84fe8e6fb601365f06502be374d1e43338902039e
```

A successful reproduction looks like this. The figures below are real output, produced by
packing the testnet Ironwood bundle committed under `tests/fixtures/bundles/`, not a
published artifact, since none exists yet:

```
Bundle:            testnet-4134682-4134686
Entries extracted: 18
Bytes extracted:   42981
Files verified:    16
Report hash:       4a5d4d7603618a80a8de29c84fe8e6fb601365f06502be374d1e43338902039e
Checks:            Pass
Result:            MATCH
```

and exits **0**.

`verify` extracts the archive into a temporary directory it creates, validates the manifest
structurally before opening any file, hashes every listed file, reconstructs the pool changes
from the blocks' own bytes, compares against the figures the capturing node reported, and
recomputes the canonical report hash. It calls the same reconciliation function `reconcile`
does, so a hash produced by one and reproduced by the other cannot differ by construction.

### If the hash does not match

```
Report hash:       4a5d4d76…
Checks:            Pass
Result:            MISMATCH
Expected:          0000000000000000000000000000000000000000000000000000000000000000
```

and it exits **1**. This is a real finding and is worth reporting either way. Distinguish
three cases before concluding anything:

- **Checks pass, hash differs.** The evidence reconciles, but this build produced a different
  canonical report. Most often a version difference. Compare `reconciled_by_version` in your
  report against the publisher's.
- **Checks fail.** The evidence itself did not reconcile. The report names the failing check
  and, for an accounting divergence, the height at which it first appears.
- **Verification refused the archive.** See the exit codes below.

### Other exit codes you may see

| Code | Meaning |
| --- | --- |
| 0 | Reconciliation completed and all required checks passed |
| 1 | Reconciliation completed but one or more checks failed, or the expected hash did not match |
| 3 | The manifest is invalid |
| 4 | An evidence file is missing, its digest mismatched, or the archive was refused |
| 5 | A transaction or block could not be decoded |
| 6 | The block sequence is incomplete or does not link |
| 8 | Network or activation-context mismatch |

An expectation that cannot be met is a failure, not an absence of one: supplying
`--expected-report-hash` for an archive whose hash cannot be computed exits non-zero rather
than staying silent.

## Step 5, Look at what you verified

`verify` works on a temporary extraction that it removes afterwards. To examine the contents,
unpack the archive yourself and reconcile the directory:

```sh
mkdir bundle && tar --zstd -xf testnet-4134682-4134686.tar.zst -C bundle
./target/release/zec-ironwood-reconcile inspect bundle
./target/release/zec-ironwood-reconcile reconcile bundle --output ./out
```

`reconcile` writes `report.json`, `report.md` and `report.sha256` into `./out`. Read
`report.md`: it carries the interval, the reconstruction, the per-height comparison, every
check with its verdict, and the nine limitations. Its hash must equal the one `verify`
reported.

`inspect` takes a directory, not an archive, and reports only what the manifest *claims*, it
does not hash file contents. It is a triage tool; `verify` is the one that establishes
anything.

## Step 6, Repeat on a different machine, if you can

Determinism across machines has been demonstrated once: the same report hash was reproduced
byte for byte on Windows/WSL2 and on Ubuntu, from the same commit. Both were `x86_64`.

What that leaves untested is a **different CPU architecture**. If you have access to an
`aarch64` machine, an Apple Silicon Mac, an ARM server, a Raspberry Pi, running steps 2 and
4 there and reporting the hash would close the last variable anyone has raised. Both
architectures are little-endian, so a difference would be surprising; surprising is exactly
what is worth checking.

A second run on any machine other than your own is still worth reporting even if it is
`x86_64`.

---

## What to report back

For a reproduction to count, it must be checkable by a third person who has neither your
machine nor the publisher's. State all of the following. A report missing the version fields
cannot be interpreted, because a hash difference and a build difference are then
indistinguishable.

1. **The archive's SHA-256**, and whether `sha256sum -c` passed.
2. **The expected report hash** you were given.
3. **The report hash you obtained**, and whether it matched.
4. **The exit code** of the `verify` command.
5. **The tool version and git commit you built**, and the output of
   `zec-ironwood-reconcile --version`.
6. **`reconciled_by_version` from your `report.json`**, the build that decided the verdicts,
   which is not necessarily the one named in the bundle's manifest.
7. **The exact command line you ran**, verbatim.
8. **Your platform**: operating system, version, and CPU architecture.
9. **The `rustc` version** you built with (`rustc --version`).
10. **The overall status and any non-passing checks**, with their identifiers.
11. **Whether `cargo test` passed** on your machine, and the count.
12. **Anything you had to change** to make it work. This is the most valuable field on the
    list, and an empty one is itself a result.

A convenient way to gather most of it:

```sh
{
  echo "platform:  $(uname -srm)"
  echo "rustc:     $(rustc --version)"
  echo "tool:      $(./target/release/zec-ironwood-reconcile --version)"
  echo "commit:    $(git rev-parse HEAD)"
  sha256sum testnet-4134682-4134686.tar.zst
} 
```

Post the result as a public issue on this repository, titled `Reproduction: <bundle id>`.
Public is deliberate: a reproduction nobody else can read establishes nothing.

### What counts, and what does not

A reproduction **counts** when an independent party, on hardware the publisher does not
control, built the tool from source and obtained the published report hash, or did not, and
said so with enough detail for the difference to be located.

A reproduction **does not count** if it used a binary the publisher supplied, if it omits the
version fields, or if it reports only "it worked". None of those can be checked by anyone
else, which is the whole standard.

**A reproduction that fails is worth more than one that succeeds**, and will be treated that
way. The project's stated purpose is to make a claim checkable; a checkable claim that turns
out to be wrong has still done its job.

---

## What a successful reproduction establishes

That two independently written decoders, run by two unrelated parties over the same public
consensus bytes, computed the same value-pool changes and the same canonical report.

It does **not** establish that total Zcash supply is correct, that no historical
counterfeiting occurred, that the Ironwood circuit is sound, or that the chain is valid. The
full boundary is in [`LIMITATIONS.md`](LIMITATIONS.md), and every report carries a summary of
it in its own text.

---

## Reproducing without a POSIX shell

Windows PowerShell has no `sha256sum`, no `uname`, and a `tar` that may lack zstd support.
The archive step is therefore skipped: the evidence bundle is committed to the repository, so
`reconcile` reads it directly and produces the same report hash `verify` would.

This establishes the same thing the archive path does, that your machine reconstructs the
published figure, but it does not exercise archive extraction. If you can use Git Bash or
WSL, prefer the main procedure above, which does both.

```powershell
$Expected = "4a5d4d7603618a80a8de29c84fe8e6fb601365f06502be374d1e43338902039e"
$Commit   = "ac045a2a8f127e26236f3f4b8b60b1eb582d6682"

if (Test-Path repro-zec) { Remove-Item -Recurse -Force repro-zec }
git clone --quiet https://github.com/steven3002/zec-ironwood-reconcile.git repro-zec
Set-Location repro-zec
git checkout --quiet $Commit
cargo build --release

$Bin = ".\target\release\zec-ironwood-reconcile.exe"
& $Bin reconcile tests\fixtures\bundles\testnet-ironwood --output .\out
$Observed = (Get-Content .\out\report.sha256).Split(" ")[0]

Write-Output ""
Write-Output "=============== COPY EVERYTHING BELOW THIS LINE ==============="
Write-Output ("date           : " + (Get-Date).ToUniversalTime().ToString("yyyy-MM-dd HH:mm") + " UTC")
Write-Output ("os             : " + (Get-CimInstance Win32_OperatingSystem).Caption)
Write-Output ("arch           : " + $env:PROCESSOR_ARCHITECTURE)
Write-Output ("rustc          : " + (rustc --version))
Write-Output ("commit built   : " + (git rev-parse HEAD))
Write-Output ("tool version   : " + (& $Bin --version))
Write-Output ""
Write-Output ("expected hash  : " + $Expected)
Write-Output ("observed hash  : " + $Observed)
Write-Output ("match          : " + $(if ($Observed -eq $Expected) { "YES" } else { "NO" }))
Write-Output "=============== COPY EVERYTHING ABOVE THIS LINE ==============="
```

**This block has not been executed on Windows by the project.** The POSIX procedure has been
cold-run from a clean clone; this one is written from the same steps but not verified on the
platform it targets. If it fails, that is worth reporting, send the error text. A broken
instruction is a more useful result than a matching hash.
