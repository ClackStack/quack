#!/usr/bin/env bash
# Build and stage desktop GUI release bundles for one target.
#
#   scripts/package-gui.sh <target-triple> [out-dir]
#
# Writes unsigned desktop bundles named quack-gui-* to out-dir (default dist/):
#   x86_64-unknown-linux-gnu   -> quack-gui_<version>_amd64.deb,
#                                 quack-gui-<version>-1.x86_64.rpm,
#                                 quack-gui-x86_64-unknown-linux-gnu.AppImage
#   aarch64-unknown-linux-gnu  -> quack-gui_<version>_arm64.deb,
#                                 quack-gui-<version>-1.aarch64.rpm,
#                                 quack-gui-aarch64-unknown-linux-gnu.AppImage
#   x86_64-apple-darwin        -> quack-gui-x86_64-apple-darwin.dmg
#   aarch64-apple-darwin       -> quack-gui-aarch64-apple-darwin.dmg
#   x86_64-pc-windows-msvc     -> quack-gui-x86_64-pc-windows-msvc-setup.exe
#   aarch64-pc-windows-msvc    -> quack-gui-aarch64-pc-windows-msvc-setup.exe
#
# Linux builds target glibc (never musl: no webview runtime exists on static musl).
# Set QUACK_GUI_BUNDLES (comma-separated, e.g. deb) to override the bundle list.
set -euo pipefail

target=${1:?usage: package-gui.sh <target-triple> [out-dir]}
root=$(cd "$(dirname "$0")/.." && pwd)
out=${2:-$root/dist}

case $target in
  *-musl*)
    echo "package-gui.sh: GUI builds require glibc on Linux (got $target; musl has no webview)" >&2
    exit 1
    ;;
  x86_64-unknown-linux-gnu)
    os=linux
    deb_arch=amd64
    rpm_arch=x86_64
    default_bundles=deb,rpm,appimage
    ;;
  aarch64-unknown-linux-gnu)
    os=linux
    deb_arch=arm64
    rpm_arch=aarch64
    default_bundles=deb,rpm,appimage
    ;;
  x86_64-apple-darwin | aarch64-apple-darwin)
    os=macos
    default_bundles=dmg
    ;;
  x86_64-pc-windows-msvc | aarch64-pc-windows-msvc)
    os=windows
    default_bundles=nsis
    ;;
  *)
    echo "package-gui.sh: unsupported target '$target'" >&2
    exit 1
    ;;
esac

bundles=${QUACK_GUI_BUNDLES:-$default_bundles}

version=$(awk '
  /^\[/ { in_pkg = ($0 == "[workspace.package]") }
  in_pkg && /^version = / { gsub(/\r/, ""); gsub(/^version = "|"/, ""); print; exit }
' "$root/Cargo.toml")
[ -n "$version" ] || { echo "package-gui.sh: could not read workspace version from Cargo.toml" >&2; exit 1; }

tauri_js_rel=ui/node_modules/@tauri-apps/cli/tauri.js
run_tauri() {
  if [ -f "$root/crates/quack-gui/$tauri_js_rel" ]; then
    command -v node >/dev/null || { echo "package-gui.sh: node is required to run $tauri_js_rel" >&2; exit 1; }
    (cd "$root/crates/quack-gui" && node "$tauri_js_rel" "$@")
  elif command -v cargo-tauri >/dev/null; then
    (cd "$root/crates/quack-gui" && cargo tauri "$@")
  else
    echo "package-gui.sh: Tauri CLI not found; run: pnpm --dir crates/quack-gui/ui install --frozen-lockfile" >&2
    exit 1
  fi
}

target_root=${CARGO_TARGET_DIR:-$root/target}
if command -v cygpath >/dev/null; then
  target_root=$(cygpath -u "$target_root")
fi
bundle_dir=$target_root/$target/release/bundle

IFS=',' read -ra bundle_list <<< "$bundles"
for b in "${bundle_list[@]}"; do
  rm -rf "$bundle_dir/$b"
done

export CI=true
if [ "$os" = linux ]; then
  # Ubuntu 24.04 ELF relocations (.relr.dyn) crash linuxdeploy's strip; cargo already strips quack-gui.
  export NO_STRIP=true
  # Allow linuxdeploy/appimagetool to run in CI containers without /dev/fuse.
  export APPIMAGE_EXTRACT_AND_RUN=1
fi

run_tauri build \
  --ci \
  --no-sign \
  --features desktop \
  --target "$target" \
  --bundles "$bundles" \
  --config "{\"version\":\"$version\"}" \
  -- --locked

single_match() {
  local dir=$1 pattern=$2
  local matches=()
  if [ -d "$dir" ]; then
    while IFS= read -r -d '' f; do
      matches+=("$f")
    done < <(find "$dir" -maxdepth 1 -type f -name "$pattern" -print0 | sort -z)
  fi
  if [ ${#matches[@]} -ne 1 ]; then
    echo "package-gui.sh: expected 1 '$pattern' in $dir, found ${#matches[@]}" >&2
    exit 1
  fi
  printf '%s' "${matches[0]}"
}

mkdir -p "$out"
out=$(cd "$out" && pwd)
staged=()

for b in "${bundle_list[@]}"; do
  case $b in
    deb)
      src=$(single_match "$bundle_dir/deb" '*.deb')
      dest=$out/quack-gui_${version}_${deb_arch}.deb
      cp "$src" "$dest"
      staged+=("$dest")
      ;;
    rpm)
      src=$(single_match "$bundle_dir/rpm" '*.rpm')
      dest=$out/quack-gui-${version//-/_}-1.$rpm_arch.rpm
      cp "$src" "$dest"
      staged+=("$dest")
      ;;
    appimage)
      src=$(single_match "$bundle_dir/appimage" '*.AppImage')
      dest=$out/quack-gui-$target.AppImage
      cp "$src" "$dest"
      chmod 755 "$dest"
      staged+=("$dest")
      ;;
    dmg)
      src=$(single_match "$bundle_dir/dmg" '*.dmg')
      dest=$out/quack-gui-$target.dmg
      cp "$src" "$dest"
      staged+=("$dest")
      ;;
    nsis)
      src=$(single_match "$bundle_dir/nsis" '*-setup.exe')
      dest=$out/quack-gui-$target-setup.exe
      cp "$src" "$dest"
      staged+=("$dest")
      ;;
    msi)
      src=$(single_match "$bundle_dir/msi" '*.msi')
      dest=$out/quack-gui-$target.msi
      cp "$src" "$dest"
      staged+=("$dest")
      ;;
    *)
      echo "package-gui.sh: unknown bundle type '$b'" >&2
      exit 1
      ;;
  esac
done

printf '%s\n' "${staged[@]}"
