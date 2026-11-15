#!/bin/sh
# Install quack (TUI and desktop GUI) on Linux or macOS.
#
#   curl -fsSL https://quack.clackstack.dev/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --tui
#   curl -fsSL .../install.sh | sh -s -- --gui
#   curl -fsSL .../install.sh | sh -s -- --plucked
#
# By default this installs BOTH the TUI (`quack`) and desktop GUI (`quack-gui`), which are
# separate apps that share your config, sessions, memory notes and prompt history. Pass
# --tui or --gui to install only one. --plucked installs the bundle without hosted-service
# provider files (OpenRouter, Claude, OpenAI, ...), for setups with their own provider.
# --binary-only updates binaries and leaves installed defaults alone.
set -eu

repo=ClackStack/quack
prefix=${QUACK_INSTALL_PREFIX:-$HOME/.local}
version=latest
want_tui=0
want_gui=0
plucked=0
binary_only=0
archive=
source_dir=
target=
uninstall=0

usage() {
  cat <<'EOF'
Install quack (TUI and desktop GUI).

Usage: install.sh [options]

  --tui, --cli         Install only the terminal UI (`quack`) and its defaults bundle.
  --gui                Install only the desktop GUI (`quack-gui`). On Linux this needs
                       glibc (AppImage); on macOS it installs quack.app + a CLI wrapper.
                       (Default with neither flag: install both TUI and GUI.)
  --plucked            Leave out the provider files for hosted services (OpenRouter,
                       Claude, OpenAI, ...) for both TUI and GUI. Profiles, sub-agents,
                       MCP, language servers and local server files are all installed.
  --binary-only        Install or update only the binaries. Existing external defaults
                       are left as they are; none are installed if there were none.
  --prefix DIR         Install into DIR/bin and DIR/share/quack (default: ~/.local,
                       or $QUACK_INSTALL_PREFIX).
  --version VER        A release tag such as v0.2.0 (default: the latest release).
  --archive FILE       Install from a downloaded quack-*.tar.gz (TUI) or .AppImage/.dmg
                       (GUI) instead of downloading.
  --from-source [DIR]  Build with cargo from a checkout (default: the one holding this
                       script) and install that.
  --target TRIPLE      Target triple to build for with --from-source (e.g.
                       x86_64-unknown-linux-musl).
  --uninstall          Remove the selected binaries (both TUI and GUI by default), GUI
                       desktop/app entry, and defaults. Config, sessions and memory stay.
  -h, --help           Show this help.

A full install replaces the defaults folder, so files you deleted from it come back.
To turn a default off for good, override it in your own config instead (for example
`enabled = false` in ~/.config/quack/providers.d/<id>.toml), or update with --binary-only.
EOF
}

say() { printf 'quack: %s\n' "$*"; }
die() { printf 'quack: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case $1 in
    --tui | --cli) want_tui=1 ;;
    --gui) want_gui=1 ;;
    --plucked) plucked=1 ;;
    --binary-only) binary_only=1 ;;
    --prefix) [ $# -ge 2 ] || die "--prefix needs a directory"; prefix=$2; shift ;;
    --prefix=*) prefix=${1#*=} ;;
    --version) [ $# -ge 2 ] || die "--version needs a tag"; version=$2; shift ;;
    --version=*) version=${1#*=} ;;
    --archive) [ $# -ge 2 ] || die "--archive needs a file"; archive=$2; shift ;;
    --archive=*) archive=${1#*=} ;;
    --from-source)
      if [ $# -ge 2 ] && [ "${2#-}" = "$2" ]; then source_dir=$2; shift
      else source_dir=$(cd "$(dirname "$0")/.." 2>/dev/null && pwd) || die "can't find the checkout"
      fi ;;
    --target) [ $# -ge 2 ] || die "--target needs a triple"; target=$2; shift ;;
    --target=*) target=${1#*=} ;;
    --uninstall) uninstall=1 ;;
    -h | --help) usage; exit 0 ;;
    *) die "unknown option $1 (see --help)" ;;
  esac
  shift
done

explicit_tui=$want_tui
explicit_gui=$want_gui

if [ "$want_tui" = 0 ] && [ "$want_gui" = 0 ]; then
  if [ -n "$archive" ]; then
    case $archive in
      *.AppImage | *.appimage | *.dmg | *.DMG) want_gui=1 ;;
      *) want_tui=1 ;;
    esac
  else
    want_tui=1
    want_gui=1
  fi
fi

bindir=$prefix/bin
defaults=$prefix/share/quack/defaults
host_os=$(uname -s)

desktop_file_path() {
  if [ "$(id -u)" -eq 0 ] && { [ "$prefix" = /usr/local ] || [ "$prefix" = /usr ]; }; then
    printf '%s' /usr/local/share/applications/quack-gui.desktop
  else
    printf '%s' "${XDG_DATA_HOME:-$HOME/.local/share}/applications/quack-gui.desktop"
  fi
}

macos_app_dir() {
  if [ -n "${QUACK_APP_DIR:-}" ]; then
    printf '%s' "$QUACK_APP_DIR"
  elif [ -w /Applications ]; then
    printf '%s' /Applications
  else
    printf '%s' "$HOME/Applications"
  fi
}

install_bin() {
  _src=$1 _name=$2
  mkdir -p "$bindir"
  cp "$_src" "$bindir/.$_name.new"
  chmod 755 "$bindir/.$_name.new"
  mv -f "$bindir/.$_name.new" "$bindir/$_name"
  say "installed $bindir/$_name"
}

install_linux_desktop() {
  _icon=
  if [ -n "$1" ] && [ -f "$1" ]; then
    _icondir="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/128x128/apps"
    mkdir -p "$_icondir"
    cp "$1" "$_icondir/quack-gui.png"
    _icon=quack-gui
  fi
  _df=$(desktop_file_path)
  mkdir -p "$(dirname "$_df")"
  {
    printf '[Desktop Entry]\n'
    printf 'Type=Application\n'
    printf 'Name=Quack\n'
    printf 'Comment=Desktop coding agent for open-weight models\n'
    printf 'Exec="%s"\n' "$bindir/quack-gui"
    printf 'TryExec=%s\n' "$bindir/quack-gui"
    [ -n "$_icon" ] && printf 'Icon=%s\n' "$_icon"
    printf 'Terminal=false\n'
    printf 'Categories=Development;IDE;\n'
    printf 'StartupWMClass=quack-gui\n'
  } > "$_df.new"
  mv -f "$_df.new" "$_df"
  if command -v update-desktop-database >/dev/null; then
    update-desktop-database "$(dirname "$_df")" >/dev/null 2>&1 || true
  fi
  say "installed desktop entry $_df"
}

if [ "$uninstall" = 1 ]; then
  if [ "$want_tui" = 1 ]; then
    rm -f "$bindir/quack"
    say "removed $bindir/quack"
  fi
  if [ "$want_gui" = 1 ]; then
    rm -f "$bindir/quack-gui"
    _df=$(desktop_file_path)
    rm -f "$_df" "${XDG_DATA_HOME:-$HOME/.local/share}/applications/quack-gui.desktop" "$prefix/share/applications/quack-gui.desktop"
    [ -w /usr/local/share/applications/quack-gui.desktop ] 2>/dev/null && rm -f /usr/local/share/applications/quack-gui.desktop || true
    rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/128x128/apps/quack-gui.png"
    if [ "$host_os" = Darwin ]; then
      for _d in "${QUACK_APP_DIR:-}" "$HOME/Applications" /Applications; do
        [ -n "$_d" ] && [ -d "$_d/quack.app" ] && [ -w "$_d" ] && rm -rf "$_d/quack.app" && say "removed $_d/quack.app"
      done
    fi
    say "removed $bindir/quack-gui"
  fi
  if [ ! -e "$bindir/quack" ] && [ ! -e "$bindir/quack-gui" ]; then
    rm -rf "$defaults"
    rmdir "$prefix/share/quack" 2>/dev/null || true
    say "removed $defaults"
  fi
  exit 0
fi

tmp=$(mktemp -d)
mounted_dmg=
cleanup() {
  if [ -n "$mounted_dmg" ] && command -v hdiutil >/dev/null; then
    hdiutil detach "$mounted_dmg" -quiet 2>/dev/null || hdiutil detach "$mounted_dmg" -force -quiet 2>/dev/null || true
  fi
  rm -rf "$tmp"
}
trap cleanup EXIT INT TERM

install_defaults_dir() {
  _src_defaults=$1
  if [ "$binary_only" = 1 ]; then
    if [ -d "$defaults" ]; then
      say "left the defaults in $defaults as they are (--binary-only)"
    else
      say "no defaults installed (--binary-only): add providers with \`quack setup\` or your own files"
    fi
  elif [ -z "$_src_defaults" ]; then
    say "this archive has no defaults bundle; left $defaults as it is"
  else
    mkdir -p "$prefix/share/quack"
    rm -rf "$defaults.new"
    cp -R "$_src_defaults" "$defaults.new"
    rm -rf "$defaults"
    mv "$defaults.new" "$defaults"
    say "installed the defaults in $defaults"
  fi
}

install_macos_dmg() {
  _dmg=$1
  _mnt=$tmp/dmg
  mkdir -p "$_mnt"
  hdiutil attach "$_dmg" -nobrowse -readonly -mountpoint "$_mnt" >/dev/null || die "could not mount $_dmg"
  mounted_dmg=$_mnt
  _src_app=$(find "$_mnt" -maxdepth 1 -name '*.app' | head -n 1)
  [ -n "$_src_app" ] && [ -d "$_src_app" ] || die "$_dmg does not contain a .app bundle"
  _app_dir=$(macos_app_dir)
  mkdir -p "$_app_dir"
  _app_dest="$_app_dir/$(basename "$_src_app")"
  rm -rf "$_app_dest.new"
  if command -v ditto >/dev/null; then
    ditto "$_src_app" "$_app_dest.new" || die "failed to copy $_src_app to $_app_dest"
  else
    cp -R "$_src_app" "$_app_dest.new" || die "failed to copy $_src_app to $_app_dest"
  fi
  rm -rf "$_app_dest"
  mv "$_app_dest.new" "$_app_dest"
  hdiutil detach "$_mnt" -quiet 2>/dev/null || hdiutil detach "$_mnt" -force -quiet 2>/dev/null || true
  mounted_dmg=

  if [ -x "$_app_dest/Contents/MacOS/quack-gui" ]; then
    _gui_exec="$_app_dest/Contents/MacOS/quack-gui"
  else
    _gui_exec=$(find "$_app_dest/Contents/MacOS" -maxdepth 1 -type f -perm -111 | head -n 1)
  fi
  [ -n "$_gui_exec" ] && [ -x "$_gui_exec" ] || die "could not find quack-gui executable inside $_app_dest"

  mkdir -p "$bindir"
  {
    printf '#!/bin/sh\n'
    printf 'exec "%s" "$@"\n' "$_gui_exec"
  } > "$bindir/.quack-gui.new"
  chmod 755 "$bindir/.quack-gui.new"
  mv -f "$bindir/.quack-gui.new" "$bindir/quack-gui"
  say "installed $_app_dest and $bindir/quack-gui"
}

is_musl_linux() {
  if getconf GNU_LIBC_VERSION >/dev/null 2>&1; then
    return 1
  fi
  case "$(ldd --version 2>&1 || true)" in
    *musl*) return 0 ;;
  esac
  if ls /lib/ld-musl-*.so* >/dev/null 2>&1; then
    return 0
  fi
  return 1
}

can_build_gui_from_source() {
  case $target in
    *-musl*) return 1 ;;
  esac
  if ! command -v pnpm >/dev/null && ! command -v npm >/dev/null; then
    return 1
  fi
  if [ "$host_os" = Linux ] && command -v pkg-config >/dev/null; then
    pkg-config --exists webkit2gtk-4.1 2>/dev/null || return 1
  fi
  return 0
}

new_tui_bin=
new_gui_bin=
new_defaults=
gui_dmg=
gui_appimage=

if [ -n "$source_dir" ]; then
  [ -f "$source_dir/Cargo.toml" ] || die "$source_dir is not a quack checkout"
  command -v cargo >/dev/null || die "cargo is needed for --from-source (https://rustup.rs)"
  target_root=${CARGO_TARGET_DIR:-$source_dir/target}
  case $target_root in
    /*) ;;
    *) target_root=$source_dir/$target_root ;;
  esac
  if [ -n "$target" ]; then
    rel_dir=$target_root/$target/release
  else
    rel_dir=$target_root/release
  fi

  if [ "$want_gui" = 1 ] && ! can_build_gui_from_source; then
    if [ "$explicit_gui" = 1 ]; then
      die "building the desktop GUI requires glibc, pnpm/npm and desktop webview libraries (on Linux: webkit2gtk-4.1)"
    fi
    say "note: skipping desktop GUI --from-source (requires glibc, pnpm/npm and webview libs; installing TUI)"
    want_gui=0
  fi

  say "building from $source_dir"
  if [ "$want_tui" = 1 ]; then
    if [ -n "$target" ]; then
      (cd "$source_dir" && cargo build --release --locked -p quack-cli --target "$target") || die "TUI build failed"
    else
      (cd "$source_dir" && cargo build --release --locked -p quack-cli) || die "TUI build failed"
    fi
    new_tui_bin=$rel_dir/quack
  fi

  if [ "$want_gui" = 1 ]; then
    if command -v pnpm >/dev/null; then
      (cd "$source_dir/crates/quack-gui/ui" && pnpm install --frozen-lockfile && pnpm run build) || die "GUI frontend build failed"
    else
      (cd "$source_dir/crates/quack-gui/ui" && npm ci && npm run build) || die "GUI frontend build failed"
    fi
    if [ -n "$target" ]; then
      (cd "$source_dir" && cargo build --release --locked -p quack-gui --features desktop --target "$target") || die "GUI build failed"
    else
      (cd "$source_dir" && cargo build --release --locked -p quack-gui --features desktop) || die "GUI build failed"
    fi
    new_gui_bin=$rel_dir/quack-gui
  fi

  if [ "$want_tui" = 1 ] || [ "$want_gui" = 1 ]; then
    new_defaults=$source_dir/defaults
    if [ "$plucked" = 1 ]; then
      "$source_dir/scripts/copy-defaults.sh" "$tmp/defaults" --plucked || die "copying the defaults failed"
      new_defaults=$tmp/defaults
    fi
  fi
else
  if [ -n "$archive" ]; then
    [ -f "$archive" ] || die "no such archive: $archive"
    case $archive in
      *.tar.gz | *.tgz)
        [ "$explicit_gui" = 0 ] || die "$archive is a TUI archive; pass --tui or use a .AppImage / .dmg for --gui"
        want_tui=1
        want_gui=0
        ;;
      *.AppImage | *.appimage)
        [ "$explicit_tui" = 0 ] || die "$archive is a GUI AppImage; pass --gui or use a .tar.gz for --tui"
        want_tui=0
        want_gui=1
        gui_appimage=$archive
        ;;
      *.dmg | *.DMG)
        [ "$host_os" = Darwin ] || die ".dmg archives are for macOS"
        [ "$explicit_tui" = 0 ] || die "$archive is a GUI disk image; pass --gui or use a .tar.gz for --tui"
        want_tui=0
        want_gui=1
        gui_dmg=$archive
        ;;
      *)
        die "unsupported --archive format '$archive' (expected .tar.gz, .AppImage or .dmg)"
        ;;
    esac
  else
    case $host_os in
      Linux)
        tui_os=unknown-linux-musl
        gui_os=unknown-linux-gnu
        if [ "$want_gui" = 1 ] && is_musl_linux; then
          if [ "$explicit_gui" = 1 ]; then
            die "the desktop GUI AppImage requires glibc (musl Linux is only supported by the TUI; pass --tui)"
          fi
          say "note: musl Linux detected; desktop GUI requires glibc, so installing the TUI only"
          want_gui=0
        fi
        ;;
      Darwin)
        tui_os=apple-darwin
        gui_os=apple-darwin
        ;;
      *)
        die "unsupported OS $host_os; on Windows use install.ps1"
        ;;
    esac
    case $(uname -m) in
      x86_64 | amd64) arch=x86_64 ;;
      aarch64 | arm64) arch=aarch64 ;;
      *) die "unsupported CPU $(uname -m)" ;;
    esac

    if [ "$version" = latest ]; then
      base=https://github.com/$repo/releases/latest/download
    else
      base=https://github.com/$repo/releases/download/$version
    fi
    if command -v curl >/dev/null; then
      fetch() { curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1"; }
    elif command -v wget >/dev/null; then
      fetch() { wget -q --https-only -O "$2" "$1"; }
    else
      die "curl or wget is needed to download"
    fi

    sha256_file() {
      if command -v sha256sum >/dev/null; then
        sha256sum "$1" | cut -d' ' -f1
      else
        shasum -a 256 "$1" | cut -d' ' -f1
      fi
    }

    fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || die "download failed: $base/SHA256SUMS"
    verify_sum() {
      _path=$1 _fname=$2
      _want=$(awk -v f="$_fname" '$2 == f || $2 == "*" f { print $1; exit }' "$tmp/SHA256SUMS")
      [ -n "$_want" ] || die "$_fname is not listed in SHA256SUMS"
      _got=$(sha256_file "$_path")
      [ "$_want" = "$_got" ] || die "checksum mismatch for $_fname"
    }

    if [ "$want_tui" = 1 ]; then
      tui_flavour=quack
      [ "$plucked" = 1 ] && tui_flavour=quack-plucked
      tui_name=$tui_flavour-$arch-$tui_os.tar.gz
      say "downloading $tui_name ($version)"
      fetch "$base/$tui_name" "$tmp/$tui_name" || die "download failed: $base/$tui_name"
      verify_sum "$tmp/$tui_name" "$tui_name"
      archive=$tmp/$tui_name
    fi

    if [ "$want_gui" = 1 ]; then
      gui_flavour=quack-gui
      [ "$plucked" = 1 ] && gui_flavour=quack-gui-plucked
      if [ "$host_os" = Linux ]; then
        gui_name=$gui_flavour-$arch-$gui_os.AppImage
        say "downloading $gui_name ($version)"
        fetch "$base/$gui_name" "$tmp/$gui_name" || die "download failed: $base/$gui_name"
        verify_sum "$tmp/$gui_name" "$gui_name"
        gui_appimage=$tmp/$gui_name
      else
        gui_name=$gui_flavour-$arch-$gui_os.dmg
        say "downloading $gui_name ($version)"
        fetch "$base/$gui_name" "$tmp/$gui_name" || die "download failed: $base/$gui_name"
        verify_sum "$tmp/$gui_name" "$gui_name"
        gui_dmg=$tmp/$gui_name
      fi
    fi
  fi

  if [ "$want_tui" = 1 ]; then
    mkdir "$tmp/x"
    tar -xzf "$archive" -C "$tmp/x" 2>/dev/null || die "$archive is not a quack .tar.gz archive"
    top=$(find "$tmp/x" -mindepth 1 -maxdepth 1 -type d | head -n 1)
    [ -n "$top" ] && [ -f "$top/bin/quack" ] || die "$archive does not look like a quack archive"
    new_tui_bin=$top/bin/quack
    [ -d "$top/share/quack/defaults" ] && new_defaults=$top/share/quack/defaults
  fi
fi

if [ "$want_tui" = 1 ]; then
  install_bin "$new_tui_bin" quack
  install_defaults_dir "$new_defaults"
fi

if [ "$want_gui" = 1 ]; then
  if [ -n "$gui_dmg" ]; then
    install_macos_dmg "$gui_dmg"
  elif [ -n "$gui_appimage" ]; then
    install_bin "$gui_appimage" quack-gui
    [ "$host_os" = Linux ] && install_linux_desktop ""
  else
    install_bin "$new_gui_bin" quack-gui
    # Source builds of quack-gui are bare binaries (outside an AppImage/.app bundle)
    # and read defaults from <exe>/../share/quack/defaults.
    if [ "$want_tui" = 0 ]; then
      install_defaults_dir "$new_defaults"
    fi
    [ "$host_os" = Linux ] && install_linux_desktop "$source_dir/crates/quack-gui/icons/128x128.png"
  fi
fi

case :$PATH: in
  *:"$bindir":*) ;;
  *) say "$bindir is not on your PATH; add it, e.g. export PATH=\"$bindir:\$PATH\"" ;;
esac

if [ "$want_tui" = 1 ] && [ "$want_gui" = 1 ]; then
  say "installed both TUI (\`quack\`) and GUI (\`quack-gui\`); both share one config, sessions and memory"
elif [ "$want_gui" = 1 ]; then
  say "installed the desktop GUI only (\`quack-gui\`); re-run with \`--tui\` anytime to add \`quack\`"
else
  say "installed the TUI only (\`quack\`); re-run with \`--gui\` anytime to add the desktop GUI"
fi

if [ "$plucked" = 1 ]; then
  say "no hosted services are set up (--plucked): add your server with \`quack setup\` or in Settings"
elif [ "$want_tui" = 1 ]; then
  say "run \`quack setup\` to pick a model, then \`quack\` or \`quack-gui\`"
else
  say "open Quack (or run \`quack-gui\`) to get started"
fi
