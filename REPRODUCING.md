# Reproducing a result

This is the instruction set for someone who has been handed a published evidence archive and
a published report hash, and who wants to establish for themselves whether the claim holds.

You need **no node, no internet access, no database, no hosted service, no wallet, and no
infrastructure belonging to this project**. You need the archive, its digest, the expected
report hash, and a machine that can build Rust.

**No independent party has reproduced a result.** The
procedure below was fixed in advance of any publication, rather than written to fit whatever
a first reproduction happened to do, and the mainnet evidence it now walks through was
captured afterwards.

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
| The archive | `mainnet-3428141-3428146.tar.zst` |
| Its SHA-256 digest, in `sha256sum -c` format | `mainnet-3428141-3428146.tar.zst.sha256` |
| The expected canonical report hash | `e1092994…` |
| The exact verification command | see step 4 |
| The tool version and git commit that produced it | `1.0.0`, tag `v1.0.0` |

If the expected report hash is missing, you can still verify the archive's internal
consistency, but you cannot confirm you arrived at the same answer as the publisher. Ask for
it.

---

## The published bundles

Five bundles have been captured, three on mainnet and two on testnet. All were captured from
a live Zebra 6.2.3 node, all reconcile with every accounting check passing, and none has a
diverging height.

| Bundle id | Network | Interval | Report hash | Archive SHA-256 |
| --- | --- | --- | --- | --- |
| `mainnet-3428141-3428146` | mainnet | 3428142–3428146 | `e10929941f3cdf05217f939b0e2f8c93efccfc3bb6b81805c75f26f29eb0b66e` | `da2db22303bab2dd33ee0262524c46c7bb834a499ac0dc69c738d2073d1ef177` |
| `mainnet-3428143-3428147` | mainnet | 3428144–3428147 | `67774f62162ee588282c1e99efc66653d0e50923b008b0d28514b1de2b43575e` | `fa94524b64a36891dd6eb9aabfcd3379e87ae17d16faa0a9227a2516a9f91dcc` |
| `mainnet-3439599-3439699` | mainnet | 3439600–3439699 | `84c797f3cf03ffc039ae5c5751586a1d16d0048d679f9a9fdb99ac62c02a5d0a` | `f63bcdbdfaf34eeca6e2a3382fd25c8e8ab9e817af761278b2dbbe92291ee886` |
| `testnet-4134682-4134686` | testnet | 4134683–4134686 | `9a103df10d29704708864da39d320bd0ad1e0bafd02f8a2f785760fbca0785aa` | packed on demand from the committed fixture |
| `testnet-4133998-4134010` | testnet | 4133999–4134010 | `b60fdf9586ae565764059fea09f4ff07d7a5ca0268e9c04172cd2dfe7d3cba83` | packed on demand from the committed fixture |

**Start with `mainnet-3428141-3428146`.** It spans the NU6.3 activation height on mainnet and
is the one bundle whose 23 checks all reach an affirmative verdict over pools that actually
move. Everything below uses it; substitute another id and its hash to verify a different one.

Three of the five are committed to the repository in full, under `tests/fixtures/bundles/`:
`mainnet-3428141-3428146`, `testnet-4134682-4134686` and `testnet-4133998-4134010`. You can
reproduce their report hashes from a clone with no archive at all, which is the path
*Reproducing without a POSIX shell* at the end of this document takes. The three mainnet
archives are release artifacts; the two testnet bundles have no published archive, because a
`.tar.zst` of a directory already in the repository adds nothing a reproducer needs.

---

## Step 1, Check the archive is the one that was published

```sh
sha256sum -c mainnet-3428141-3428146.tar.zst.sha256
```

```
mainnet-3428141-3428146.tar.zst: OK
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
git checkout v1.0.0
cargo build --release
```

The toolchain is pinned in `rust-toolchain.toml` and `rustup` will select it. A release build
needs a C toolchain, because one dependency pulls a C library.

Record the version you built:

```sh
./target/release/zec-ironwood-reconcile --version
```

```
zec-ironwood-reconcile 1.0.0
```

If this does not match the published tool version, say so in your report. It does not
invalidate the reproduction, but check semantics decide the report hash, so a different
build may legitimately produce a different hash, and that is a finding rather than a failure.

## Step 3, Confirm the tests pass on your machine

```sh
cargo test
```

This is not ceremony. The suite includes committed evidence bundles captured from real
testnet **and mainnet** chain data, and pins both of their published report hashes as test
literals, so a passing run establishes that *your* build reconstructs the pool changes the
network reported, before you apply it to anything the publisher gave you.

## Step 4, Verify the archive offline

```sh
./target/release/zec-ironwood-reconcile verify \
  mainnet-3428141-3428146.tar.zst \
  --expected-report-hash e10929941f3cdf05217f939b0e2f8c93efccfc3bb6b81805c75f26f29eb0b66e
```

A successful reproduction looks like this, and the figures are real output:

```
Bundle:            mainnet-3428141-3428146
Entries extracted: 20
Bytes extracted:   572424
Files verified:    18
Report hash:       e10929941f3cdf05217f939b0e2f8c93efccfc3bb6b81805c75f26f29eb0b66e
Checks:            Pass
Result:            MATCH
```

and exits **0**. The other two mainnet archives verify the same way:

```sh
./target/release/zec-ironwood-reconcile verify \
  mainnet-3428143-3428147.tar.zst \
  --expected-report-hash 67774f62162ee588282c1e99efc66653d0e50923b008b0d28514b1de2b43575e

./target/release/zec-ironwood-reconcile verify \
  mainnet-3439599-3439699.tar.zst \
  --expected-report-hash 84c797f3cf03ffc039ae5c5751586a1d16d0048d679f9a9fdb99ac62c02a5d0a
```

`verify` extracts the archive into a temporary directory it creates, validates the manifest
structurally before opening any file, hashes every listed file, reconstructs the pool changes
from the blocks' own bytes, compares against the figures the capturing node reported, and
recomputes the canonical report hash. It calls the same reconciliation function `reconcile`
does, so a hash produced by one and reproduced by the other cannot differ by construction.

### If the hash does not match

```
Report hash:       e1092994…
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
mkdir bundle && tar --zstd -xf mainnet-3428141-3428146.tar.zst -C bundle
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

### What has actually been run where

**These reproductions were performed against version 0.1.0, and the figures they produced are
not the figures this release produces.** The package version is part of the hashed report, so
the 1.0.0 release reconciles the same evidence to different hashes. The evidence bytes did not
change; nothing was re-captured. What changed is the version string inside every report.

| Report hash observed | Under | Reproduced on | Not yet run on |
| --- | --- | --- | --- |
| `4a5d4d76…`, testnet | 0.1.0 | Ubuntu, Windows/WSL2, native Windows, three machines, all `x86_64` | macOS, `aarch64` |
| `0a2ca229…`, `64d975d6…`, `ea003acb…`, mainnet | 0.1.0 | Two Ubuntu `x86_64` hosts, both operated by the publisher | any machine the publisher does not operate; any other OS or architecture |
| `9a103df1…` testnet, `e1092994…` `67774f62…` `84c797f3…` mainnet | **1.0.0, this release** | one Ubuntu `x86_64` host | **everything else, including every machine above** |

The last row is the one that matters for anyone reproducing today, and it is the weakest of
the three. Cross-platform agreement was established for the 0.1.0 figures and has not yet been
re-established for 1.0.0. The determinism argument is unchanged, nothing platform-dependent
enters a report, and the pinned test literals would fail a machine that disagreed, but the
argument is no longer backed by three machines having agreed on the figures this release
actually emits.

The mainnet row is the weaker of the two and is stated that way deliberately. The two hosts
run the same Ubuntu release on the same architecture with the same pinned toolchain, and
their release binaries are byte-identical, so the agreement shows the evidence survives a
change of host and nothing more. It is not the cross-platform result the testnet row records,
and it should not be cited as one.

### What the testnet row is worth

Determinism across machines was demonstrated on three: the same report hash reproduced byte
for byte on Ubuntu, on Windows/WSL2, and on **native Windows**, from the same commit. All
three were `x86_64`.

The native Windows run is worth singling out, because it did not agree on the first attempt.
WSL2 runs a Linux kernel, so it had tested a different host rather than a different operating
system. Native Windows was the first genuinely different platform and it exposed two real
defects: line-ending translation corrupted the committed evidence on checkout, and a path
separator reaching a hashed check result made the tool compute a **different report hash from
identical evidence**. Both are fixed, and the repository now pins the testnet and mainnet
report hashes as test literals.

A CI job that runs the suite on `windows-latest` and `macos-latest` is configured in
`.github/workflows/ci.yml`. **It has never executed**, because no push has reached a
repository with Actions enabled, so it is a stated intention rather than a result. macOS has
never run this suite at all.

That history is the reason this step is worth your time. Two prior reproductions and a full
audit had all passed.

### What is worth running, in order of what it would settle

1. **Any mainnet hash on a machine the publisher does not operate.** This is the largest gap.
   Every mainnet figure published here rests on two near-identical hosts.
2. **A different CPU architecture.** If you have an `aarch64` machine, an Apple Silicon Mac,
   an ARM server, a Raspberry Pi, running steps 2 and 4 there would close the last variable
   anyone has raised. Both architectures are little-endian, so a difference would be
   surprising; surprising is exactly what is worth checking.
3. **macOS on any architecture**, which no run has covered.

A second run on any machine other than your own is still worth reporting even if it is Ubuntu
on `x86_64`.

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
  sha256sum mainnet-3428141-3428146.tar.zst
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

Three bundles are committed, so pick one and set `$Bundle` and `$Expected` together. They must
match; a mainnet bundle checked against a testnet hash reports `NO` for the reason you would
expect and not for an interesting one.

| `$Bundle` | `$Expected` |
| --- | --- |
| `mainnet-activation-boundary` | `e10929941f3cdf05217f939b0e2f8c93efccfc3bb6b81805c75f26f29eb0b66e` |
| `testnet-ironwood` | `9a103df10d29704708864da39d320bd0ad1e0bafd02f8a2f785760fbca0785aa` |
| `testnet-activation-boundary` | `b60fdf9586ae565764059fea09f4ff07d7a5ca0268e9c04172cd2dfe7d3cba83` |

```powershell
$Bundle   = "mainnet-activation-boundary"
$Expected = "e10929941f3cdf05217f939b0e2f8c93efccfc3bb6b81805c75f26f29eb0b66e"
$Commit   = "v1.0.0"

if (Test-Path repro-zec) { Remove-Item -Recurse -Force repro-zec }
git clone --quiet https://github.com/steven3002/zec-ironwood-reconcile.git repro-zec
Set-Location repro-zec
git checkout --quiet $Commit
cargo build --release

$Bin = ".\target\release\zec-ironwood-reconcile.exe"
& $Bin reconcile (Join-Path "tests\fixtures\bundles" $Bundle) --output .\out
$Observed = (Get-Content .\out\report.sha256).Split(" ")[0]

Write-Output ""
Write-Output "=============== COPY EVERYTHING BELOW THIS LINE ==============="
Write-Output ("date           : " + (Get-Date).ToUniversalTime().ToString("yyyy-MM-dd HH:mm") + " UTC")
Write-Output ("os             : " + (Get-CimInstance Win32_OperatingSystem).Caption)
Write-Output ("arch           : " + $env:PROCESSOR_ARCHITECTURE)
Write-Output ("rustc          : " + (rustc --version))
Write-Output ("commit built   : " + (git rev-parse HEAD))
Write-Output ("tool version   : " + (& $Bin --version))
Write-Output ("bundle         : " + $Bundle)
Write-Output ""
Write-Output ("expected hash  : " + $Expected)
Write-Output ("observed hash  : " + $Observed)
Write-Output ("match          : " + $(if ($Observed -eq $Expected) { "YES" } else { "NO" }))
Write-Output "=============== COPY EVERYTHING ABOVE THIS LINE ==============="
```

**What has been run here, precisely.** This block was executed on native Windows, in
PowerShell, against `testnet-ironwood` at commit `2f9b5c030c121d49706ee7d1fde7b10a3dfa52d0`,
under version 0.1.0, and reproduced `4a5d4d76…`, the figure that version emitted. This
release emits `9a103df1…` for the same evidence. The mainnet pairing is the same code path
over a different
directory and has **not** been run on Windows. If you run it, you are the first, and that is
worth reporting whichever way it comes out.

Getting the Windows path working at all took two fixes to the repository, both described in
step 6. If you are working from a clone taken before commit `f2372e1`, expect either
`evidence_hash_mismatch` on `anchor/value-pools.json` or a hash of `fd43e96c…`; pull first. If
you see either symptom on a current clone, that is a finding worth reporting rather than
something to work around.
