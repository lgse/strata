ARG E2E_DEPENDENCIES_IMAGE=dependencies
ARG E2E_RUNTIME_IMAGE=runtime

FROM docker.io/library/archlinux:base-20260906.0.587075@sha256:522dd24e4a16f41afe71c7561febe519f7575404c291565e9f0be823d1794ca4 AS snapshot-base
ENV STRATA_E2E_ARCH_SNAPSHOT=2026/09/11
COPY tests/e2e/install-packages.sh /opt/e2e-install-packages.sh

FROM snapshot-base AS package-indexes
RUN sh /opt/e2e-install-packages.sh indexes

FROM package-indexes AS gtk-build
RUN sh /opt/e2e-install-packages.sh install \
        base-devel glib2-devel gtk4 libsysprof-capture meson sassc shaderc \
        vulkan-headers wayland-protocols \
    && rm -rf /var/cache/pacman/pkg/*
COPY tests/e2e/gtk/gtk-4.22-accessible-bounds.patch /opt/gtk-4.22-accessible-bounds.patch
RUN mkdir /build-gtk && cd /build-gtk \
    && curl --fail --location --output gtk-4.22.5.tar.xz \
        https://download.gnome.org/sources/gtk/4.22/gtk-4.22.5.tar.xz \
    && printf '%s  %s\n' \
        7fd725deb2cb3f8dc218ad862c5056ff8548f49d3b0e4081796e444c22d19686 \
        gtk-4.22.5.tar.xz | sha256sum --check --strict - \
    && tar -xf gtk-4.22.5.tar.xz \
    && patch --batch --fuzz=0 -d gtk-4.22.5 -p1 < /opt/gtk-4.22-accessible-bounds.patch \
    && CFLAGS="${CFLAGS:-} -DG_DISABLE_CAST_CHECKS" arch-meson gtk-4.22.5 build \
        -Dbroadway-backend=true -Dbuild-demos=false -Dbuild-examples=false \
        -Dbuild-tests=false -Dbuild-testsuite=false -Dcloudproviders=enabled \
        -Dcolord=enabled -Ddocumentation=false -Dintrospection=disabled \
        -Dman-pages=false -Dmedia-gstreamer=enabled -Dprint-cups=enabled \
        -Dsysprof=enabled -Dtracker=enabled -Dvulkan=enabled \
        -Dwayland-backend=true -Dx11-backend=true \
    && meson compile -C build gtk-4 \
    && install -Dm755 build/gtk/libgtk-4.so.1.2200.5 /opt/patched-gtk/libgtk-4.so.1.2200.5

FROM package-indexes AS runtime
RUN sh /opt/e2e-install-packages.sh install \
        at-spi2-core bubblewrap ca-certificates cantarell-fonts dbus ffmpeg \
        fontconfig gobject-introspection-runtime gst-libav gst-plugins-base \
        gst-plugins-good gtk4 gtksourceview5 imagemagick librsvg poppler-glib \
        python-gobject python-pip xorg-server-xvfb xorg-xauth \
    && rm -rf /var/cache/pacman/pkg/* \
    && test "$(pacman -Q gtk4)" = 'gtk4 1:4.22.5-1'
COPY --from=gtk-build /opt/patched-gtk/libgtk-4.so.1.2200.5 /usr/lib/libgtk-4.so.1.2200.5
COPY tests/e2e/gtk/coordinate-probe.py /opt/strata-e2e-coordinate-probe.py
RUN mkdir -p /usr/share/strata-e2e \
    && sha256sum /usr/lib/libgtk-4.so.1.2200.5 > /usr/share/strata-e2e/gtk.sha256 \
    && env -u DISPLAY -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS -u AT_SPI_BUS_ADDRESS \
        GTK_A11Y=none NO_AT_BRIDGE=1 GIO_USE_VFS=local GDK_BACKEND=x11 GSK_RENDERER=cairo \
        timeout 15s dbus-run-session -- xvfb-run -a python /opt/strata-e2e-coordinate-probe.py

COPY tests/e2e/requirements.txt /opt/e2e-requirements.txt
RUN python -m venv --system-site-packages /opt/e2e-venv \
    && /opt/e2e-venv/bin/pip install --no-cache-dir -r /opt/e2e-requirements.txt \
    && cp /opt/e2e-requirements.txt /opt/e2e-venv/strata-requirements.txt

ARG E2E_UID=1000
ARG E2E_GID=1000
RUN (getent group "$E2E_GID" || groupadd --gid "$E2E_GID" strata-e2e) \
    && (getent passwd "$E2E_UID" || useradd --no-log-init --uid "$E2E_UID" \
        --gid "$E2E_GID" --home-dir /tmp/strata-build-home --no-create-home strata-e2e)

ENV STRATA_E2E_GTK_VERSION=4.22.5 STRATA_E2E_VENV=/opt/e2e-venv
FROM runtime AS toolchain
RUN sh /opt/e2e-install-packages.sh install base-devel glib2-devel pkgconf rustup \
    && rm -rf /var/cache/pacman/pkg/* /var/lib/pacman/sync/*
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
