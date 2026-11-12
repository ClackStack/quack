#!/usr/bin/env bash
# Stamp a release version into the workspace before building.
#
#   scripts/set-version.sh <version>
#
# Rewrites [workspace.package] version in Cargo.toml (every crate inherits it
# through version.workspace = true, so env!("CARGO_PKG_VERSION") follows) and
# refreshes Cargo.lock for the workspace members only, so --locked builds keep
# passing. The release workflow runs this with the tag minus its v; the version
# committed to the tree can lag behind.
set -euo pipefail

version=${1:?usage: set-version.sh <version>}
root=$(cd "$(dirname "$0")/.." && pwd)
case $version in
  v*) echo "set-version.sh: pass the tag without its v, e.g. 0.2.0, not $version" >&2; exit 1 ;;
esac
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || {
  echo "set-version.sh: '$version' is not a version like 0.2.0 or 0.3.1-rc1" >&2
  exit 1
}

# Populate the registry cache from the committed lock before stamping: the
# refresh below runs --offline, and on a fresh runner with an empty cargo home
# offline resolution cannot see the crates.io index at all. Fetching here also
# pins what that refresh can pick to versions already in Cargo.lock. It must
# run while Cargo.toml and Cargo.lock still agree, so before the stamp.
(cd "$root" && cargo fetch --locked --quiet)

toml=$root/Cargo.toml
awk -v ver="$version" '
  /^\[/ { in_pkg = ($0 == "[workspace.package]") }
  in_pkg && /^version = / { print "version = \"" ver "\""; next }
  { print }
' "$toml" > "$toml.stamped" && mv "$toml.stamped" "$toml"

(cd "$root" && cargo update --workspace --offline --quiet)
echo "set version $version"
