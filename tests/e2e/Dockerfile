FROM docker.io/library/ubuntu:24.04@sha256:1e0a86e57d247923571b75e0aaf48a1449cf8c543d51fb3e07a4a7d7bfa79316 AS certificates

RUN apt-get update && apt-get install --yes --no-install-recommends ca-certificates=20260601~24.04.1

FROM docker.io/library/ubuntu:24.04@sha256:1e0a86e57d247923571b75e0aaf48a1449cf8c543d51fb3e07a4a7d7bfa79316
COPY --from=certificates /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt

RUN printf '%s\n' \
    'deb [check-valid-until=no] http://snapshot.ubuntu.com/ubuntu/20260901T000000Z noble main universe' \
    'deb [check-valid-until=no] http://snapshot.ubuntu.com/ubuntu/20260901T000000Z noble-updates main universe' \
    'deb [check-valid-until=no] http://snapshot.ubuntu.com/ubuntu/20260901T000000Z noble-security main universe' \
    > /etc/apt/sources.list \
    && rm /etc/apt/sources.list.d/ubuntu.sources \
    && apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
    at-spi2-core build-essential ca-certificates dbus-bin dbus-daemon fonts-cantarell \
    gir1.2-atspi-2.0 imagemagick libfontconfig1-dev libgtk-4-dev \
    libgtksourceview-5-dev libpoppler-glib-dev librsvg2-common pkg-config python3-gi python3-venv \
    rustup xauth xvfb \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/opt/rustup RUSTUP_TOOLCHAIN=1.98.1
RUN rustup toolchain install 1.98.1 --no-self-update --profile minimal --component clippy,rustfmt \
    && chmod -R a+rX /opt/rustup

COPY requirements.txt /opt/e2e-requirements.txt
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
