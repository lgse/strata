#!/usr/bin/env bash

set -Eeuo pipefail

REPOSITORY="lgse/strata"
APP_ID="io.github.lgse.Strata"
MIN_GLIBC="2.39"
REQUIRED_PACKAGES=(
  bubblewrap desktop-file-utils ffmpeg ffmpegthumbnailer fontconfig gst-libav
  gst-plugins-good gtk4 gtksourceview5 poppler-glib github-cli xdg-utils
)

info() {
  printf '\n\033[1;34m==>\033[0m %s\n' "$*"
}

warn() {
  printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2
}

die() {
  printf '\033[1;31merror:\033[0m %s\n' "$*" >&2
  exit 1
}

show_banner() {
  local reset="" bold=""
  local -a colors=("" "" "" "" "" "" "" "" "" "")

  if [[ -t 1 && ${TERM:-dumb} != dumb && -z ${NO_COLOR:-} ]]; then
    reset=$'\033[0m'
    bold=$'\033[1m'
    colors=(
      $'\033[38;2;156;203;255m'
      $'\033[38;2;145;193;255m'
      $'\033[38;2;132;181;255m'
      $'\033[38;2;122;169;255m'
      $'\033[38;2;145;157;255m'
      $'\033[38;2;157;140;255m'
      $'\033[38;2;142;128;255m'
      $'\033[38;2;124;116;255m'
      $'\033[38;2;103;104;255m'
      $'\033[38;2;102;116;255m'
    )
  fi

  printf '\n'
  printf '%b%s%b\n' "${colors[0]}" '         ▄▄██▄' "$reset"
  printf '%b%s%b         %bS T R A T A%b\n' "${colors[1]}" '      ▄████▀▀   ▄▄▄' "$reset" "$bold" "$reset"
  printf '%b%s%b       %s\n' "${colors[2]}" '   ▄████▀      ▀▀███▄' "$reset" 'Navigate every layer.'
  printf '%b%s%b\n' "${colors[3]}" '   ███    ████▄▄   ▀▀' "$reset"
  printf '%b%s%b\n' "${colors[4]}" '   ███▄▄    ▀▀███▄▄' "$reset"
  printf '%b%s%b\n' "${colors[5]}" '    ▀▀███▄▄    ▀▀████' "$reset"
  printf '%b%s%b\n' "${colors[6]}" '   ▄   ▀▀████▄    ███' "$reset"
  printf '%b%s%b\n' "${colors[7]}" '   ███▄▄   ▀▀   ▄████' "$reset"
  printf '%b%s%b         %s\n' "${colors[8]}" '    ▀▀█▀▀   ▄▄███▀▀' "$reset" 'Interactive installer'
  printf '%b%s%b\n\n' "${colors[9]}" '          ████▀▀' "$reset"
}

prompt() {
  local question=$1 default=${2:-yes} answer suffix
  if [[ $default == yes ]]; then
    suffix="[Y/n]"
  else
    suffix="[y/N]"
  fi

  printf '%s %s ' "$question" "$suffix" >"$PROMPT_DEVICE"
  IFS= read -r answer <"$PROMPT_DEVICE" || die "Could not read your answer."
  answer=${answer:-$default}
  [[ $answer == [Yy] || $answer == [Yy][Ee][Ss] ]]
}

version_at_least() {
  local actual=$1 required=$2 first
  first=$(printf '%s\n%s\n' "$required" "$actual" | sort -V | head -n 1)
  [[ $first == "$required" ]]
}

detect_target() {
  case $(uname -m) in
    x86_64 | amd64) printf '%s\n' x86_64-unknown-linux-gnu ;;
    aarch64 | arm64) printf '%s\n' aarch64-unknown-linux-gnu ;;
    *) die "Strata has no prebuilt release for $(uname -m)." ;;
  esac
}

detect_omarchy_major() {
  local output="" version_file

  if command -v omarchy >/dev/null 2>&1; then
    output=$(omarchy version 2>/dev/null || true)
  fi

  if [[ -z $output ]]; then
    for version_file in /usr/share/omarchy/version "$HOME/.local/share/omarchy/version"; do
      if [[ -r $version_file ]]; then
        output=$(<"$version_file")
        break
      fi
    done
  fi

  if [[ $output =~ ([34])([.][0-9]+)* ]]; then
    printf '%s\n' "${BASH_REMATCH[1]}"
  fi
  return 0
}

latest_stable_version() {
  local effective tag
  effective=$(curl -fsSL -o /dev/null -w '%{url_effective}' \
    "https://github.com/$REPOSITORY/releases/latest") \
    || die "Could not find the latest stable Strata release."
  tag=${effective##*/}
  [[ $tag =~ ^v([0-9]+[.][0-9]+[.][0-9]+)$ ]] \
    || die "GitHub returned an unexpected stable release tag: $tag"
  printf '%s\n' "${BASH_REMATCH[1]}"
}

install_arch_dependencies() {
  local missing=() package
  for package in "${REQUIRED_PACKAGES[@]}"; do
    pacman -Q "$package" >/dev/null 2>&1 || missing+=("$package")
  done

  if ((${#missing[@]} == 0)); then
    info "All required runtime packages are already installed."
    return
  fi

  printf 'Required packages: %s\n' "${missing[*]}"
  prompt "Install these packages with sudo pacman?" \
    || die "Required runtime packages were not installed."
  sudo pacman -S --needed -- "${missing[@]}"
}

install_desktop_entry() {
  local extracted=$1 desktop_dir icon_dir escaped_bin staged
  desktop_dir=${XDG_DATA_HOME:-$HOME/.local/share}/applications
  icon_dir=${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor
  staged=$TEMP_DIR/$APP_ID.desktop
  escaped_bin=${BIN_PATH//&/\\&}

  install -Dm644 "$extracted/$APP_ID.svg" \
    "$icon_dir/scalable/apps/$APP_ID.svg"
  sed "s|^Exec=strata |Exec=$escaped_bin |" "$extracted/$APP_ID.desktop" >"$staged"
  install -Dm644 "$staged" "$desktop_dir/$APP_ID.desktop"

  command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database "$desktop_dir" 2>/dev/null || true
  command -v gtk-update-icon-cache >/dev/null 2>&1 \
    && gtk-update-icon-cache -qtf "$icon_dir" 2>/dev/null || true
  info "Added Strata to the desktop application menu."

  if prompt "Make Strata the default application for opening folders?" no; then
    command -v xdg-mime >/dev/null 2>&1 \
      || die "xdg-mime is required to change the folder association."
    xdg-mime default "$APP_ID.desktop" inode/directory
    local current
    current=$(xdg-mime query default inode/directory)
    [[ $current == "$APP_ID.desktop" ]] \
      || die "The folder association did not change (current value: $current)."
    info "Strata is now the default application for folders."
  fi
}

configure_omarchy_bindings() {
  local major=$1 bindings backup errors
  if [[ $major == 4 ]]; then
    bindings=$HOME/.config/hypr/bindings.lua
  else
    bindings=$HOME/.config/hypr/bindings.conf
  fi

  install -d "$(dirname "$bindings")"
  if grep -q 'strata-installer: file-manager start' "$bindings" 2>/dev/null; then
    info "Omarchy file-manager keybinds already point to Strata."
    return
  fi

  backup=$bindings.bak.$(date +%Y%m%d%H%M%S)
  if [[ -e $bindings ]]; then
    cp -p "$bindings" "$backup"
  else
    : >"$bindings"
    backup=""
  fi

  if [[ $major == 4 ]]; then
    cat >>"$bindings" <<EOF

-- strata-installer: file-manager start
hl.unbind("SUPER + SHIFT + F")
hl.unbind("SUPER + ALT + SHIFT + F")
o.bind("SUPER + SHIFT + F", "File manager", { launch = "$BIN_PATH" })
o.bind("SUPER + ALT + SHIFT + F", "File manager (cwd)",
  "uwsm-app -- $BIN_PATH \"\$(omarchy-cmd-terminal-cwd)\"")
-- strata-installer: file-manager end
EOF
  else
    cat >>"$bindings" <<EOF

# strata-installer: file-manager start
unbind = SUPER SHIFT, F
unbind = SUPER ALT SHIFT, F
bindd = SUPER SHIFT, F, File manager, exec, uwsm-app -- $BIN_PATH
bindd = SUPER ALT SHIFT, F, File manager (cwd), exec, uwsm-app -- $BIN_PATH "\$(omarchy-cmd-terminal-cwd)"
# strata-installer: file-manager end
EOF
  fi

  if command -v hyprctl >/dev/null 2>&1 && [[ -n ${HYPRLAND_INSTANCE_SIGNATURE:-} ]]; then
    hyprctl reload >/dev/null
    errors=$(hyprctl configerrors 2>&1 || true)
    if [[ -n ${errors//[[:space:]]/} ]]; then
      if [[ -n $backup ]]; then
        cp -p "$backup" "$bindings"
      else
        rm -f "$bindings"
      fi
      hyprctl reload >/dev/null 2>&1 || true
      die "Hyprland rejected the keybind change; restored the previous config:"$'\n'"$errors"
    fi
  else
    warn "Hyprland is not running, so the keybind file could not be reloaded now."
  fi

  info "Omarchy $major file-manager shortcuts now open Strata."
  [[ -n $backup ]] && printf 'Backup: %s\n' "$backup"
  return 0
}

main() {
  local target glibc distro_id distro_like omarchy_major version archive extracted url
  local local_bin_on_path=no

  [[ $(uname -s) == Linux ]] || die "The prebuilt Strata release supports Linux only."
  [[ $EUID -ne 0 ]] || die "Run this installer as your normal desktop user, not as root."
  [[ -e /dev/tty && -r /dev/tty && -w /dev/tty ]] \
    || die "This interactive installer needs a terminal."
  PROMPT_DEVICE=/dev/tty
  [[ :$PATH: == *":$HOME/.local/bin:"* ]] && local_bin_on_path=yes
  show_banner

  target=$(detect_target)
  command -v getconf >/dev/null 2>&1 || die "Could not detect the system C library."
  glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')
  [[ $glibc =~ ^[0-9]+[.][0-9]+ ]] || die "Strata requires a glibc-based Linux system."
  version_at_least "$glibc" "$MIN_GLIBC" \
    || die "Strata requires glibc $MIN_GLIBC or newer (found $glibc)."

  distro_id="unknown"
  distro_like=""
  if [[ -r /etc/os-release ]]; then
    # shellcheck disable=SC1091
    source /etc/os-release
    distro_id=${ID:-unknown}
    distro_like=${ID_LIKE:-}
  fi
  omarchy_major=$(detect_omarchy_major)

  info "Detected system"
  printf 'Linux distribution: %s\nArchitecture: %s\nglibc: %s\n' \
    "$distro_id" "$target" "$glibc"
  if [[ -n $omarchy_major ]]; then
    printf 'Omarchy: major version %s\n' "$omarchy_major"
  else
    printf 'Omarchy: not detected\n'
  fi

  if [[ $distro_id == arch || $distro_like == *arch* || -n $omarchy_major ]]; then
    command -v pacman >/dev/null 2>&1 || die "This Arch-based system does not provide pacman."
    install_arch_dependencies
    if prompt "Install optional SMB network-share support (gvfs-smb)?" no; then
      sudo pacman -S --needed -- gvfs-smb
    fi
  else
    printf '\nStrata needs GTK 4.12+, GtkSourceView 5, Poppler GLib, Fontconfig, Bubblewrap,\n'
    printf 'FFmpeg, ffmpegthumbnailer, and GStreamer runtime plugins.\n'
    prompt "Have you installed the equivalent packages for this distribution?" \
      || die "Install the runtime dependencies, then run this installer again."
  fi

  for command in curl tar sha256sum gh install sed; do
    command -v "$command" >/dev/null 2>&1 || die "Required command not found: $command"
  done

  version=$(latest_stable_version)
  archive="strata-$version-$target.tar.gz"
  url="https://github.com/$REPOSITORY/releases/download/v$version"
  TEMP_DIR=$(mktemp -d)
  trap 'rm -rf -- "$TEMP_DIR"' EXIT

  info "Downloading stable Strata v$version"
  curl --fail --location --show-error --progress-bar \
    --output "$TEMP_DIR/$archive" "$url/$archive"
  curl --fail --location --show-error --progress-bar \
    --output "$TEMP_DIR/$archive.sha256" "$url/$archive.sha256"

  info "Verifying checksum and GitHub Actions provenance"
  (cd "$TEMP_DIR" && sha256sum --check "$archive.sha256")
  gh attestation verify "$TEMP_DIR/$archive" --repo "$REPOSITORY"

  tar -xzf "$TEMP_DIR/$archive" -C "$TEMP_DIR"
  extracted=$TEMP_DIR/${archive%.tar.gz}
  [[ -x $extracted/strata ]] || die "The verified archive does not contain the Strata binary."
  [[ -r $extracted/$APP_ID.desktop && -r $extracted/$APP_ID.svg ]] \
    || die "The verified archive is missing desktop integration files."

  BIN_PATH=$HOME/.local/bin/strata
  if [[ -e $BIN_PATH ]] && ! prompt "Replace the existing $BIN_PATH?" no; then
    die "Installation cancelled without replacing the existing file."
  fi
  install -Dm755 "$extracted/strata" "$BIN_PATH"
  export PATH="$HOME/.local/bin:$PATH"
  info "Installed $BIN_PATH"

  if prompt "Add Strata to your desktop application menu?"; then
    install_desktop_entry "$extracted"
  fi

  if [[ -n $omarchy_major ]] \
    && prompt "Replace Omarchy's Nautilus file-manager keybinds with Strata?" no; then
    configure_omarchy_bindings "$omarchy_major"
  fi

  info "Installation complete"
  printf 'Installed Strata v%s from the stable release.\n' "$version"
  if [[ -r $extracted/SOURCE_COMMIT ]]; then
    printf 'Source commit: %s\n' "$(<"$extracted/SOURCE_COMMIT")"
  fi
  printf 'Run Strata with: %s\n' "$BIN_PATH"
  if [[ $local_bin_on_path == no ]]; then
    warn "$HOME/.local/bin is not on PATH in this shell."
  fi
}

if [[ ${STRATA_INSTALLER_TESTING:-0} != 1 ]]; then
  main "$@"
fi
