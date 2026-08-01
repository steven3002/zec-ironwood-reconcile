#!/usr/bin/env bash
#
# Builds the release binary with its dependency tree embedded, and reads the tree back out.
#
# A published binary should be scannable on its own. Someone holding the artifact, and not
# this repository, not its `Cargo.lock`, needs to be able to ask what went into it when an
# advisory lands against a crate months from now. `cargo-auditable` embeds that list in the
# binary; this script checks that it is actually there and actually parses.
#
# A negative control runs first. `rust-audit-info` reporting a tree tells you nothing unless
# you know it would have reported none had the build not embedded one, so an ordinary release
# build is checked to have no tree before the auditable build is checked to have one. Without
# it, a reader that succeeded unconditionally would be indistinguishable from success.
#
# The auditable build runs second on purpose: it leaves the binary carrying the tree in place
# as the artifact of this script.

set -euo pipefail

readonly REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly BINARY="${REPO_ROOT}/target/release/zec-ironwood-reconcile"

# Crates whose presence in the embedded tree is worth asserting by name: the decoder the
# reconstruction depends on, and the only crate in the graph that can open a socket.
readonly REQUIRED_PACKAGES='zcash_primitives orchard zcash_protocol ureq'

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

command -v cargo-auditable >/dev/null 2>&1 ||
    fail "cargo-auditable is not installed; run: cargo install cargo-auditable --locked"
command -v rust-audit-info >/dev/null 2>&1 ||
    fail "rust-audit-info is not installed; run: cargo install rust-audit-info --locked"

cd "${REPO_ROOT}"

printf 'Negative control: building without the dependency tree...\n'
cargo build --release --quiet
[ -x "${BINARY}" ] || fail "release binary not found at ${BINARY}"

if rust-audit-info "${BINARY}" >/dev/null 2>&1; then
    fail "an ordinary release build already reports a dependency tree, so finding one after
      the auditable build would prove nothing about the build"
fi
printf '  ordinary build         no embedded tree (expected)\n'

printf 'Building with the dependency tree embedded...\n'
cargo auditable build --release --quiet
[ -x "${BINARY}" ] || fail "release binary not found at ${BINARY}"

readonly EMBEDDED="$(rust-audit-info "${BINARY}")" ||
    fail "the binary carries no readable dependency tree"

# Read the tree with a parser rather than a pattern match. A grep over the raw bytes would
# also match a crate name that merely appeared somewhere in the binary's data.
package_count="$(
    printf '%s' "${EMBEDDED}" |
        python3 -c 'import json,sys; print(len(json.load(sys.stdin)["packages"]))'
)" || fail "the embedded dependency tree is not valid JSON"

[ "${package_count}" -gt 1 ] ||
    fail "the embedded tree lists ${package_count} package(s), so it records no dependencies"
printf '  auditable build        %s packages\n' "${package_count}"

for package in ${REQUIRED_PACKAGES}; do
    printf '%s' "${EMBEDDED}" |
        python3 -c '
import json, sys
wanted = sys.argv[1]
tree = json.load(sys.stdin)
match = next((p for p in tree["packages"] if p["name"] == wanted), None)
if match is None:
    sys.exit(f"{wanted} is absent from the embedded dependency tree")
print("  {:<22} {}".format(match["name"], match["version"]))
' "${package}" || fail "the embedded tree does not describe ${package}"
done

printf '\nThe release binary carries a readable dependency tree.\n'
