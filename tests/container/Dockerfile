FROM docker.io/library/archlinux:base-20260906.0.587075@sha256:522dd24e4a16f41afe71c7561febe519f7575404c291565e9f0be823d1794ca4 AS snapshot-base
ENV STRATA_TEST_ARCH_SNAPSHOT=2026/09/11
COPY tests/container/install-packages.sh /opt/strata-install-packages.sh

FROM snapshot-base AS package-indexes
RUN sh /opt/strata-install-packages.sh indexes

FROM package-indexes AS gtk-build
RUN sh /opt/strata-install-packages.sh install \
        base-devel glib2-devel gtk4 libsysprof-capture meson sassc shaderc \
        vulkan-headers wayland-protocols \
    && rm -rf /var/cache/pacman/pkg/*
COPY tests/container/gtk/gtk-4.22-accessible-bounds.patch /opt/gtk-4.22-accessible-bounds.patch
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
RUN sh /opt/strata-install-packages.sh install \
        at-spi2-core bubblewrap ca-certificates cantarell-fonts dbus diffutils ffmpeg \
        fontconfig gobject-introspection-runtime gst-libav gst-plugins-base \
        gst-plugins-good gtk4 gtksourceview5 imagemagick libheif librsvg poppler-glib \
        python-gobject strace xorg-server-xvfb xorg-xauth \
    && rm -rf /var/cache/pacman/pkg/* \
    && test "$(pacman -Q gtk4)" = 'gtk4 1:4.22.5-1' \
    && command -v cmp >/dev/null
COPY --from=gtk-build /opt/patched-gtk/libgtk-4.so.1.2200.5 /usr/lib/libgtk-4.so.1.2200.5
COPY tests/container/gtk/coordinate-probe.py /opt/strata-coordinate-probe.py
RUN mkdir -p /usr/share/strata-test \
    && sha256sum /usr/lib/libgtk-4.so.1.2200.5 > /usr/share/strata-test/gtk.sha256 \
    && env -u DISPLAY -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS -u AT_SPI_BUS_ADDRESS \
        GTK_A11Y=none NO_AT_BRIDGE=1 GIO_USE_VFS=local GDK_BACKEND=x11 GSK_RENDERER=cairo \
        timeout 15s dbus-run-session -- xvfb-run -a python /opt/strata-coordinate-probe.py

ARG TEST_UID=1000
ARG TEST_GID=1000
RUN (getent group "$TEST_GID" || groupadd --gid "$TEST_GID" strata-test) \
    && (getent passwd "$TEST_UID" || useradd --no-log-init --uid "$TEST_UID" \
        --gid "$TEST_GID" --home-dir /tmp/strata-build-home --no-create-home strata-test)

FROM runtime AS toolchain
RUN sh /opt/strata-install-packages.sh install base-devel glib2-devel pkgconf rustup \
    && rm -rf /var/cache/pacman/pkg/* /var/lib/pacman/sync/*
ENV RUSTUP_HOME=/opt/rustup RUSTUP_TOOLCHAIN=1.99.1
RUN rustup toolchain install 1.99.1 --no-self-update --profile minimal --component clippy,rustfmt \
    && chmod -R a+rX /opt/rustup

FROM toolchain AS dependencies
# The crates Cargo.lock names, fetched, and the third-party crates of the two QA builds,
# compiled: the application's, and strata-rune's in a cargo run of its own, as
# scripts/qa-parity.sh builds them. A run copies them into an empty Cargo cache
# (scripts/dependency_seed.py), so they are made where target/qa-container puts them: Cargo's
# fingerprints hold the path of a crate's sources.
WORKDIR /workspace
ENV CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0
COPY tests/container/stub-workspace.py /opt/strata-stub-workspace.py
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# Each workspace crate is reduced to its manifest and empty targets: no source or build
# script of Strata's runs here. What the stubs built is then removed, and the stage fails if
# any of it is left, so that a run can only compile the real crates. The Cargo home tells
# Cargo not to clean it: a base outlives the month after which Cargo deletes the crates a
# command did not use, and the first run from an older base would delete most of them.
RUN find crates -type f ! -name Cargo.toml -delete \
    && python3 /opt/strata-stub-workspace.py stub . \
    && export CARGO_HOME=/workspace/target/qa-container/cargo \
        CARGO_TARGET_DIR=/workspace/target/qa-container/build \
    && cargo fetch --locked \
    && cargo test --locked --features qa,invariant-checks --test qa --no-run \
    && cargo build --locked -p strata-rune --features qa \
    && packages="$(python3 /opt/strata-stub-workspace.py packages .)" \
    && cargo clean --locked --offline $(printf -- '-p %s ' $packages) \
    && python3 /opt/strata-stub-workspace.py leftovers . "$CARGO_TARGET_DIR" \
    && test -n "$(find "$CARGO_TARGET_DIR/debug/deps" -name '*.rlib' -print -quit)" \
    && printf '[cache]\nauto-clean-frequency = "never"\n' > "$CARGO_HOME/config.toml" \
    && mkdir /opt/strata-dependencies \
    && mv "$CARGO_HOME" "$CARGO_TARGET_DIR" /opt/strata-dependencies/ \
    && printf '%s\n' /workspace/target/qa-container > /opt/strata-dependencies/cache-path \
    && sha256sum < Cargo.lock | cut -d ' ' -f 1 > /opt/strata-dependencies/lock.sha256 \
    && chmod -R a+rX /opt/strata-dependencies \
    && cd / && rm -rf /workspace && mkdir /workspace
