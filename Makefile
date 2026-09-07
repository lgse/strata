.PHONY: start-dev run-dev run-chooser-dev build install-local uninstall-local

DATA_HOME ?= $(HOME)/.local/share
BIN_DIR ?= $(HOME)/.local/bin
CHOOSER_CASE ?= save
CHOOSER_ARGS ?= --choices

start-dev:
	./scripts/dev.sh

run-dev:
	cargo run

run-chooser-dev:
	cargo build
	GTK_A11Y=none python3 scripts/portal-test.py $(CHOOSER_CASE) --binary target/debug/strata $(CHOOSER_ARGS)

build:
	cargo build --release

install-local: build
	install -Dm755 target/release/strata "$(BIN_DIR)/strata"
	install -Dm644 data/icons/scalable/apps/io.github.lgse.Strata.svg \
		"$(DATA_HOME)/icons/hicolor/scalable/apps/io.github.lgse.Strata.svg"
	install -d "$(DATA_HOME)/applications"
	sed 's|^Exec=strata |Exec=$(BIN_DIR)/strata |' data/io.github.lgse.Strata.desktop \
		> "$(DATA_HOME)/applications/io.github.lgse.Strata.desktop"
	update-desktop-database "$(DATA_HOME)/applications" 2>/dev/null || true
	gtk-update-icon-cache -qtf "$(DATA_HOME)/icons/hicolor" 2>/dev/null || true

install-file-manager: build
	install -d "$(DATA_HOME)/dbus-1/services"
	@conflict="$$(grep -l '^Name=org\.freedesktop\.FileManager1$$' \
		"$(DATA_HOME)"/dbus-1/services/*.service 2>/dev/null | \
		grep -v '/io.github.lgse.Strata.FileManager1.service$$' | head -n 1)"; \
	if [ -n "$$conflict" ]; then \
		echo "Refusing to override the per-user FileManager1 provider: $$conflict" >&2; \
		exit 1; \
	fi
	sed 's|^Exec=/usr/bin/strata |Exec=$(BIN_DIR)/strata |' \
		data/io.github.lgse.Strata.FileManager1.service \
		> "$(DATA_HOME)/dbus-1/services/io.github.lgse.Strata.FileManager1.service"

uninstall-file-manager:
	rm -f "$(DATA_HOME)/dbus-1/services/io.github.lgse.Strata.FileManager1.service"

uninstall-local: uninstall-file-manager
	rm -f "$(BIN_DIR)/strata" \
		"$(DATA_HOME)/applications/io.github.lgse.Strata.desktop" \
		"$(DATA_HOME)/icons/hicolor/scalable/apps/io.github.lgse.Strata.svg"
	update-desktop-database "$(DATA_HOME)/applications" 2>/dev/null || true
	gtk-update-icon-cache -qtf "$(DATA_HOME)/icons/hicolor" 2>/dev/null || true
