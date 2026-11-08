#!/bin/sh
# Regenerate defaults/catalog.json from `catalog.url` in defaults/config.toml, for every
# provider in the bundle. Run it before a release and read the diff before committing:
# it is what people see as prices until they refresh their own copy.
#
# Uses empty user directories, so a developer's own providers don't end up in the bundle.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cd "$root"
cargo build -q -p quack-cli --locked
cd "$tmp"
env HOME="$tmp" XDG_CONFIG_HOME="$tmp/config" XDG_DATA_HOME="$tmp/data" \
    XDG_CACHE_HOME="$tmp/cache" QUACK_SYSTEM_DIR="$tmp/system" \
    QUACK_DEFAULTS_DIR="$root/defaults" \
    "$root/target/debug/quack" catalog refresh --out "$root/defaults/catalog.json"
