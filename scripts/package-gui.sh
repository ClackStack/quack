#!/usr/bin/env bash
# Build and stage desktop GUI release bundles for one target.
#
#   scripts/package-gui.sh <target-triple> [out-dir]
#
# Writes unsigned desktop bundles for both quack-gui (full defaults) and
# quack-gui-plucked (no hosted-service providers) to out-dir (default dist/):
#   x86_64-unknown-linux-gnu   -> quack-gui[-plucked]_<version>_amd64.deb,
#                                 quack-gui[-plucked]-<version>-1.x86_64.rpm,
#                                 quack-gui[-plucked]-x86_64-unknown-linux-gnu.AppImage
#   aarch64-unknown-linux-gnu  -> quack-gui[-plucked]_<version>_arm64.deb,
#                                 quack-gui[-plucked]-<version>-1.aarch64.rpm,
#                                 quack-gui[-plucked]-aarch64-unknown-linux-gnu.AppImage
#   x86_64-apple-darwin        -> quack-gui[-plucked]-x86_64-apple-darwin.dmg
#   aarch64-apple-darwin       -> quack-gui[-plucked]-aarch64-apple-darwin.dmg
#   x86_64-pc-windows-msvc     -> quack-gui[-plucked]-x86_64-pc-windows-msvc-setup.exe
#   aarch64-pc-windows-msvc    -> quack-gui[-plucked]-aarch64-pc-windows-msvc-setup.exe
#
# Linux builds target glibc (never musl: no webview runtime exists on static musl).
# Set QUACK_GUI_BUNDLES (comma-separated, e.g. deb) to override the bundle list.
# Set QUACK_GUI_FLAVOURS (comma-separated, e.g. quack-gui-plucked) to override the flavours.
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
flavours=${QUACK_GUI_FLAVOURS:-quack-gui,quack-gui-plucked}

version=$(awk '
  { gsub(/\r/, "") }
  /^\[/ { in_pkg = ($0 == "[workspace.package]") }
  in_pkg && /^version = / { gsub(/^version = "|"/, ""); print; exit }
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
IFS=',' read -ra flavour_list <<< "$flavours"

export CI=true
if [ "$os" = linux ]; then
  # Ubuntu 24.04 ELF relocations (.relr.dyn) crash linuxdeploy's strip; cargo already strips quack-gui.
  export NO_STRIP=true
  # Allow linuxdeploy/appimagetool to run in CI containers without /dev/fuse.
  export APPIMAGE_EXTRACT_AND_RUN=1
fi

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
# Stage plucked defaults under $root/tmp so the path passed to Tauri is relative to
# crates/quack-gui (../../tmp/...). tauri-utils's resource path normalizer drops Windows
# drive prefixes (e.g. C:), turning absolute %TEMP% paths into /Users/... on the repo's drive.
tmp_parent=$root/tmp
tmp_parent_created=false
if [ ! -d "$tmp_parent" ]; then
  mkdir -p "$tmp_parent"
  tmp_parent_created=true
fi
tmp=$(mktemp -d "$tmp_parent/package-gui.XXXXXX")
cleanup() {
  rm -rf "$tmp"
  if [ "$tmp_parent_created" = true ]; then
    rmdir "$tmp_parent" 2>/dev/null || true
  fi
}
trap cleanup EXIT
staged=()

for flavour in "${flavour_list[@]}"; do
  for b in "${bundle_list[@]}"; do
    rm -rf "$bundle_dir/$b"
  done

  case $flavour in
    quack-gui)
      tauri_cfg="{\"version\":\"$version\"}"
      ;;
    quack-gui-plucked)
      rm -rf "$tmp/defaults" "$target_root/$target/release/defaults"
      "$root/scripts/copy-defaults.sh" "$tmp/defaults" --plucked
      plucked_rel=../../tmp/${tmp##*/}/defaults
      tauri_cfg="{\"version\":\"$version\",\"bundle\":{\"resources\":{\"../../defaults/\":null,\"$plucked_rel/\":\"defaults/\"}}}"
      ;;
    *)
      echo "package-gui.sh: unknown flavour '$flavour'" >&2
      exit 1
      ;;
  esac

  run_tauri build \
    --ci \
    --no-sign \
    --features desktop \
    --target "$target" \
    --bundles "$bundles" \
    --config "$tauri_cfg" \
    -- --locked

  for b in "${bundle_list[@]}"; do
    case $b in
      deb)
        src=$(single_match "$bundle_dir/deb" '*.deb')
        dest=$out/${flavour}_${version}_${deb_arch}.deb
        cp "$src" "$dest"
        staged+=("$dest")
        ;;
      rpm)
        src=$(single_match "$bundle_dir/rpm" '*.rpm')
        dest=$out/${flavour}-${version//-/_}-1.$rpm_arch.rpm
        cp "$src" "$dest"
        staged+=("$dest")
        ;;
      appimage)
        src=$(single_match "$bundle_dir/appimage" '*.AppImage')
        dest=$out/$flavour-$target.AppImage
        cp "$src" "$dest"
        chmod 755 "$dest"
        staged+=("$dest")
        ;;
      dmg)
        src=$(single_match "$bundle_dir/dmg" '*.dmg')
        dest=$out/$flavour-$target.dmg
        cp "$src" "$dest"
        staged+=("$dest")
        ;;
      nsis)
        src=$(single_match "$bundle_dir/nsis" '*-setup.exe')
        dest=$out/$flavour-$target-setup.exe
        cp "$src" "$dest"
        staged+=("$dest")
        ;;
      msi)
        src=$(single_match "$bundle_dir/msi" '*.msi')
        dest=$out/$flavour-$target.msi
        cp "$src" "$dest"
        staged+=("$dest")
        ;;
      *)
        echo "package-gui.sh: unknown bundle type '$b'" >&2
        exit 1
        ;;
    esac
  done
done

printf '%s\n' "${staged[@]}"
