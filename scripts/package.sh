#!/usr/bin/env bash
# Build the release archive for one target from an already built binary.
#
#   scripts/package.sh <target-triple> [path/to/quack[.exe]] [out-dir]
#
# Writes one archive to out-dir (default dist/):
#   quack-<target>.tar.gz  the binary and the defaults bundle (install.sh --plucked filters
#                          out hosted-service providers when installing)
# Windows targets get .zip instead.
#
# Unix layout is a prefix (bin/quack, share/quack/defaults/) so it can be unpacked
# straight into ~/.local or /usr/local. Windows is flat (quack.exe, defaults\), matching
# where the binary looks for its bundle.
set -euo pipefail

target=${1:?usage: package.sh <target-triple> [binary] [out-dir]}
root=$(cd "$(dirname "$0")/.." && pwd)
case $target in
  *windows*) exe=quack.exe ;;
  *) exe=quack ;;
esac
bin=${2:-$root/target/$target/release/$exe}
out=${3:-$root/dist}
[ -f "$bin" ] || { echo "package.sh: no binary at $bin" >&2; exit 1; }

mkdir -p "$out"
out=$(cd "$out" && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

dir=$stage/quack-$target
mkdir -p "$dir"
cp "$root/LICENSE" "$root/NOTICE" "$dir/"
case $target in
  *windows*)
    cp "$bin" "$dir/$exe"
    "$root/scripts/copy-defaults.sh" "$dir/defaults"
    rm -f "$out/quack-$target.zip"
    if command -v 7z >/dev/null; then
      (cd "$stage" && 7z a -tzip -bso0 "$out/quack-$target.zip" "quack-$target")
    else
      (cd "$stage" && zip -qr "$out/quack-$target.zip" "quack-$target")
    fi
    ;;
  *)
    mkdir -p "$dir/bin"
    cp "$bin" "$dir/bin/$exe"
    chmod 755 "$dir/bin/$exe"
    "$root/scripts/copy-defaults.sh" "$dir/share/quack/defaults"
    tar -C "$stage" -czf "$out/quack-$target.tar.gz" "quack-$target"
    ;;
esac

ls -1 "$out" | grep -- "-$target\." | sed "s|^|$out/|"
