#!/usr/bin/env bash

set -Eeuo pipefail

REPOSITORY="lgse/strata"
APP_ID="io.github.lgse.Strata"
MIN_GLIBC="2.39"
REQUIRED_PACKAGES=(
  bubblewrap desktop-file-utils ffmpeg ffmpegthumbnailer fontconfig gst-libav gstreamer
  gst-plugins-base gst-plugins-good gtk4 gtksourceview5 gvfs poppler-glib python xdg-utils
)
RAW_PREVIEW_PACKAGES=(imagemagick libraw dcraw)

NON_INTERACTIVE=no
WITH_SMB=ask
WITH_RAW=ask
WITH_DESKTOP_ENTRY=ask
WITH_FOLDER_ASSOCIATION=ask
WITH_FILE_MANAGER=ask
WITH_FILE_CHOOSER=ask
WITH_OMARCHY_KEYBINDS=ask

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

verify_provenance() {
  local archive=$1

  if ! command -v gh >/dev/null 2>&1; then
    warn "GitHub CLI is unavailable; continuing after HTTPS download and checksum verification."
    return
  fi
  if ! gh auth status --hostname github.com >/dev/null 2>&1; then
    warn "GitHub CLI is not authenticated; continuing after HTTPS download and checksum verification."
    return
  fi

  info "Verifying GitHub Actions provenance"
  gh attestation verify "$archive" --repo "$REPOSITORY"
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

usage() {
  cat <<'EOF'
Usage: install.sh [options]

Without options, Strata asks about dependencies and desktop integration.

Options:
  --non-interactive             Never prompt; install required components only
  --with-smb                    Install SMB network-share support
  --with-raw                    Install broader image and camera RAW support
  --with-desktop-entry          Add Strata to the desktop application menu
  --with-folder-association     Make Strata the default folder handler
  --with-file-manager           Handle "Open file location" requests
  --with-file-chooser           Use Strata for portal Open and Save dialogs
  --without-file-chooser        Keep the current chooser; suppress the app offer
  --with-omarchy-keybinds       Replace Omarchy's file-manager keybinds
  -h, --help                    Show this help

Integration flags imply --non-interactive. Folder association also installs the
required desktop entry and enables "Open file location" integration.
EOF
}

parse_args() {
  while (($# > 0)); do
    case $1 in
      --non-interactive) NON_INTERACTIVE=yes ;;
      --with-smb) NON_INTERACTIVE=yes; WITH_SMB=yes ;;
      --with-raw) NON_INTERACTIVE=yes; WITH_RAW=yes ;;
      --with-desktop-entry) NON_INTERACTIVE=yes; WITH_DESKTOP_ENTRY=yes ;;
      --with-folder-association)
        NON_INTERACTIVE=yes
        WITH_DESKTOP_ENTRY=yes
        WITH_FOLDER_ASSOCIATION=yes
        WITH_FILE_MANAGER=yes
        ;;
      --with-file-manager) NON_INTERACTIVE=yes; WITH_FILE_MANAGER=yes ;;
      --with-file-chooser) NON_INTERACTIVE=yes; WITH_FILE_CHOOSER=yes ;;
      --without-file-chooser) NON_INTERACTIVE=yes; WITH_FILE_CHOOSER=no ;;
      --with-omarchy-keybinds) NON_INTERACTIVE=yes; WITH_OMARCHY_KEYBINDS=yes ;;
      -h | --help) usage; exit 0 ;;
      *) die "Unknown option: $1 (run with --help for usage)." ;;
    esac
    shift
  done
}

want_option() {
  local selection=$1 question=$2 default=${3:-no}
  case $selection in
    yes) return 0 ;;
    no) return 1 ;;
    ask)
      [[ $NON_INTERACTIVE == no ]] && prompt "$question" "$default"
      ;;
    *) die "Invalid installer option state: $selection" ;;
  esac
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

omarchy_major_from() {
  if [[ $1 =~ (^|[^0-9.])([34])[.][0-9]+ ]]; then
    printf '%s\n' "${BASH_REMATCH[2]}"
    return 0
  fi
  return 1
}

detect_omarchy_major() {
  local output="" version_file

  if command -v omarchy >/dev/null 2>&1; then
    output=$(omarchy version 2>/dev/null || true)
    if omarchy_major_from "$output"; then
      return 0
    fi
  fi

  for version_file in /usr/share/omarchy/version "$HOME/.local/share/omarchy/version"; do
    if [[ -r $version_file ]] && omarchy_major_from "$(<"$version_file")"; then
      return 0
    fi
  done

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

run_pacman() {
  if [[ $NON_INTERACTIVE == yes ]]; then
    sudo -n pacman -S --needed --noconfirm -- "$@" \
      || die "Non-interactive package installation failed; passwordless sudo or cached credentials may be required."
  else
    sudo pacman -S --needed -- "$@" </dev/tty
  fi
}

install_arch_dependencies() {
  local missing=() package
  for package in "${REQUIRED_PACKAGES[@]}"; do
    [[ $package == github-cli ]] && command -v gh >/dev/null 2>&1 && continue
    pacman -Q "$package" >/dev/null 2>&1 || missing+=("$package")
  done

  if ((${#missing[@]} == 0)); then
    info "All required runtime packages are already installed."
    return
  fi

  printf 'Required packages: %s\n' "${missing[*]}"
  if [[ $NON_INTERACTIVE == no ]]; then
    prompt "Install these packages with sudo pacman?" \
      || die "Required runtime packages were not installed."
  fi
  run_pacman "${missing[@]}"
}

install_optional_arch_packages() {
  local description=$1 package missing=()
  shift
  for package in "$@"; do
    pacman -Q "$package" >/dev/null 2>&1 || missing+=("$package")
  done
  if ((${#missing[@]} == 0)); then
    info "$description is already installed."
  else
    printf '%s packages: %s\n' "$description" "${missing[*]}"
    run_pacman "${missing[@]}"
  fi
}

install_desktop_entry() {
  local extracted=$1 make_default=$2 desktop_dir icon_dir escaped_bin staged
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

  if [[ $make_default == yes ]]; then
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

install_file_manager_service() {
  local extracted=$1 service_dir target conflict escaped_bin staged
  service_dir=${XDG_DATA_HOME:-$HOME/.local/share}/dbus-1/services
  target=$service_dir/$APP_ID.FileManager1.service

  [[ -r $extracted/$APP_ID.FileManager1.service ]] \
    || die "The verified archive is missing FileManager1 integration."
  install -d "$service_dir"
  conflict=$(grep -l '^Name=org\.freedesktop\.FileManager1$' \
    "$service_dir"/*.service 2>/dev/null \
    | grep -v "/$APP_ID.FileManager1.service$" | head -n 1 || true)
  [[ -z $conflict ]] \
    || die "Another per-user FileManager1 provider is already installed: $conflict"

  staged=$TEMP_DIR/$APP_ID.FileManager1.service
  escaped_bin=${BIN_PATH//&/\\&}
  sed "s|^Exec=/usr/bin/strata |Exec=$escaped_bin |" \
    "$extracted/$APP_ID.FileManager1.service" >"$staged"
  install -Dm644 "$staged" "$target"
  info 'Strata now handles "Open file location" requests.'
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

configure_file_chooser() {
  local extracted=$1 arch_based=${2:-no}
  if [[ ! -r $extracted/portal/strata.portal ]]; then
    [[ $WITH_FILE_CHOOSER != yes ]] \
      || die "This release does not include file chooser integration. Install a newer release."
    return 0
  fi
  if want_option "$WITH_FILE_CHOOSER" \
    'Replace your current Open/Save chooser with Strata in portal-aware apps? (Restarts the portal service; close open file dialogs first.)' no; then
    if [[ $arch_based == yes ]]; then
      install_optional_arch_packages "File chooser integration" xdg-desktop-portal
    fi
    "$BIN_PATH" --install-portal \
      || die "File chooser setup failed. Strata is installed; retry in Settings → General → System file chooser."
  elif [[ $NON_INTERACTIVE == no || $WITH_FILE_CHOOSER == no ]]; then
    "$BIN_PATH" --dismiss-portal-prompt \
      || warn "Could not save your choice. Strata may offer file chooser integration on first launch."
  fi
}

private_install_tempdir() {
  python3 -I - <<'PY'
import os, pathlib, stat, tempfile
root = pathlib.Path(tempfile.gettempdir()).resolve(strict=True)
for path in (root, *root.parents):
    info = path.stat()
    if info.st_uid not in (0, os.geteuid()) or (info.st_mode & 0o022 and not info.st_mode & stat.S_ISVTX):
        raise SystemExit("Unsafe temporary directory; use private user-owned storage")
print(tempfile.mkdtemp(prefix="strata-install-", dir=root))
PY
}

install_bundle() {
  python3 -I - "$1" "$2" "$3" "$4" <<'PY'
import fcntl, gzip, hashlib, io, json, os, pathlib, re, shutil, stat, struct, sys, tarfile, tempfile, zlib
archive_path, version, target, bin_path = sys.argv[1:]
archive_path, launcher = pathlib.Path(archive_path), pathlib.Path(bin_path)
expected_top = f"strata-{version}-{target}"
limit = 512 * 1024 * 1024
# Keep in sync with the updater's audited published-release compatibility list.
legacy_tags = set("""v0.2.0 v0.3.0 v0.4.0 v0.5.0 v0.6.0 v0.6.1 v0.7.0
v0.7.1-rc.1 v0.7.1-rc.2 v0.8.0 v0.8.1 v0.9.0 v0.9.1 v0.10.0
v0.11.0 v0.11.1 v0.11.2 v0.12.0 v0.12.1-nightly.20260907
v0.12.1-rc.1 v0.12.1-rc.2 v0.13.0 v0.14.0 v0.14.0-rc.1
v0.14.0-rc.2 v0.14.1-rc.1 v0.15.0 v0.16.0 v0.16.0-rc.1
v0.17.0-nightly.20260912""".split())

def check_ancestors(path):
    path = path.resolve(strict=True)
    for ancestor in (path, *path.parents):
        info = ancestor.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid not in (0, os.geteuid()) or (info.st_mode & 0o022 and not info.st_mode & stat.S_ISVTX):
            raise ValueError("Unsafe writable installation ancestor")
    return path

def check_dir(path):
    check_ancestors(path)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o022:
        raise ValueError("Installation storage is not a private user-owned directory")

def sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)

def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

def unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("Duplicate bundle manifest field")
        result[key] = value
    return result

def pointer(root, name, target, committed=False):
    with tempfile.TemporaryDirectory(prefix=".activation-", dir=root) as temporary:
        path = pathlib.Path(temporary) / name
        path.symlink_to(target)
        sync_dir(temporary)
        os.replace(path, root / name)
    try:
        sync_dir(root)
    except OSError:
        if not committed:
            raise
        print("Bundle activated; directory durability could not be confirmed. Keep the previous bundle.", file=sys.stderr)

def elf(path):
    with path.open("rb") as stream:
        header = stream.read(64)
    machine = {"x86_64-unknown-linux-gnu": 62, "aarch64-unknown-linux-gnu": 183}[target]
    if len(header) != 64 or header[:7] != b"\x7fELF\x02\x01\x01":
        raise ValueError("Invalid executable ELF")
    kind, arch, elf_version = struct.unpack_from("<HHI", header, 16)
    offset = struct.unpack_from("<Q", header, 32)[0]
    size, count = struct.unpack_from("<HH", header, 54)
    if kind not in (2, 3) or arch != machine or elf_version != 1 or size != 56 or not 0 < count <= 1024 or offset + size * count > path.stat().st_size:
        raise ValueError("Wrong architecture or corrupt executable")

try:
    if not launcher.is_absolute() or ".." in launcher.parts:
        raise ValueError("Installation path must be absolute without parent traversal")
    existing = next(path for path in (launcher.parent, *launcher.parents) if path.exists())
    launcher = check_ancestors(existing) / launcher.relative_to(existing)
    launcher.parent.mkdir(parents=True, exist_ok=True)
    check_dir(launcher.parent)
    launcher = launcher.parent.resolve(strict=True) / launcher.name
    root = launcher.parent / ".strata-bundles"
    root.mkdir(mode=0o700, exist_ok=True)
    check_dir(root)
    lock = os.open(root / "install.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
    info = os.fstat(lock)
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o022:
        raise ValueError("Invalid bundle installation lock")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    versions = root / "versions"
    versions.mkdir(mode=0o700, exist_ok=True)
    check_dir(versions)
    archive_info = archive_path.lstat()
    if not stat.S_ISREG(archive_info.st_mode) or archive_info.st_size > limit:
        raise ValueError("Archive is not a bounded regular file")
    identity = digest(archive_path)
    destination = versions / identity
    decoder = zlib.decompressobj(16 + zlib.MAX_WBITS)
    raw = bytearray()
    with archive_path.open("rb") as stream:
        while data := stream.read(65536):
            raw.extend(decoder.decompress(data, limit + 1 - len(raw)))
            if len(raw) > limit or decoder.unconsumed_tail or decoder.unused_data:
                raise ValueError("Oversized archive or trailing compressed data")
    if not decoder.eof:
        raise ValueError("Truncated archive")
    with tempfile.TemporaryDirectory(prefix=".staging-", dir=versions) as temporary:
        staging = pathlib.Path(temporary)
        names, files = set(), {}
        with tarfile.open(fileobj=io.BytesIO(raw), mode="r:") as archive:
            for member in archive:
                name = pathlib.PurePosixPath(member.name)
                if name.is_absolute() or ".." in name.parts or not name.parts or len(name.parts) > 16 or len(member.name.encode()) > 1024 or name.parts[0] != expected_top or name in names or len(names) >= 192:
                    raise ValueError("Unsafe or duplicate archive path")
                names.add(name)
                relative = pathlib.Path(*name.parts[1:])
                output = staging / relative
                if member.isdir():
                    output.mkdir(parents=True, exist_ok=True)
                    continue
                if not member.isfile() or member.size > 256 * 1024 * 1024 or relative == pathlib.Path("."):
                    raise ValueError("Archive contains a link or non-regular/oversized file")
                output.parent.mkdir(parents=True, exist_ok=True)
                with output.open("xb") as stream:
                    shutil.copyfileobj(archive.extractfile(member), stream)
                files[relative.as_posix()] = digest(output)
            if any(raw[archive.offset:]):
                raise ValueError("Trailing archive payload")
        manifest_path = staging / "bundle.json"
        if manifest_path.exists():
            if manifest_path.stat().st_size > 1024 * 1024:
                raise ValueError("Oversized bundle manifest")
            manifest = json.loads(manifest_path.read_text(), object_pairs_hook=unique_pairs)
            expected_fields = {"format", "release_tag", "target", "source_commit", "media_protocol", "files"}
            if set(manifest) != expected_fields or manifest["format"] != 1 or manifest["media_protocol"] != 1 or manifest["release_tag"] != f"v{version}" or manifest["target"] != target or not re.fullmatch("[a-fA-F0-9]{40}", manifest["source_commit"]):
                raise ValueError("Bundle manifest identity mismatch")
            files.pop("bundle.json")
            if files != manifest["files"] or not {"strata", "strata-media-helper"} <= files.keys():
                raise ValueError("Incomplete or corrupt media bundle")
            binaries = ("strata", "strata-media-helper")
        else:
            # Preserve support for immutable, already-published single-binary releases.
            if f"v{version}" not in legacy_tags or "strata-media-helper" in files:
                raise ValueError("Media bundle manifest is missing; only audited published releases may contain one binary")
            binaries = ("strata",)
        for name in binaries:
            elf(staging / name)
            (staging / name).chmod(0o755)
        for path in staging.rglob("*"):
            if path.is_file():
                with path.open("rb") as stream:
                    os.fsync(stream.fileno())
        for path in sorted((p for p in staging.rglob("*") if p.is_dir()), reverse=True):
            sync_dir(path)
        sync_dir(staging)
        if destination.exists() or destination.is_symlink():
            check_dir(destination)
            stored = set()
            for path in destination.rglob("*"):
                info = path.lstat()
                if info.st_uid != os.geteuid() or info.st_mode & 0o022 or not (stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode)):
                    raise ValueError("Unsafe stored immutable version")
                stored.add(path.relative_to(destination))
                if len(stored) > 192:
                    raise ValueError("Too many stored bundle entries")
            if stored != {p.relative_to(staging) for p in staging.rglob("*")}:
                raise ValueError("Stored immutable version has different contents")
            for path in staging.rglob("*"):
                if path.is_file() and digest(path) != digest(destination / path.relative_to(staging)):
                    raise ValueError("Stored immutable version was modified")
        else:
            if len([p for p in versions.iterdir() if not p.name.startswith(".")]) >= 8:
                raise ValueError("Eight retained bundles: close Strata and remove unused versions, preserving current and previous")
            os.rename(staging, destination)
            sync_dir(versions)
    expected_launcher = pathlib.Path(".strata-bundles/current/strata")
    if launcher.is_symlink() and launcher.readlink() != expected_launcher:
        raise ValueError("Unexpected existing launcher symlink; nothing activated")
    if launcher.exists() and not launcher.is_symlink():
        info = launcher.lstat()
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o022:
            raise ValueError("Unsafe existing launcher")
        legacy = versions / ("legacy-" + digest(launcher))
        if not legacy.exists():
            if len([p for p in versions.iterdir() if not p.name.startswith(".")]) >= 8:
                raise ValueError("No retention slot for the legacy backup; close Strata and remove unused versions")
            with tempfile.TemporaryDirectory(prefix=".legacy-", dir=versions) as temporary:
                temporary = pathlib.Path(temporary)
                shutil.copy2(launcher, temporary / "strata")
                with (temporary / "strata").open("rb") as stream:
                    os.fsync(stream.fileno())
                sync_dir(temporary)
                os.rename(temporary, legacy)
                sync_dir(versions)
        check_dir(legacy)
        info = (legacy / "strata").lstat()
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o022 or digest(legacy / "strata") != legacy.name.removeprefix("legacy-"):
            raise ValueError("Stored legacy backup is damaged")
        if not (root / "current").is_symlink():
            pointer(root, "current", pathlib.Path("versions") / legacy.name)
    if (root / "current").is_symlink():
        old = (root / "current").readlink()
        if len(old.parts) != 2 or old.parts[0] != "versions" or not re.fullmatch("[a-zA-Z0-9-]+", old.parts[1]):
            raise ValueError("Unsafe current bundle pointer")
        if old != pathlib.Path("versions") / identity:
            pointer(root, "previous", old)
    pointer(root, "current", pathlib.Path("versions") / identity, committed=launcher.is_symlink())
    if not launcher.is_symlink():
        pointer(launcher.parent, launcher.name, expected_launcher, committed=True)
    print(destination)
except (OSError, ValueError, KeyError, TypeError, tarfile.TarError, zlib.error) as error:
    print(f"Bundle installation failed: {error}", file=sys.stderr)
    sys.exit(1)
PY
}

main() {
  local target glibc distro_id distro_like omarchy_major version archive extracted url
  local local_bin_on_path=no make_default=no arch_based=no original_path=$PATH
  export PATH=/usr/bin:/bin

  parse_args "$@"
  [[ $(uname -s) == Linux ]] || die "The prebuilt Strata release supports Linux only."
  [[ $EUID -ne 0 ]] || die "Run this installer as your normal desktop user, not as root."
  if [[ $NON_INTERACTIVE == no ]]; then
    [[ -e /dev/tty && -r /dev/tty && -w /dev/tty ]] \
      || die "This interactive installer needs a terminal."
    PROMPT_DEVICE=/dev/tty
  fi
  [[ :$original_path: == *":$HOME/.local/bin:"* ]] && local_bin_on_path=yes
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
    arch_based=yes
    install_arch_dependencies
    if want_option "$WITH_SMB" "Install optional SMB network-share support (gvfs-smb)?"; then
      install_optional_arch_packages "SMB support" gvfs-smb
    fi
    if want_option "$WITH_RAW" \
      "Install optional broader image and camera RAW support (imagemagick, libraw, dcraw)?"; then
      install_optional_arch_packages "Image and camera RAW support" \
        "${RAW_PREVIEW_PACKAGES[@]}"
    fi
  else
    printf '\nStrata needs GTK 4.12+, GtkSourceView 5, Poppler GLib, Fontconfig, Bubblewrap,\n'
    printf 'FFmpeg, ffmpegthumbnailer, GStreamer plugins, and the GVfs UDisks2 volume monitor.\n'
    printf 'For the volume monitor, install gvfs-daemons on Debian/Ubuntu or gvfs on Fedora.\n'
    if [[ $NON_INTERACTIVE == yes ]]; then
      die "Non-interactive dependency installation currently supports Arch-based systems only."
    fi
    prompt "Have you installed the equivalent packages for this distribution?" \
      || die "Install the runtime dependencies, then run this installer again."
  fi

  for command in curl python3 sha256sum install sed; do
    command -v "$command" >/dev/null 2>&1 || die "Required command not found: $command"
  done

  version=$(latest_stable_version)
  archive="strata-$version-$target.tar.gz"
  url="https://github.com/$REPOSITORY/releases/download/v$version"
  TEMP_DIR=$(private_install_tempdir)
  trap 'rm -rf -- "$TEMP_DIR"' EXIT

  info "Downloading stable Strata v$version"
  curl --fail --location --proto '=https' --proto-redir '=https' --max-time 120 \
    --max-filesize 536870912 --show-error --progress-bar --output "$TEMP_DIR/$archive" "$url/$archive"
  curl --fail --location --proto '=https' --proto-redir '=https' --max-time 120 \
    --max-filesize 65536 --show-error --progress-bar --output "$TEMP_DIR/$archive.sha256" "$url/$archive.sha256"

  info "Verifying checksum"
  (cd "$TEMP_DIR" && sha256sum --check "$archive.sha256")
  verify_provenance "$TEMP_DIR/$archive"

  BIN_PATH=${STRATA_INSTALL_DIR:-$HOME/.local/bin}/strata
  if command -v pacman >/dev/null 2>&1 && pacman --query --owns --quiet -- "$BIN_PATH" >/dev/null 2>&1; then
    die "This installation belongs to a package manager. Use its update command instead."
  fi
  if [[ -e $BIN_PATH ]]; then
    if [[ $NON_INTERACTIVE == yes ]]; then
      die "$BIN_PATH already exists; remove it or run the interactive installer to replace it."
    fi
    prompt "Replace the existing $BIN_PATH?" no \
      || die "Installation cancelled without replacing the existing file."
  fi
  extracted=$(install_bundle "$TEMP_DIR/$archive" "$version" "$target" "$BIN_PATH")
  [[ -x $extracted/strata ]] || die "The installed bundle has no Strata executable."
  info "Installed $BIN_PATH"

  if want_option "$WITH_DESKTOP_ENTRY" "Add Strata to your desktop application menu?" yes; then
    if want_option "$WITH_FOLDER_ASSOCIATION" \
      "Make Strata the default application for opening folders?"; then
      make_default=yes
      WITH_FILE_MANAGER=yes
    fi
    install_desktop_entry "$extracted" "$make_default"
  fi

  if want_option "$WITH_FILE_MANAGER" \
    'Use Strata for "Open file location" from other applications?'; then
    install_file_manager_service "$extracted"
  fi

  configure_file_chooser "$extracted" "$arch_based"

  if want_option "$WITH_OMARCHY_KEYBINDS" \
    "Replace Omarchy's Nautilus file-manager keybinds with Strata?"; then
    [[ -n $omarchy_major ]] \
      || die "--with-omarchy-keybinds requires Omarchy 3 or 4."
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
