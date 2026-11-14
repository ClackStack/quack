#!/bin/sh
# No-egress invariant, enforced by the OS (Linux only; needs strace and iproute2).
#
# Runs the invariant tests (startup + turns with the whole defaults bundle installed) in a
# network namespace that has only loopback, tracing every connect() and every UDP send
# (sendto, sendmmsg) from every thread. Fails if anything sends anywhere but 127.0.0.1 or
# ::1, DNS lookups included: a lookup leaves via sendto on port 53, not connect.
# The in-process version of this test lives in crates/quack-core/tests/invariants.rs.
set -eu

bin=$(cargo test -p quack-core --test invariants --no-run --locked --message-format=json \
    | sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' | grep invariants | tail -n 1)
# strace creates the log itself, inside the namespace: on hardened images (Ubuntu 24.04+)
# the sudo fallback runs as root, and opening the runner-owned mktemp file in sticky /tmp
# is denied (fs.protected_regular=2). The 0700 mktemp directory gates access afterwards.
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
log=$dir/trace

# Unprivileged namespaces are disabled on some distributions (Ubuntu 24.04+); fall back to sudo.
if unshare -rn true 2>/dev/null; then ns="unshare -rn"; else ns="sudo unshare -n"; fi
$ns sh -c "ip link set lo up && strace -f -qq -e trace=connect,sendto,sendmmsg -o '$log' '$bin' --test-threads 1; rc=\$?; chmod 644 '$log' 2>/dev/null; exit \$rc"

if [ ! -r "$log" ]; then
    echo "no-egress: no readable trace at $log" >&2
    exit 1
fi
total=$(grep -cE 'connect\(|sendto\(|sendmmsg\(' "$log" || true)
# Sends without an address (an already-connected socket, or a UNIX one) carry no
# sa_family and cannot point off the machine; everything addressed must be loopback.
bad=$(grep -E 'sa_family=AF_INET6?,' "$log" \
    | grep -v -E 'inet_addr\("127\.0\.0\.1"\)|inet_pton\(AF_INET6, "::1"' || true)
if [ -n "$bad" ]; then
    echo "no-egress: unexpected connections or sends:" >&2
    echo "$bad" >&2
    exit 1
fi
echo "no-egress: $total connect()/send calls, all to loopback"
