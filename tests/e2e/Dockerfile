ARG E2E_DEPENDENCIES_IMAGE=dependencies
ARG E2E_RUNTIME_IMAGE=runtime

FROM docker.io/library/archlinux:base@sha256:374011da98ed9bef569a91a00b6466a071414049c10054ae27e62a75615b7e8e AS runtime
ENV STRATA_E2E_ARCH_SNAPSHOT=2026/09/19
RUN printf 'Server = https://archive.archlinux.org/repos/%s/$repo/os/$arch\n' "$STRATA_E2E_ARCH_SNAPSHOT" > /etc/pacman.d/mirrorlist \
    && pacman -Syu --noconfirm --needed \
        at-spi2-core bubblewrap ca-certificates dbus ffmpeg cantarell-fonts \
        gobject-introspection-runtime gst-libav gst-plugins-base gst-plugins-good \
        imagemagick gtk4 gtksourceview5 poppler librsvg python-gobject python-pip \
        xorg-xauth xorg-server-xvfb \
    && pacman -Scc --noconfirm

# Upstream stable releases awaiting promotion from Arch's GNOME staging repository.
# Do not enable that repository: select only these signed, checksum-pinned packages.
RUN mkdir /opt/gtk-packages && cd /opt/gtk-packages \
    && for package in gtk4-1:4.24.0-1-x86_64 glib2-2.90.0-1-x86_64 glib2-devel-2.90.0-1-x86_64; do \
        curl -fSL "https://archive.archlinux.org/repos/$STRATA_E2E_ARCH_SNAPSHOT/gnome-unstable/os/x86_64/$package.pkg.tar.zst" -o "$package.pkg.tar.zst"; \
        curl -fSL "https://archive.archlinux.org/repos/$STRATA_E2E_ARCH_SNAPSHOT/gnome-unstable/os/x86_64/$package.pkg.tar.zst.sig" -o "$package.pkg.tar.zst.sig"; \
    done \
    && printf '%s\n' \
        'd24e12e302286a5c60083c6655663155c6bbfe237b17d8de2b11933c5a54ded7  gtk4-1:4.24.0-1-x86_64.pkg.tar.zst' \
        'e4b03addffe9c89b095a1c34e64a6df5e894281e1eb1f15a2d0a8c5294637a00  glib2-2.90.0-1-x86_64.pkg.tar.zst' \
        'f368a7b2c5a4864e436fde1c23b9292d6bade054c24dd8a39a04408b3aef47fa  glib2-devel-2.90.0-1-x86_64.pkg.tar.zst' | sha256sum -c - \
    && pacman -U --noconfirm gtk4-1:4.24.0-1-x86_64.pkg.tar.zst glib2-2.90.0-1-x86_64.pkg.tar.zst

RUN pacman -S --noconfirm --needed poppler-glib && pacman -Scc --noconfirm

COPY tests/e2e/requirements.txt /opt/e2e-requirements.txt
RUN python -m venv --system-site-packages /opt/e2e-venv \
    && /opt/e2e-venv/bin/pip install --no-cache-dir -r /opt/e2e-requirements.txt \
    && cp /opt/e2e-requirements.txt /opt/e2e-venv/strata-requirements.txt
ARG E2E_UID=1000
ARG E2E_GID=1000
RUN (getent group "$E2E_GID" || groupadd --gid "$E2E_GID" strata-e2e) \
    && (getent passwd "$E2E_UID" || useradd --no-log-init --uid "$E2E_UID" \
        --gid "$E2E_GID" --home-dir /tmp/strata-build-home --no-create-home strata-e2e)
ENV STRATA_E2E_VENV=/opt/e2e-venv

FROM runtime AS toolchain
RUN pacman -S --noconfirm --needed base-devel pkgconf rustup glib2-devel \
    && pacman -U --noconfirm /opt/gtk-packages/glib2-devel-2.90.0-1-x86_64.pkg.tar.zst \
    && pacman -Scc --noconfirm
ENV RUSTUP_HOME=/opt/rustup RUSTUP_TOOLCHAIN=1.98.1
RUN rustup toolchain install 1.98.1 --no-self-update --profile minimal --component clippy,rustfmt \
    && chmod -R a+rX /opt/rustup

FROM toolchain AS dependencies
WORKDIR /build
ENV CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && printf 'fn main() {}\n' > src/main.rs \
    && printf 'fn main() {}\n' > build.rs \
    && cargo build --locked --bin strata \
    && rm -rf src build.rs target/debug/strata target/debug/deps/strata* \
        target/debug/.fingerprint/strata-* target/debug/build/strata-*

FROM ${E2E_RUNTIME_IMAGE} AS ci-runtime

FROM ${E2E_DEPENDENCIES_IMAGE} AS application
COPY Cargo.toml Cargo.lock /tmp/strata-e2e-manifests/
RUN cmp Cargo.toml /tmp/strata-e2e-manifests/Cargo.toml \
    && cmp Cargo.lock /tmp/strata-e2e-manifests/Cargo.lock
COPY src ./src
COPY data ./data
COPY build.rs ./
ARG STRATA_BUILD_COMMIT
RUN cargo build --locked --bin strata
COPY tests/e2e ./tests/e2e
COPY scripts/e2e_bundle.py ./scripts/e2e_bundle.py
RUN /opt/e2e-venv/bin/python -m pytest -c tests/e2e/pytest.ini --rootdir /build \
        --collect-only -q -p no:cacheprovider --e2e-write-plan=/bundle/plan.json \
    && cp target/debug/strata /bundle/strata \
    && python3 scripts/e2e_bundle.py create /bundle --commit "$STRATA_BUILD_COMMIT"

FROM scratch AS bundle
COPY --from=application /bundle/ /
