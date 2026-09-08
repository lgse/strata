FROM docker.io/library/ubuntu:24.04@sha256:1e0a86e57d247923571b75e0aaf48a1449cf8c543d51fb3e07a4a7d7bfa79316 AS certificates

RUN apt-get update && apt-get install --yes --no-install-recommends ca-certificates=20260601~24.04.1

FROM docker.io/library/ubuntu:24.04@sha256:1e0a86e57d247923571b75e0aaf48a1449cf8c543d51fb3e07a4a7d7bfa79316 AS runtime
COPY --from=certificates /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt

RUN printf '%s\n' \
    'deb [check-valid-until=no] http://snapshot.ubuntu.com/ubuntu/20260901T000000Z noble main universe' \
    'deb [check-valid-until=no] http://snapshot.ubuntu.com/ubuntu/20260901T000000Z noble-updates main universe' \
    'deb [check-valid-until=no] http://snapshot.ubuntu.com/ubuntu/20260901T000000Z noble-security main universe' \
    > /etc/apt/sources.list \
    && rm /etc/apt/sources.list.d/ubuntu.sources \
    && apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
    at-spi2-core ca-certificates dbus-bin dbus-daemon fonts-cantarell \
    gir1.2-atspi-2.0 gir1.2-gtk-4.0 imagemagick libfontconfig1 libgtk-4-1 \
    libgtksourceview-5-0 libpoppler-glib8 librsvg2-common python3-gi python3-venv \
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
ARG E2E_IMAGE_KEY
LABEL org.strata.e2e.inputs=$E2E_IMAGE_KEY

FROM runtime AS toolchain
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
        build-essential libfontconfig1-dev libgtk-4-dev libgtksourceview-5-dev \
        libpoppler-glib-dev pkg-config rustup \
    && rm -rf /var/lib/apt/lists/*
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

FROM dependencies AS application
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
