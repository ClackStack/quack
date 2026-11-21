#!/bin/sh
# Install quack (TUI and desktop GUI) on Linux or macOS.
#
#   curl -fsSL https://quack.clackstack.dev/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --tui
#   curl -fsSL .../install.sh | sh -s -- --gui
#   curl -fsSL .../install.sh | sh -s -- --plucked
#
# With no --tui/--gui flag the installer asks what to install: both the TUI (`quack`) and
# the desktop GUI (`quack-gui`), the default, or just one. They are separate apps that
# share your config, sessions, memory notes and prompt history. When quack is already
# installed the prompt defaults to what is there, so re-running the script updates that
# instead of adding the other app. With no terminal to ask (CI, scripts) the default is
# taken silently.
#
# On Linux the release ships the TUI and GUI as .deb and .rpm packages besides the
# self-contained archives. If quack or quack-gui is already installed as a .deb or .rpm
# package, re-running the installer updates it with that same package format (never
# silently falling back to the archive). On a fresh install it uses a package when the
# machine can install one (.deb on apt/dpkg systems, .rpm on dnf/yum/zypper/rpm systems,
# with root or sudo), falling back to the self-contained archive under ~/.local
# otherwise; --pkg forces a choice. --plucked installs the bundle without hosted-service
# provider files (OpenRouter, Claude, OpenAI, ...), for setups with their own provider
# (works for .deb/.rpm packages as well as archives, and updates to a plucked install keep
# it plucked unless --full is passed). --binary-only updates binaries and leaves
# installed defaults alone; like --prefix, --archive and --from-source, it uses the
# archive path.
set -eu

repo=ClackStack/quack
prefix=${QUACK_INSTALL_PREFIX:-$HOME/.local}
prefix_explicit=0
if [ -n "${QUACK_INSTALL_PREFIX:-}" ]; then prefix_explicit=1; fi
sys_root=${QUACK_INSTALL_SYSROOT:-}
sys_defaults=$sys_root/usr/share/quack/defaults
version=latest
want_tui=0
want_gui=0
plucked=0
full=0
binary_only=0
pkg_forced=
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
                       glibc; on macOS it installs quack.app + a CLI wrapper. With
                       neither flag the installer asks what to install: both by default,
                       or, when quack is already installed, whatever is installed.
  --pkg deb|rpm|archive
                       Linux only: how to install. `deb` and `rpm` are system packages
                       (binaries in /usr/bin, desktop integration and the shared
                       defaults bundle registered system-wide); `archive` is the
                       self-contained bundle under --prefix. Default: the package format
                       already installed (.deb or .rpm), else the first package type this
                       machine can install (with root or sudo), else the archive.
                       `deb` and `rpm` cannot be combined with --binary-only,
                       --prefix, --archive or --from-source, which use the archive.
  --plucked            Leave out the provider files for hosted services (OpenRouter,
                       Claude, OpenAI, ...) for both TUI and GUI. Profiles, sub-agents,
                       MCP, language servers and local server files are all installed.
                       Updates to a plucked install stay plucked automatically.
  --full               Include hosted-service provider files and config.toml again when
                       updating an install that was previously --plucked.
  --binary-only        Install or update only the binaries. Existing defaults
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
  --uninstall          Remove the selected binaries (whatever is installed by default;
                       system packages via apt/dpkg or rpm when installed that way), GUI
                       desktop/app entry, and defaults. Config, sessions and memory stay.
  -h, --help           Show this help.

A full install replaces the defaults folder, so files you deleted from it come back.
To turn a default off for good, override it in your own config instead (for example
`enabled = false` in ~/.config/quack/providers.d/<id>.toml), or update with --binary-only.
EOF
}

say() { printf 'quack: %s\n' "$*"; }
die() { printf 'quack: %s\n' "$*" >&2; exit 1; }

# Ask a question and read one line of answer into $answer. Reads the terminal directly
# when stdin is not one (curl | sh), and falls back to $2, the default, when there is no
# terminal to ask at all (scripts, CI).
ask() {
  answer=
  _line=
  if [ -t 0 ]; then
    printf '%s' "$1"
    IFS= read -r _line || _line=
  elif [ -c /dev/tty ] && { printf '' >/dev/tty; } 2>/dev/null; then
    { printf '%s' "$1"; } 2>/dev/null >/dev/tty || true
    { IFS= read -r _line; } 2>/dev/null </dev/tty || _line=
  fi
  answer=${_line:-$2}
}

# Extract a release tag from a GitHub /releases/tag/<tag> redirect URL.
tag_from_url() {
  printf '%s\n' "$1" | tr -d '\r' \
    | sed -n 's|.*/releases/tag/\([^/?# 	]*\).*|\1|p' \
    | head -n 1
}

# Resolve the "latest" release tag so downloads are pinned to one immutable release.
# Prefers GitHub's /releases/latest redirect (no API rate limit), falling back to the
# GitHub API. Returns non-zero when neither yields a tag.
latest_tag() {
  _rel=https://github.com/$repo/releases/latest
  _tag=
  if command -v curl >/dev/null 2>&1; then
    _loc=$(curl -fsSI --proto '=https' --tlsv1.2 "$_rel" 2>/dev/null \
      | tr -d '\r' \
      | sed -n 's/^[Ll][Oo][Cc][Aa][Tt][Ii][Oo][Nn]:[ 	]*//p' \
      | head -n 1) || _loc=
    if [ -n "$_loc" ]; then
      _tag=$(tag_from_url "$_loc")
    fi
    if [ -z "$_tag" ]; then
      _eff=$(curl -fsSLI --proto '=https' --tlsv1.2 -o /dev/null -w '%{url_effective}' "$_rel" 2>/dev/null) || _eff=
      if [ -n "$_eff" ]; then
        _tag=$(tag_from_url "$_eff")
      fi
    fi
  elif command -v wget >/dev/null 2>&1; then
    _loc=$(wget --https-only --max-redirect=0 -S -O /dev/null "$_rel" 2>&1 \
      | tr -d '\r' \
      | sed -n 's/^[ 	]*[Ll][Oo][Cc][Aa][Tt][Ii][Oo][Nn]:[ 	]*//p' \
      | head -n 1) || _loc=
    if [ -n "$_loc" ]; then
      _tag=$(tag_from_url "$_loc")
    fi
    if [ -z "$_tag" ]; then
      _loc=$(wget --https-only -S --spider "$_rel" 2>&1 \
        | tr -d '\r' \
        | sed -n 's/^[ 	]*[Ll][Oo][Cc][Aa][Tt][Ii][Oo][Nn]:[ 	]*//p' \
        | tail -n 1) || _loc=
      if [ -n "$_loc" ]; then
        _tag=$(tag_from_url "$_loc")
      fi
    fi
  else
    return 1
  fi
  if [ -n "$_tag" ]; then
    printf '%s\n' "$_tag"
    return 0
  fi

  _api=https://api.github.com/repos/$repo/releases/latest
  if command -v curl >/dev/null 2>&1; then
    _json=$(curl -fsSL --proto '=https' --tlsv1.2 "$_api" 2>/dev/null) || return 1
  elif command -v wget >/dev/null 2>&1; then
    _json=$(wget -q --https-only -O - "$_api" 2>/dev/null) || return 1
  else
    return 1
  fi
  _tag=$(printf '%s\n' "$_json" | sed -n 's/.*"tag_name"[: ]*"\([^"]*\)".*/\1/p' | head -n 1)
  [ -n "$_tag" ] || return 1
  printf '%s\n' "$_tag"
}

# --- System packages (.deb / .rpm) -----------------------------------------------

have_root() {
  [ "$(id -u)" -eq 0 ] || command -v sudo >/dev/null 2>&1 || command -v doas >/dev/null 2>&1
}

as_root() {
  if [ "$(id -u)" -eq 0 ]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  elif command -v doas >/dev/null 2>&1; then
    doas "$@"
  else
    die "this step needs root; re-run as root, install sudo, or pass --prefix DIR for a local install"
  fi
}

deb_tooling() {
  command -v apt-get >/dev/null 2>&1 || command -v dpkg >/dev/null 2>&1
}

rpm_tooling() {
  command -v dnf >/dev/null 2>&1 || command -v microdnf >/dev/null 2>&1 \
    || command -v yum >/dev/null 2>&1 || command -v zypper >/dev/null 2>&1 \
    || command -v rpm >/dev/null 2>&1
}

deb_installed() {
  command -v dpkg-query >/dev/null 2>&1 \
    && dpkg-query -W -f='${Status}\n' "$1" 2>/dev/null | grep -q 'ok installed'
}

rpm_installed() {
  command -v rpm >/dev/null 2>&1 && rpm -q "$1" >/dev/null 2>&1
}

# Is $1 installed as a deb or an rpm package?
pkg_installed() {
  deb_installed "$1" || rpm_installed "$1"
}

install_deb() {
  _deb=$1
  case $_deb in
    /*) _deb_arg=$_deb ;;
    *) _deb_arg=./$_deb ;;
  esac
  chmod 755 "$tmp" 2>/dev/null || true
  chmod 644 "$_deb" 2>/dev/null || true
  if command -v apt-get >/dev/null 2>&1; then
    if as_root apt-get install -y --no-install-recommends "$_deb_arg"; then
      return 0
    fi
    say "apt-get could not install $(basename "$_deb"); retrying with dpkg"
  fi
  command -v dpkg >/dev/null 2>&1 || die "neither apt-get nor dpkg can install $_deb"
  as_root dpkg -i "$_deb" || die "dpkg could not install $_deb"
}

install_rpm() {
  _rpm=$1
  if command -v dnf >/dev/null 2>&1; then
    as_root dnf install -y "$_rpm" || die "dnf could not install $_rpm"
  elif command -v microdnf >/dev/null 2>&1; then
    as_root microdnf install -y "$_rpm" || die "microdnf could not install $_rpm"
  elif command -v yum >/dev/null 2>&1; then
    as_root yum install -y "$_rpm" || die "yum could not install $_rpm"
  elif command -v zypper >/dev/null 2>&1; then
    as_root zypper --non-interactive --no-gpg-checks install "$_rpm" || die "zypper could not install $_rpm"
  elif command -v rpm >/dev/null 2>&1; then
    as_root rpm -U --replacepkgs "$_rpm" || die "rpm could not install $_rpm"
  else
    die "no rpm tooling found to install $(basename "$_rpm")"
  fi
}

# Remove $1 through the package manager that owns it. Returns 1 when no package owns it.
pkg_remove() {
  _name=$1
  if deb_installed "$_name"; then
    if command -v apt-get >/dev/null 2>&1; then
      as_root apt-get remove -y "$_name" || die "apt-get could not remove $_name"
    else
      as_root dpkg -r "$_name" || die "dpkg could not remove $_name"
    fi
    return 0
  fi
  if rpm_installed "$_name"; then
    as_root rpm -e "$_name" || die "rpm could not remove $_name"
    return 0
  fi
  return 1
}

while [ $# -gt 0 ]; do
  case $1 in
    --tui | --cli) want_tui=1 ;;
    --gui) want_gui=1 ;;
    --plucked) plucked=1; full=0 ;;
    --full | --no-plucked) full=1; plucked=0 ;;
    --binary-only) binary_only=1 ;;
    --pkg)
      [ $# -ge 2 ] || die "--pkg needs deb, rpm or archive"
      case $2 in
        deb | rpm | archive) ;;
        *) die "--pkg wants deb, rpm or archive (got $2)" ;;
      esac
      pkg_forced=$2
      shift
      ;;
    --pkg=*)
      case ${1#--pkg=} in
        deb | rpm | archive) pkg_forced=${1#--pkg=} ;;
        *) die "--pkg wants deb, rpm or archive (got ${1#--pkg=})" ;;
      esac
      ;;
    --prefix) [ $# -ge 2 ] || die "--prefix needs a directory"; prefix=$2; prefix_explicit=1; shift ;;
    --prefix=*) prefix=${1#*=}; prefix_explicit=1 ;;
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

if [ -n "$pkg_forced" ]; then
  [ "$uninstall" = 0 ] || die "--pkg does not apply to --uninstall"
  [ -z "$source_dir" ] || die "--pkg cannot be combined with --from-source"
  [ -z "$archive" ] || die "--pkg cannot be combined with --archive"
fi

bindir=$prefix/bin

# What is already installed? A copy in $bindir, one anywhere on PATH, or a deb/rpm
# package all count: re-running the installer then defaults to updating what is there.
installed_tui=0
installed_gui=0
installed_tui_deb=0
installed_gui_deb=0
installed_tui_rpm=0
installed_gui_rpm=0
if [ -x "$bindir/quack" ]; then installed_tui=1; fi
if [ -x "$bindir/quack-gui" ]; then installed_gui=1; fi
if command -v quack >/dev/null 2>&1; then installed_tui=1; fi
if command -v quack-gui >/dev/null 2>&1; then installed_gui=1; fi
if deb_installed quack || deb_installed quack-plucked; then
  installed_tui=1
  installed_tui_deb=1
fi
if deb_installed quack-gui || deb_installed quack-gui-plucked; then
  installed_gui=1
  installed_gui_deb=1
fi
if rpm_installed quack || rpm_installed quack-plucked; then
  installed_tui=1
  installed_tui_rpm=1
fi
if rpm_installed quack-gui || rpm_installed quack-gui-plucked; then
  installed_gui=1
  installed_gui_rpm=1
fi

if [ "$want_tui" = 0 ] && [ "$want_gui" = 0 ]; then
  if [ -n "$archive" ]; then
    case $archive in
      *.AppImage | *.appimage | *.dmg | *.DMG) want_gui=1 ;;
      *) want_tui=1 ;;
    esac
  elif [ "$uninstall" = 1 ]; then
    # Uninstall what is there, not everything.
    want_tui=$installed_tui
    want_gui=$installed_gui
  else
    # Ask what to install. A fresh install defaults to both; an update defaults to
    # whatever is already installed (both when both are).
    def=1
    have=both
    if [ "$installed_tui" = 1 ] && [ "$installed_gui" = 1 ]; then
      def=1
    elif [ "$installed_tui" = 1 ]; then
      def=2
      have='the TUI (`quack`)'
    elif [ "$installed_gui" = 1 ]; then
      def=3
      have='the desktop GUI (`quack-gui`)'
    else
      have=none
    fi
    d1=
    d2=
    d3=
    case $def in
      1) d1=' [default]' ;;
      2) d2=' [default]' ;;
      3) d3=' [default]' ;;
    esac
    if [ "$have" = none ]; then
      printf 'quack is not installed yet. What should this install?\n'
    else
      printf 'quack is already installed (%s). Keep that, or switch?\n' "$have"
    fi
    printf '  1) both the TUI (`quack`) and the desktop GUI (`quack-gui`)%s\n' "$d1"
    printf '  2) only the TUI (`quack`)%s\n' "$d2"
    printf '  3) only the desktop GUI (`quack-gui`)%s\n' "$d3"
    while :; do
      ask 'Choose 1, 2 or 3: ' "$def"
      case $answer in
        1 | both | b) want_tui=1; want_gui=1; break ;;
        2 | tui | t) want_tui=1; break ;;
        3 | gui | g) want_gui=1; break ;;
        *) printf 'please answer 1, 2 or 3\n' ;;
      esac
    done
  fi
fi
defaults=$prefix/share/quack/defaults
host_os=$(uname -s)

# If quack or quack-gui is already installed as a system package, record which
# package manager owns it so updates keep using that same format.
installed_pkg_kind=
if [ "$want_tui" = 1 ] && [ "$installed_tui_deb" = 1 ]; then
  installed_pkg_kind=deb
elif [ "$want_gui" = 1 ] && [ "$installed_gui_deb" = 1 ]; then
  installed_pkg_kind=deb
elif [ "$want_tui" = 1 ] && [ "$installed_tui_rpm" = 1 ]; then
  installed_pkg_kind=rpm
elif [ "$want_gui" = 1 ] && [ "$installed_gui_rpm" = 1 ]; then
  installed_pkg_kind=rpm
elif [ "$installed_tui_deb" = 1 ] || [ "$installed_gui_deb" = 1 ]; then
  installed_pkg_kind=deb
elif [ "$installed_tui_rpm" = 1 ] || [ "$installed_gui_rpm" = 1 ]; then
  installed_pkg_kind=rpm
fi

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
    _removed_tui_pkg=0
    if pkg_remove quack; then
      say "removed the quack system package"
      _removed_tui_pkg=1
    fi
    if pkg_remove quack-plucked; then
      say "removed the quack-plucked system package"
      _removed_tui_pkg=1
    fi
    if [ -e "$bindir/quack" ] || [ "$_removed_tui_pkg" = 0 ]; then
      rm -f "$bindir/quack"
      say "removed $bindir/quack"
    fi
  fi
  if [ "$want_gui" = 1 ]; then
    if pkg_remove quack-gui; then
      say "removed the quack-gui system package"
    fi
    if pkg_remove quack-gui-plucked; then
      say "removed the quack-gui-plucked system package"
    fi
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
  if [ ! -e "$bindir/quack" ] && [ ! -e "$bindir/quack-gui" ] \
    && [ ! -e "$sys_root/usr/bin/quack" ] && [ ! -e "$sys_root/usr/bin/quack-gui" ]; then
    rm -rf "$defaults"
    rmdir "$prefix/share/quack" 2>/dev/null || true
    if [ -d "$sys_root/usr/share/quack" ] && have_root; then
      as_root rm -rf "$sys_root/usr/share/quack" || true
    fi
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

has_hosted_provider() {
  for _f in "$1"/providers.d/*.toml; do
    [ -f "$_f" ] || continue
    _host=$(sed -n 's|^base_url *= *"[a-z]*://\([^/:"]*\).*|\1|p' "$_f" | head -n 1)
    case $_host in
      127.* | localhost | "[::1]") ;;
      *) return 0 ;;
    esac
  done
  return 1
}

dir_is_plucked() {
  _d=$1
  [ -d "$_d" ] || return 1
  if [ ! -d "$_d/profiles.d" ] && [ ! -d "$_d/providers.d" ] && [ ! -f "$_d/catalog.json" ]; then
    return 1
  fi
  [ ! -f "$_d/config.toml" ] || return 1
  ! has_hosted_provider "$_d"
}

# Remove config.toml and non-loopback provider files from $1 (with root when $2 = 1).
pluck_defaults_dir() {
  _dir=$1
  _use_root=${2:-0}
  set -- "$_dir/config.toml"
  for _f in "$_dir"/providers.d/*.toml; do
    [ -f "$_f" ] || continue
    _host=$(sed -n 's|^base_url *= *"[a-z]*://\([^/:"]*\).*|\1|p' "$_f" | head -n 1)
    case $_host in
      127.* | localhost | "[::1]") ;;
      *) set -- "$@" "$_f" ;;
    esac
  done
  if [ "$_use_root" = 1 ]; then
    as_root rm -f "$@"
  else
    rm -f "$@"
  fi
}

# Preserve an existing plucked install on updates unless --plucked, --full or --binary-only
# was passed explicitly. $1 is the selected pkg_kind ("deb", "rpm", or "").
maybe_preserve_plucked() {
  _target_pkg=$1
  [ "$plucked" = 0 ] && [ "$full" = 0 ] && [ "$binary_only" = 0 ] || return 0
  _was_plucked=0
  if [ -n "$_target_pkg" ]; then
    if deb_installed quack-plucked || deb_installed quack-gui-plucked \
      || rpm_installed quack-plucked || rpm_installed quack-gui-plucked; then
      _was_plucked=1
    elif dir_is_plucked "$sys_defaults"; then
      _was_plucked=1
    elif [ ! -d "$sys_defaults" ] && dir_is_plucked "$defaults"; then
      _was_plucked=1
    fi
  elif dir_is_plucked "$defaults"; then
    _was_plucked=1
  fi
  if [ "$_was_plucked" = 1 ]; then
    plucked=1
    say "existing installation is plucked; keeping hosted providers omitted (pass --full to include them)"
  fi
}

install_defaults_dir() {
  _src_defaults=$1
  if [ "$binary_only" = 1 ]; then
    if [ -d "$defaults" ]; then
      say "left the defaults in $defaults as they are (--binary-only)"
    else
      say "no defaults installed (--binary-only): add providers with \`quack setup\` or your own files"
    fi
  elif [ -z "$_src_defaults" ]; then
    if [ "$plucked" = 1 ]; then
      say "warning: this archive has no defaults bundle, so --plucked had nothing to filter; left $defaults as it is"
    else
      say "this archive has no defaults bundle; left $defaults as it is"
    fi
  else
    mkdir -p "$prefix/share/quack"
    rm -rf "$defaults.new"
    cp -R "$_src_defaults" "$defaults.new"
    if [ "$plucked" = 1 ]; then
      pluck_defaults_dir "$defaults.new"
    fi
    rm -rf "$defaults"
    mv "$defaults.new" "$defaults"
    if [ "$plucked" = 1 ]; then
      say "installed plucked defaults in $defaults (hosted providers left out)"
    else
      say "installed the defaults in $defaults"
    fi
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
    if [ "$prefix" != "$HOME/.local" ]; then
      printf 'export QUACK_DEFAULTS_DIR="${QUACK_DEFAULTS_DIR:-%s}"\n' "$defaults"
    fi
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
tui_pkg=
gui_pkg=
pkg_kind=

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
    maybe_preserve_plucked ""
    new_defaults=$source_dir/defaults
    if [ "$plucked" = 1 ]; then
      "$source_dir/scripts/copy-defaults.sh" "$tmp/defaults" --plucked || die "copying the defaults failed"
      new_defaults=$tmp/defaults
    fi
  fi
else
  tui_archive=
  if [ -n "$archive" ]; then
    [ -f "$archive" ] || die "no such archive: $archive"
    case $archive in
      *.tar.gz | *.tgz)
        [ "$explicit_gui" = 0 ] || die "$archive is a TUI archive; pass --tui or use a .AppImage / .dmg for --gui"
        want_tui=1
        want_gui=0
        tui_archive=$archive
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
            die "the desktop GUI requires glibc (musl Linux is only supported by the TUI; pass --tui)"
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

    # On Linux, keep updating an existing .deb or .rpm installation with that
    # same package format. On a fresh install (or when updating an archive
    # install), prefer a system package when this machine can install one
    # (.deb on apt/dpkg systems, .rpm on dnf/yum/zypper/rpm systems, with root
    # or sudo) and the release ships one; otherwise use the self-contained
    # archives below. --pkg forces a choice. --binary-only and a custom
    # --prefix use the archive path.
    pkg_kind=
    if [ "$host_os" = Linux ]; then
      if [ -n "$pkg_forced" ]; then
        if [ "$pkg_forced" != archive ]; then
          if [ "$binary_only" = 1 ] || [ "$prefix_explicit" = 1 ]; then
            die "--pkg $pkg_forced cannot be combined with --binary-only or --prefix (those always use the archive)"
          fi
          pkg_kind=$pkg_forced
        fi
      elif [ -n "$installed_pkg_kind" ] && [ "$prefix_explicit" = 0 ]; then
        if [ "$binary_only" = 1 ]; then
          die "quack is already installed as a .$installed_pkg_kind package, which cannot be updated with --binary-only; re-run without --binary-only to update the package, or pass --pkg archive to install a separate copy under $prefix"
        fi
        pkg_kind=$installed_pkg_kind
      elif [ "$binary_only" = 0 ] && [ "$prefix_explicit" = 0 ] && have_root; then
        if command -v apt-get >/dev/null 2>&1; then
          pkg_kind=deb
        elif command -v dnf >/dev/null 2>&1 || command -v microdnf >/dev/null 2>&1 \
          || command -v yum >/dev/null 2>&1 || command -v zypper >/dev/null 2>&1; then
          pkg_kind=rpm
        elif command -v dpkg >/dev/null 2>&1; then
          pkg_kind=deb
        elif command -v rpm >/dev/null 2>&1; then
          pkg_kind=rpm
        fi
      fi
    elif [ -n "$pkg_forced" ]; then
      die "--pkg is Linux only"
    fi
    if [ "$pkg_kind" = deb ] && ! deb_tooling; then
      die "installing a .deb needs apt-get or dpkg on this machine (try --pkg archive)"
    fi
    if [ "$pkg_kind" = deb ] && ! have_root; then
      if [ "$installed_pkg_kind" = deb ] && [ -z "$pkg_forced" ]; then
        die "quack is already installed as a .deb package; updating it needs root or sudo (or pass --pkg archive to install a separate copy under $prefix)"
      fi
      die "installing a .deb needs root or sudo (try --pkg archive)"
    fi
    if [ "$pkg_kind" = rpm ] && ! rpm_tooling; then
      die "installing an .rpm needs dnf, yum, zypper or rpm on this machine (try --pkg archive)"
    fi
    if [ "$pkg_kind" = rpm ] && ! have_root; then
      if [ "$installed_pkg_kind" = rpm ] && [ -z "$pkg_forced" ]; then
        die "quack is already installed as an .rpm package; updating it needs root or sudo (or pass --pkg archive to install a separate copy under $prefix)"
      fi
      die "installing an .rpm needs root or sudo (try --pkg archive)"
    fi

    if [ -n "$installed_pkg_kind" ] && [ "$pkg_kind" = "$installed_pkg_kind" ]; then
      say "updating existing .$pkg_kind installation"
    fi

    # Resolve "latest" to a concrete tag when possible so every download of this
    # run is pinned to one immutable release. If tag resolution fails, keep
    # pkg_kind and discover the exact .deb/.rpm filename directly from SHA256SUMS.
    pkg_version=
    if [ -n "$pkg_kind" ] && [ "$version" = latest ]; then
      _tag=$(latest_tag) || _tag=
      if [ -n "$_tag" ]; then
        version=$_tag
        base=https://github.com/$repo/releases/download/$version
      fi
    fi
    if [ -n "$pkg_kind" ]; then
      case $arch in
        x86_64) pkg_deb_arch=amd64; pkg_rpm_arch=x86_64 ;;
        aarch64) pkg_deb_arch=arm64; pkg_rpm_arch=aarch64 ;;
      esac
      if [ "$version" != latest ]; then
        pkg_version=${version#v}
      fi
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

    # Find the package asset filename for $1 ("quack" or "quack-gui") in SHA256SUMS.
    # Uses the exact version when known, and falls back to matching the package
    # pattern in SHA256SUMS so a tag lookup failure never blocks .deb/.rpm.
    find_pkg_in_sums() {
      _pkg=$1
      if [ -n "$pkg_version" ]; then
        if [ "$pkg_kind" = deb ]; then
          _exact=${_pkg}_${pkg_version}_${pkg_deb_arch}.deb
        else
          _rpm_ver=$(printf '%s' "$pkg_version" | tr '-' '_')
          _exact=${_pkg}-${_rpm_ver}-1.${pkg_rpm_arch}.rpm
        fi
        if awk -v f="$_exact" '$2 == f || $2 == "*" f { found = 1; exit } END { exit !found }' "$tmp/SHA256SUMS"; then
          printf '%s\n' "$_exact"
          return 0
        fi
      fi
      if [ "$pkg_kind" = deb ]; then
        _pat="^${_pkg}_[0-9][^_]*_${pkg_deb_arch}[.]deb\$"
      else
        _pat="^${_pkg}-[0-9][^-]*-1[.]${pkg_rpm_arch}[.]rpm\$"
      fi
      awk -v pat="$_pat" '{
        f = $2
        sub(/^\*/, "", f)
        if (f ~ pat) { print f; found = 1; exit }
      } END { exit !found }' "$tmp/SHA256SUMS"
    }

    tui_pkg_name=
    gui_pkg_name=
    if [ -n "$pkg_kind" ]; then
      _missing_pkg=
      if [ "$want_tui" = 1 ]; then
        tui_pkg_name=$(find_pkg_in_sums quack) || _missing_pkg="quack .$pkg_kind package"
      fi
      if [ -z "$_missing_pkg" ] && [ "$want_gui" = 1 ]; then
        gui_pkg_name=$(find_pkg_in_sums quack-gui) || _missing_pkg="quack-gui .$pkg_kind package"
      fi
      if [ -n "$_missing_pkg" ]; then
        if [ -n "$pkg_forced" ]; then
          die "release $version has no $_missing_pkg for $arch in SHA256SUMS"
        elif [ -n "$installed_pkg_kind" ]; then
          die "release $version has no $_missing_pkg for $arch in SHA256SUMS (cannot update the installed .$installed_pkg_kind package; pass --pkg archive to install a separate copy under $prefix)"
        else
          # A fresh install against an older release without packages falls back to the archives.
          say "note: release $version has no $_missing_pkg for $arch; using the self-contained archive instead"
          pkg_kind=
          tui_pkg_name=
          gui_pkg_name=
        fi
      fi
      if [ -n "$pkg_kind" ] && [ -n "$tui_pkg_name" ]; then
        say "downloading $tui_pkg_name ($version)"
        fetch "$base/$tui_pkg_name" "$tmp/$tui_pkg_name" || die "download failed: $base/$tui_pkg_name"
        verify_sum "$tmp/$tui_pkg_name" "$tui_pkg_name"
        tui_pkg=$tmp/$tui_pkg_name
      fi
      if [ -n "$pkg_kind" ] && [ -n "$gui_pkg_name" ]; then
        say "downloading $gui_pkg_name ($version)"
        fetch "$base/$gui_pkg_name" "$tmp/$gui_pkg_name" || die "download failed: $base/$gui_pkg_name"
        verify_sum "$tmp/$gui_pkg_name" "$gui_pkg_name"
        gui_pkg=$tmp/$gui_pkg_name
      fi
    fi

    if [ -z "$pkg_kind" ]; then
      if [ "$want_tui" = 1 ] || [ "$binary_only" = 0 ]; then
        tui_name=quack-$arch-$tui_os.tar.gz
        if [ "$want_tui" = 1 ]; then
          say "downloading $tui_name ($version)"
        else
          say "downloading $tui_name ($version, for shared defaults)"
        fi
        fetch "$base/$tui_name" "$tmp/$tui_name" || die "download failed: $base/$tui_name"
        verify_sum "$tmp/$tui_name" "$tui_name"
        tui_archive=$tmp/$tui_name
      fi
    elif [ "$want_tui" = 0 ]; then
      # A GUI installed as a system package still needs the shared defaults, which
      # ship in the TUI archive; fetch it for those only.
      tui_name=quack-$arch-$tui_os.tar.gz
      say "downloading $tui_name ($version, for shared defaults)"
      fetch "$base/$tui_name" "$tmp/$tui_name" || die "download failed: $base/$tui_name"
      verify_sum "$tmp/$tui_name" "$tui_name"
      tui_archive=$tmp/$tui_name
    fi

    if [ "$want_gui" = 1 ] && [ -z "$pkg_kind" ]; then
      if [ "$host_os" = Linux ]; then
        gui_name=quack-gui-$arch-$gui_os.AppImage
        say "downloading $gui_name ($version)"
        fetch "$base/$gui_name" "$tmp/$gui_name" || die "download failed: $base/$gui_name"
        verify_sum "$tmp/$gui_name" "$gui_name"
        gui_appimage=$tmp/$gui_name
      else
        gui_name=quack-gui-$arch-$gui_os.dmg
        say "downloading $gui_name ($version)"
        fetch "$base/$gui_name" "$tmp/$gui_name" || die "download failed: $base/$gui_name"
        verify_sum "$tmp/$gui_name" "$gui_name"
        gui_dmg=$tmp/$gui_name
      fi
    fi
  fi

  if [ -n "$tui_archive" ]; then
    mkdir "$tmp/x"
    tar -xzf "$tui_archive" -C "$tmp/x" 2>/dev/null || die "$tui_archive is not a quack .tar.gz archive"
    top=$(find "$tmp/x" -mindepth 1 -maxdepth 1 -type d | head -n 1)
    [ -n "$top" ] && [ -f "$top/bin/quack" ] || die "$tui_archive does not look like a quack archive"
    if [ "$want_tui" = 1 ]; then new_tui_bin=$top/bin/quack; fi
    if [ -d "$top/share/quack/defaults" ]; then new_defaults=$top/share/quack/defaults; fi
  fi
  maybe_preserve_plucked "$pkg_kind"
fi

# Install the shared defaults bundle next to a system-package install (/usr/bin
# binaries find it at ../share/quack/defaults).
install_system_defaults() {
  _src=$1
  _dest=$sys_defaults
  if [ -z "$_src" ]; then
    if [ "$plucked" = 1 ]; then
      say "warning: this archive has no defaults bundle, so --plucked had nothing to filter; left $_dest as it is"
    else
      say "this archive has no defaults bundle; left $_dest as it is"
    fi
    return 0
  fi
  rm -rf "$tmp/defaults.sys"
  cp -R "$_src" "$tmp/defaults.sys"
  if [ "$plucked" = 1 ]; then
    pluck_defaults_dir "$tmp/defaults.sys"
  fi
  chmod -R a+rX "$tmp/defaults.sys" 2>/dev/null || true
  as_root mkdir -p "$(dirname "$_dest")"
  as_root rm -rf "${_dest}.new"
  as_root cp -R "$tmp/defaults.sys" "${_dest}.new"
  as_root rm -rf "$_dest"
  as_root mv "${_dest}.new" "$_dest"
  if [ "$plucked" = 1 ]; then
    say "installed plucked defaults in $_dest (hosted providers left out)"
  else
    say "installed the defaults in $_dest"
  fi
}

# The package binaries live in /usr/bin; drop an earlier non-package copy in $bindir
# (and its desktop entry) so the two cannot shadow each other on PATH.
drop_stale_local() {
  _name=$1
  if [ -e "$bindir/$_name" ]; then
    rm -f "$bindir/$_name"
    say "removed the earlier $bindir/$_name (replaced by the system package)"
  fi
  if [ "$_name" = quack-gui ]; then
    rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/applications/quack-gui.desktop"
  fi
}

if [ -n "$tui_pkg" ] || [ -n "$gui_pkg" ]; then
  # System package install: the packages carry the binaries, the shared defaults
  # bundle (/usr/share/quack/defaults) and the desktop integration themselves.
  if [ -n "$tui_pkg" ]; then
    if { [ "$pkg_kind" = deb ] && deb_installed quack-plucked; } \
      || { [ "$pkg_kind" = rpm ] && rpm_installed quack-plucked; }; then
      pkg_remove quack-plucked
      say "removed legacy quack-plucked system package before upgrading"
    fi
    if [ "$pkg_kind" = deb ]; then install_deb "$tui_pkg"; else install_rpm "$tui_pkg"; fi
    say "installed the quack package ($pkg_kind)"
    if [ "$plucked" = 1 ]; then
      pluck_defaults_dir "$sys_defaults" 1
      say "installed plucked defaults in $sys_defaults (hosted providers left out)"
    fi
    drop_stale_local quack
  fi
  if [ -n "$gui_pkg" ]; then
    if { [ "$pkg_kind" = deb ] && deb_installed quack-gui-plucked; } \
      || { [ "$pkg_kind" = rpm ] && rpm_installed quack-gui-plucked; }; then
      pkg_remove quack-gui-plucked
      say "removed legacy quack-gui-plucked system package before upgrading"
    fi
    if [ "$pkg_kind" = deb ]; then install_deb "$gui_pkg"; else install_rpm "$gui_pkg"; fi
    say "installed the quack-gui package ($pkg_kind)"
    drop_stale_local quack-gui
  fi
  if [ "$want_tui" = 0 ] && [ "$want_gui" = 1 ]; then
    install_system_defaults "$new_defaults"
  fi
else
  if [ "$want_tui" = 1 ]; then
    install_bin "$new_tui_bin" quack
  fi

  if [ "$want_tui" = 1 ] || [ "$want_gui" = 1 ]; then
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
      [ "$host_os" = Linux ] && install_linux_desktop "$source_dir/crates/quack-gui/icons/128x128.png"
    fi
  fi
fi

if [ -z "$tui_pkg$gui_pkg" ]; then
  case :$PATH: in
    *:"$bindir":*) ;;
    *) say "$bindir is not on your PATH; add it, e.g. export PATH=\"$bindir:\$PATH\"" ;;
  esac
fi

if [ -n "$tui_pkg" ] && [ -n "$gui_pkg" ]; then
  say "installed as system packages ($pkg_kind): quack and quack-gui are in /usr/bin"
elif [ -n "$tui_pkg" ]; then
  say "installed as a system package ($pkg_kind): quack is in /usr/bin"
elif [ -n "$gui_pkg" ]; then
  say "installed as a system package ($pkg_kind): quack-gui is in /usr/bin"
fi

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
