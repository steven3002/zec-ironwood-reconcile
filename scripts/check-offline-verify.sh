#!/usr/bin/env bash
#
# Asserts that offline verification opens no network connection.
#
# The tool's central promise is that a published archive plus a published hash is enough for
# an unrelated person to reproduce a result with no node and no internet access. The module
# dependency rules make that structural and `tests/layering.rs` enforces them, but both are
# statements about source code. This checks the shipped binary's behaviour instead.
#
# Syscall tracing is used rather than a network namespace because it answers a stronger
# question. A namespace shows the process could not have reached anything; a trace shows it
# never asked to. It also needs no privileges, so it runs in an unprivileged container.
#
# `capture` is traced as a positive control. Without it, a run in which tracing silently
# recorded nothing would look identical to success.

set -euo pipefail

readonly REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly BINARY="${REPO_ROOT}/target/release/zec-ironwood-reconcile"
readonly ARCHIVE="${REPO_ROOT}/target/offline-check/evidence.tar.zst"
readonly TRACE_DIR="$(mktemp -d)"

# Networking syscalls. `socketpair` is included because it creates sockets even though it
# cannot reach a network, and its absence keeps the assertion simple to state.
readonly NETWORK_SYSCALLS='socket,connect,sendto,recvfrom,sendmsg,recvmsg,bind,listen,accept,accept4,socketpair'
readonly NETWORK_PATTERN='\b(socket|connect|sendto|recvfrom|sendmsg|recvmsg|bind|listen|accept|accept4|socketpair)\('

cleanup() { rm -rf "${TRACE_DIR}"; }
trap cleanup EXIT

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

command -v strace >/dev/null 2>&1 || fail "strace is not installed"
[ -x "${BINARY}" ] || fail "release binary not found; run: cargo build --release"
[ -f "${ARCHIVE}" ] || fail "check archive not found; run: cargo test --test capture"

# Counts network syscalls made by a traced command, whatever its exit status: a command that
# failed after opening a socket must still be reported.
count_network_syscalls() {
    local label="$1"; shift
    local trace="${TRACE_DIR}/${label}.trace"

    strace -f -e "trace=${NETWORK_SYSCALLS}" -o "${trace}" "$@" >/dev/null 2>&1 || true
    [ -f "${trace}" ] || fail "tracing produced no output for ${label}"

    grep -cE "${NETWORK_PATTERN}" "${trace}" || true
}

printf 'Tracing offline commands...\n'

for command in verify inspect; do
    case "${command}" in
        verify)  count="$(count_network_syscalls "${command}" "${BINARY}" verify "${ARCHIVE}")" ;;
        inspect) count="$(count_network_syscalls "${command}" "${BINARY}" inspect "${ARCHIVE}")" ;;
    esac

    if [ "${count}" -ne 0 ]; then
        printf '%s issued %s network syscall(s):\n' "${command}" "${count}" >&2
        grep -E "${NETWORK_PATTERN}" "${TRACE_DIR}/${command}.trace" | head -20 >&2
        fail "${command} touched the network"
    fi
    printf '  %-22s 0 network syscalls\n' "${command}"
done

# A single happy path proves little on its own: a different input could reach code the run
# never executed. The failure paths are where that matters most, because they run different
# extraction, validation, and error-formatting code. Each variant below is derived from the
# valid archive so the scenarios stay in step with it.
printf 'Tracing failure paths...\n'

readonly VARIANTS="${TRACE_DIR}/variants"
mkdir -p "${VARIANTS}"

cp "${ARCHIVE}" "${VARIANTS}/tampered.tar.zst"
printf 'corrupted' | dd of="${VARIANTS}/tampered.tar.zst" bs=1 seek=64 conv=notrunc status=none

head -c 200 "${ARCHIVE}" > "${VARIANTS}/truncated.tar.zst"
printf 'not an archive at all' > "${VARIANTS}/garbage.tar.zst"

check_no_network() {
    local label="$1"; shift
    local count
    count="$(count_network_syscalls "${label}" "$@")"

    if [ "${count}" -ne 0 ]; then
        printf '%s issued %s network syscall(s):\n' "${label}" "${count}" >&2
        grep -E "${NETWORK_PATTERN}" "${TRACE_DIR}/${label}.trace" | head -20 >&2
        fail "${label} touched the network"
    fi
    printf '  %-22s 0 network syscalls\n' "${label}"
}

check_no_network tampered      "${BINARY}" verify "${VARIANTS}/tampered.tar.zst"
check_no_network truncated     "${BINARY}" verify "${VARIANTS}/truncated.tar.zst"
check_no_network garbage       "${BINARY}" verify "${VARIANTS}/garbage.tar.zst"
check_no_network missing-file  "${BINARY}" verify "${VARIANTS}/does-not-exist.tar.zst"
check_no_network wrong-hash    "${BINARY}" verify "${ARCHIVE}" --expected-report-hash "$(printf '0%.0s' $(seq 64))"
check_no_network inspect-dir   "${BINARY}" inspect "${VARIANTS}"

# Positive control. `capture` must reach the network, and its failure to connect to an
# endpoint that is not listening is expected, what matters is that it tried.
#
# It doubles as proof that tracing sees more than this crate: every syscall it records is
# issued from inside `ureq`, since no source file here names a socket operation.
printf 'Tracing the positive control...\n'
control="$(count_network_syscalls capture \
    "${BINARY}" --quiet capture \
    --rpc-url http://127.0.0.1:1 \
    --network mainnet \
    --from-height 3428143 --to-height 3428144 \
    --output "${TRACE_DIR}/control-bundle")"

if [ "${control}" -eq 0 ]; then
    fail "the positive control issued no network syscall, so tracing is not observing anything"
fi
printf '  %-22s %s network syscalls (expected)\n' 'capture' "${control}"

printf '\nOffline verification performs no network access.\n'
