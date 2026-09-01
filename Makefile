.PHONY: start-dev run-dev build install-local

start-dev:
	./scripts/dev.sh

run-dev:
	cargo run

build:
	cargo build --release

install-local: build
	install -Dm755 target/release/strata "$(HOME)/.local/bin/strata"
