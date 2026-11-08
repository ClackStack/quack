#!/usr/bin/env bash
# Copy the defaults bundle into a package, without editor or OS leftovers.
#
#   scripts/copy-defaults.sh <dest-dir> [--plucked]
#
# --plucked leaves out every provider file whose base_url is not on this machine (hosted
# services, Claude, OpenAI, ChatGPT), so the bundle names no outside host to call. Everything
# else stays: profiles, sub-agents, language servers, MCP examples (off until enabled),
# hooks, the local server files (127.0.0.1), and the model catalog, which names no hosts.
# config.toml goes too: all it holds is where the opt-in catalog and version checks look.
set -euo pipefail

dest=${1:?usage: copy-defaults.sh <dest-dir> [--plucked]}
plucked=${2:-}
root=$(cd "$(dirname "$0")/.." && pwd)

mkdir -p "$dest"
(cd "$root/defaults" && tar cf - --exclude='*/.*' --exclude='*~' .) | (cd "$dest" && tar xf -)

[ "$plucked" = --plucked ] || exit 0
rm -f "$dest/config.toml"
for f in "$dest"/providers.d/*.toml; do
  host=$(sed -n 's|^base_url *= *"[a-z]*://\([^/:"]*\).*|\1|p' "$f" | head -n 1)
  case $host in
    127.* | localhost | "[::1]") ;;
    *) rm -f "$f" ;;
  esac
done
