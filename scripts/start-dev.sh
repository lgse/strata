#!/usr/bin/env bash
set -euo pipefail

native_dependencies_available() {
  command -v pkg-config >/dev/null 2>&1 \
    && pkg-config --exists fontconfig \
    && pkg-config --exists 'gtk4 >= 4.12' \
    && pkg-config --exists gtksourceview-5 \
    && pkg-config --exists poppler-glib
}

run_as_root() {
  if (( EUID == 0 )); then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    echo "Native development dependencies are missing, and sudo is unavailable." >&2
    return 1
  fi
}

install_native_dependencies() {
  echo "Native development dependencies are missing; installing them now..."

  if command -v pacman >/dev/null 2>&1; then
    run_as_root pacman -S --needed base-devel rust fontconfig gtk4 gtksourceview5 poppler-glib
  elif command -v apt-get >/dev/null 2>&1; then
    run_as_root apt-get update
    run_as_root apt-get install -y build-essential cargo rustc pkg-config \
      libfontconfig1-dev libgtk-4-dev libgtksourceview-5-dev libpoppler-glib-dev
  elif command -v dnf >/dev/null 2>&1; then
    run_as_root dnf install -y gcc gcc-c++ make rust cargo pkgconf-pkg-config \
      fontconfig-devel gtk4-devel gtksourceview5-devel poppler-glib-devel
  else
    echo "Unsupported package manager. Install the native dependencies listed in README.md." >&2
    return 1
  fi

  if ! native_dependencies_available; then
    echo "Native dependency installation completed, but required pkg-config libraries are still unavailable." >&2
    return 1
  fi
}

if ! command -v cargo >/dev/null 2>&1 || ! native_dependencies_available; then
  install_native_dependencies
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "Cargo is unavailable after installing development dependencies." >&2
  exit 1
fi

if ! cargo watch --version >/dev/null 2>&1; then
  echo "cargo-watch is not installed; installing it now..."
  cargo install --locked cargo-watch
fi

# cargo-watch restarts running commands by default when a change is detected.
exec cargo watch \
  --watch src \
  --watch data \
  --watch Cargo.toml \
  --watch build.rs \
  --exec run
