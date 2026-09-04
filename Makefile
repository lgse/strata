.PHONY: start-dev run-dev build install-local uninstall-local

DATA_HOME ?= $(HOME)/.local/share
BIN_DIR ?= $(HOME)/.local/bin

start-dev:
	./scripts/dev.sh

run-dev:
	cargo run

build:
	cargo build --release

install-local: build
	install -Dm755 target/release/strata "$(BIN_DIR)/strata"
	install -Dm644 data/icons/scalable/apps/io.github.lgse.Strata.svg \
		"$(DATA_HOME)/icons/hicolor/scalable/apps/io.github.lgse.Strata.svg"
	install -d "$(DATA_HOME)/applications"
	sed 's|^Exec=strata |Exec=$(BIN_DIR)/strata |' data/io.github.lgse.Strata.desktop \
		> "$(DATA_HOME)/applications/io.github.lgse.Strata.desktop"
	install -d "$(DATA_HOME)/dbus-1/services"
	sed 's|^Exec=/usr/bin/strata |Exec=$(BIN_DIR)/strata |' \
		data/io.github.lgse.Strata.FileManager1.service \
		> "$(DATA_HOME)/dbus-1/services/io.github.lgse.Strata.FileManager1.service"
	update-desktop-database "$(DATA_HOME)/applications" 2>/dev/null || true
	gtk-update-icon-cache -qtf "$(DATA_HOME)/icons/hicolor" 2>/dev/null || true

uninstall-local:
	rm -f "$(BIN_DIR)/strata" \
		"$(DATA_HOME)/applications/io.github.lgse.Strata.desktop" \
		"$(DATA_HOME)/dbus-1/services/io.github.lgse.Strata.FileManager1.service" \
		"$(DATA_HOME)/icons/hicolor/scalable/apps/io.github.lgse.Strata.svg"
	update-desktop-database "$(DATA_HOME)/applications" 2>/dev/null || true
	gtk-update-icon-cache -qtf "$(DATA_HOME)/icons/hicolor" 2>/dev/null || true
