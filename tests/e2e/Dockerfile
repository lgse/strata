ARG E2E_DEPENDENCIES_IMAGE=dependencies
ARG E2E_RUNTIME_IMAGE=runtime

# noble-20260810 index: retains the original amd64 image and adds native arm64.
FROM docker.io/library/ubuntu:24.04@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517 AS certificates

RUN apt-get update && apt-get install --yes --no-install-recommends ca-certificates=20260601~24.04.1

FROM docker.io/library/ubuntu:24.04@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517 AS snapshot-base
COPY --from=certificates /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
ENV STRATA_E2E_SNAPSHOT_URL=http://snapshot.ubuntu.com/ubuntu/20260901T000000Z

RUN printf '%s\n' \
    "deb [check-valid-until=no] $STRATA_E2E_SNAPSHOT_URL noble main universe" \
    "deb [check-valid-until=no] $STRATA_E2E_SNAPSHOT_URL noble-updates main universe" \
    "deb [check-valid-until=no] $STRATA_E2E_SNAPSHOT_URL noble-security main universe" \
    > /etc/apt/sources.list \
    && rm /etc/apt/sources.list.d/ubuntu.sources

FROM snapshot-base AS package-indexes
RUN apt-get update --error-on=any

FROM snapshot-base AS runtime
COPY tests/e2e/install-packages.sh /opt/e2e-install-packages.sh
# Reuse authenticated indexes without retaining them in the runtime image.
RUN --mount=type=bind,from=package-indexes,source=/var/lib/apt/lists,target=/opt/e2e-apt-lists \
    cp -a /opt/e2e-apt-lists/. /var/lib/apt/lists/ \
    && DEBIAN_FRONTEND=noninteractive sh /opt/e2e-install-packages.sh \
    at-spi2-core bubblewrap ca-certificates dbus-bin dbus-daemon ffmpeg fonts-cantarell \
    gir1.2-atspi-2.0 gir1.2-gtk-4.0 gstreamer1.0-libav gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good imagemagick libfontconfig1 libgtk-4-1 \
    libgtk-4-media-gstreamer libgtksourceview-5-0 libpoppler-glib8 librsvg2-common \
    pulseaudio pulseaudio-utils python3-gi python3-venv \
    xauth xvfb \
    && rm -rf /var/lib/apt/lists/*

COPY tests/e2e/requirements.txt /opt/e2e-requirements.txt
RUN python3 -m venv --system-site-packages /opt/e2e-venv \
    && /opt/e2e-venv/bin/pip install --no-cache-dir -r /opt/e2e-requirements.txt \
    && cp /opt/e2e-requirements.txt /opt/e2e-venv/strata-requirements.txt

# D-Bus requires an account entry even when the runtime accepts a numeric UID.
ARG E2E_UID=1000
ARG E2E_GID=1000
RUN (getent group "$E2E_GID" || groupadd --gid "$E2E_GID" strata-e2e) \
    && (getent passwd "$E2E_UID" || useradd --no-log-init --uid "$E2E_UID" \
        --gid "$E2E_GID" --home-dir /tmp/strata-build-home --no-create-home strata-e2e)

ENV STRATA_E2E_VENV=/opt/e2e-venv
FROM runtime AS toolchain
RUN --mount=type=bind,from=package-indexes,source=/var/lib/apt/lists,target=/opt/e2e-apt-lists \
    cp -a /opt/e2e-apt-lists/. /var/lib/apt/lists/ \
    && DEBIAN_FRONTEND=noninteractive sh /opt/e2e-install-packages.sh \
        build-essential libfontconfig1-dev libgtk-4-dev libgtksourceview-5-dev \
        libpoppler-glib-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev pkg-config rustup \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/opt/rustup RUSTUP_TOOLCHAIN=1.98.1
RUN rustup toolchain install 1.98.1 --no-self-update --profile minimal --component clippy,rustfmt \
    && chmod -R a+rX /opt/rustup

FROM toolchain AS dependencies
WORKDIR /build
ENV CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0
COPY Cargo.toml Cargo.lock ./
COPY crates/media-helper/Cargo.toml ./crates/media-helper/Cargo.toml
COPY crates/media-protocol/Cargo.toml ./crates/media-protocol/Cargo.toml
RUN mkdir -p src crates/media-helper/src crates/media-protocol/src \
    && printf 'fn main() {}\n' > src/main.rs \
    && printf 'fn main() {}\n' > crates/media-helper/src/main.rs \
    && touch crates/media-protocol/src/lib.rs \
    && printf 'fn main() {}\n' > build.rs \
    && cargo build --locked --workspace --bins \
    && rm -rf src build.rs target/debug/strata target/debug/deps/strata* \
        target/debug/.fingerprint/strata-* target/debug/build/strata-*

FROM ${E2E_RUNTIME_IMAGE} AS ci-runtime

FROM ${E2E_DEPENDENCIES_IMAGE} AS application
COPY Cargo.toml Cargo.lock /tmp/strata-e2e-manifests/
RUN cmp Cargo.toml /tmp/strata-e2e-manifests/Cargo.toml \
    && cmp Cargo.lock /tmp/strata-e2e-manifests/Cargo.lock
COPY src ./src
COPY crates ./crates
COPY data ./data
COPY build.rs ./
ARG STRATA_BUILD_COMMIT
RUN cargo build --locked --workspace --bins
COPY tests/e2e ./tests/e2e
COPY scripts/e2e_bundle.py ./scripts/e2e_bundle.py
RUN /opt/e2e-venv/bin/python -m pytest -c tests/e2e/pytest.ini --rootdir /build \
        --collect-only -q -p no:cacheprovider --e2e-write-plan=/bundle/plan.json \
    && cp target/debug/strata target/debug/strata-media-helper /bundle/ \
    && python3 scripts/e2e_bundle.py create /bundle --commit "$STRATA_BUILD_COMMIT"

FROM scratch AS bundle
COPY --from=application /bundle/ /
