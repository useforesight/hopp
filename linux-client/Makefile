PREFIX ?= $(HOME)/.local
CARGO ?= cargo
TARGET_DIR ?= $(if $(CARGO_TARGET_DIR),$(abspath $(CARGO_TARGET_DIR)),$(shell $(CARGO) metadata --offline --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])'))
INSTALL_FLAGS ?=
BINARY ?= $(TARGET_DIR)/release/hopp

.PHONY: build plugin check test install uninstall clean

build:
	$(CARGO) build --release --locked --workspace

# The Hyprland plugin that keeps Hopp's overlays out of your screen share. Rebuild after
# every Hyprland update; it refuses to load into a Hyprland it wasn't built for.
plugin:
	$(MAKE) -C hyprland-plugin

test:
	$(CARGO) test --locked --workspace

check:
	$(CARGO) fmt --all -- --check
	$(CARGO) test --locked --workspace
	$(CARGO) clippy --locked --workspace --all-targets -- -D warnings

install: build plugin
	python3 packaging/install.py install --prefix "$(PREFIX)" --binary "$(BINARY)" --share-plugin hyprland-plugin/hopp-share-clean.so $(INSTALL_FLAGS)

uninstall:
	python3 packaging/install.py uninstall --prefix "$(PREFIX)"

clean:
	$(CARGO) clean
	$(MAKE) -C hyprland-plugin clean
