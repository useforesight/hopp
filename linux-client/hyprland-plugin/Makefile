# hopp-share-clean: see main.cpp. Built against the installed Hyprland's headers; rebuild
# after every Hyprland update (the plugin refuses to load into a different build).
PLUGIN_NAME = hopp-share-clean

CXXFLAGS ?= -O2
CXXFLAGS += -shared -fPIC -std=c++2b -g

PKG_CONFIG = pkg-config --cflags pixman-1 libdrm hyprland pangocairo libinput libudev wayland-server xkbcommon libeis-1.0

OUT = $(PLUGIN_NAME).so

all: $(OUT)

$(OUT): main.cpp bench.hpp
	$(CXX) $(CXXFLAGS) main.cpp -o $@ `$(PKG_CONFIG)`

# Measuring build, never installed: see bench.hpp.
bench: hopp-share-clean-bench.so

hopp-share-clean-bench.so: main.cpp bench.hpp
	$(CXX) $(CXXFLAGS) -DHOPP_SHARE_CLEAN_BENCH main.cpp -o $@ `$(PKG_CONFIG)` -lGLESv2 -lEGL

clean:
	rm -f $(OUT) hopp-share-clean-bench.so

.PHONY: all bench clean
