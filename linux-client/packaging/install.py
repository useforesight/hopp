#!/usr/bin/env python3
"""Install/remove the native binary and browser deep-link integration."""

import argparse
import json
import os
import shlex
import shutil
import subprocess
import tempfile
from pathlib import Path

APP = "hopp.desktop"
# Names from before the client was renamed to `hopp`; install and uninstall clear them.
LEGACY_APP = "hopp-omarchy.desktop"
SCHEME = "x-scheme-handler/hopp"
HELPER = "hopp-transcribe"
# The helper before it ran transcribe.cpp.
LEGACY_HELPER = "hopp-whisper"
PACKAGE = Path(__file__).resolve().parent
# Call sounds are shared with the desktop app rather than copied into this client.
SOUNDS = PACKAGE.parent.parent / "tauri/src-tauri/resources/sounds"
SOUND_FILES = ("incoming-call.mp3", "ring1.mp3", "bubble-pop.mp3", "rejected.mp3")


def run_optional(*command):
    if shutil.which(command[0]):
        return subprocess.run(command, capture_output=True, text=True, check=False)
    return None


def current_handler():
    result = run_optional("xdg-mime", "query", "default", SCHEME)
    return result.stdout.strip() if result and result.returncode == 0 else ""


def atomic_write(destination, content, mode):
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as output:
        temporary = Path(output.name)
        try:
            output.write(content)
            output.flush()
            os.fchmod(output.fileno(), mode)
            os.fsync(output.fileno())
            os.replace(temporary, destination)
        finally:
            temporary.unlink(missing_ok=True)


def desktop_binary(path):
    # Exec is parsed by the desktop launcher, not by a shell. Percent signs
    # must be escaped so a literal filename cannot become a field code.
    value = str(path).replace("%", "%%").replace("\\", "\\\\")
    for character in ('"', '`', '$'):
        value = value.replace(character, "\\" + character)
    return '"' + value + '"'


REQUIRE = 'require("hypr.hopp") -- Hopp for Omarchy; `make uninstall` removes this line\n'


def hypr_paths():
    config = Path(os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config")))
    return config / "hypr/hopp.lua", config / "hypr/hyprland.lua"


def install_hyprland(binary, share_plugin=None):
    rules, main = hypr_paths()
    if not main.is_file():
        print("No Lua Hyprland config found; skipped window rules and keybindings")
        return
    # o.bind runs the command through a shell, so quote the path for the shell and
    # then encode it as a Lua string (JSON string escapes are valid Lua for paths).
    command = json.dumps(shlex.quote(str(binary)))
    template = (PACKAGE / "hypr/hopp.lua").read_text()
    template = template.replace("@SHARE_PLUGIN@", install_share_plugin(share_plugin))
    atomic_write(rules, template.replace("@BINARY@", command).encode(), 0o644)
    text = main.read_text()
    if 'require("hypr.hopp")' not in text:
        separator = "" if text.endswith("\n") else "\n"
        atomic_write(main, (text + separator + REQUIRE).encode(), main.stat().st_mode & 0o777)
        print(f"Added one line to {main} to load {rules}")
    print(f"Hyprland rules and Super+H / Super+Alt+H / Super+Ctrl+Esc / Super+Alt+C / Super+Alt+D / Super+Alt+P: {rules}")


SHARE_PLUGIN = "hopp-share-clean.so"


def share_plugin_path():
    data = Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))
    return data.parent / "lib/hyprland" / SHARE_PLUGIN


def install_share_plugin(source):
    """Install the Hyprland plugin and return the Lua line that loads it.

    The first install loads it straight away, when Hyprland rereads its config. Hyprland
    loads a path only once per session, so a rebuilt plugin takes effect at the next
    login, never replacing the one running in the compositor."""
    if source is None or not source.is_file():
        print("Hyprland plugin not built; Hopp won't draw over a screen you share")
        return ""
    destination = share_plugin_path()
    first = not destination.exists()
    if not destination.exists() or destination.read_bytes() != source.read_bytes():
        atomic_write(destination, source.read_bytes(), 0o755)
    if first:
        print(f"Hyprland plugin: {destination}. Hyprland loads it as it rereads its config")
    else:
        print(f"Hyprland plugin: {destination} (a new build loads at the next login)")
    return f"hl.plugin.load({json.dumps(str(destination))})"


def uninstall_hyprland():
    rules, main = hypr_paths()
    rules.unlink(missing_ok=True)
    share_plugin_path().unlink(missing_ok=True)
    if main.is_file():
        lines = main.read_text().splitlines(keepends=True)
        kept = [line for line in lines if 'require("hypr.hopp")' not in line]
        if kept != lines:
            atomic_write(main, "".join(kept).encode(), main.stat().st_mode & 0o777)
            print(f"Removed the Hopp line from {main}")


PLUGIN_ID = "local.hopp"


def plugin_path():
    config = Path(os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config")))
    return config / "omarchy/plugins" / PLUGIN_ID


def install_plugin(enable):
    destination = plugin_path()
    if not destination.parent.parent.is_dir():
        print("No Omarchy shell config found; skipped the bar plugin")
        return
    source = PACKAGE.parent / "omarchy-plugin" / PLUGIN_ID
    destination.mkdir(parents=True, exist_ok=True)
    for item in source.iterdir():
        if item.is_file() and item.suffix in {".qml", ".py", ".json"}:
            atomic_write(destination / item.name, item.read_bytes(), 0o755 if item.suffix == ".py" else 0o644)
    run_optional("omarchy-shell", "shell", "rescanPlugins")
    if enable:
        # The shell discovers new plugins asynchronously after a rescan.
        for _ in range(20):
            result = run_optional("omarchy", "plugin", "enable", PLUGIN_ID)
            if result and result.returncode == 0 and "Enabled" in result.stdout:
                print("Hopp is in your bar")
                break
            import time
            time.sleep(0.25)
        else:
            print(f"Bar plugin installed; enable it with: omarchy plugin enable {PLUGIN_ID}")
    elif plugin_enabled():
        print(f"Bar plugin updated in {destination}")
    else:
        print(f"Bar plugin installed in {destination}; add it to your bar with: omarchy plugin enable {PLUGIN_ID}")


def plugin_enabled():
    result = run_optional("omarchy-shell", "shell", "listPlugins")
    try:
        plugins = json.loads(result.stdout) if result and result.returncode == 0 else []
    except json.JSONDecodeError:
        return False
    return any(plugin.get("id") == PLUGIN_ID and plugin.get("enabled") for plugin in plugins)


def uninstall_plugin():
    destination = plugin_path()
    if destination.is_dir():
        run_optional("omarchy", "plugin", "disable", PLUGIN_ID)
        shutil.rmtree(destination)
        run_optional("omarchy-shell", "shell", "rescanPlugins")
        print(f"Removed the bar plugin from {destination}")


def paths(prefix):
    return (prefix / "bin/hopp", prefix / "share/applications" / APP,
            prefix / "share/icons/hicolor/scalable/apps/hopp.svg",
            prefix / "share/hopp/install-state.json")


def legacy_paths(prefix):
    return (prefix / "bin/hopp-omarchy", prefix / "share/applications" / LEGACY_APP,
            prefix / "share/icons/hicolor/scalable/apps/hopp-omarchy.svg",
            prefix / "share/hopp-omarchy/install-state.json")


def remove_legacy(prefix):
    """Remove files installed under the old `hopp-omarchy` name. Returns the browser
    handler that was in place before the old install, if it recorded one."""
    binary, desktop, icon, state = legacy_paths(prefix)
    previous = ""
    if state.exists():
        previous = json.loads(state.read_text()).get("previous_handler", "")
    found = [path for path in (binary, desktop, icon, state) if path.exists()]
    for path in found:
        path.unlink(missing_ok=True)
    (binary.parent / LEGACY_HELPER).unlink(missing_ok=True)
    shutil.rmtree(state.parent / "sounds", ignore_errors=True)
    if state.parent.is_dir() and not any(state.parent.iterdir()):
        state.parent.rmdir()
    if found:
        print(f"Removed the old hopp-omarchy install ({len(found)} files)")
    return previous


def install(prefix, source, hyprland=True, plugin=True, enable_plugin=False, share_plugin=None):
    binary, desktop, icon, state = paths(prefix)
    if source is None or not source.is_file():
        raise RuntimeError("Build the release binary before installing")
    legacy_previous = remove_legacy(prefix)
    previous = current_handler()
    if previous == APP and state.exists():
        previous = json.loads(state.read_text()).get("previous_handler", "")
    elif previous == LEGACY_APP:
        previous = legacy_previous
    atomic_write(binary, source.read_bytes(), 0o755)
    # Capture's transcription helper is built next to the client.
    helper = source.parent / HELPER
    if helper.is_file():
        atomic_write(binary.parent / HELPER, helper.read_bytes(), 0o755)
    else:
        print(f"{helper} not found; Capture won't be able to transcribe")
    template = (PACKAGE / "hopp.desktop.in").read_text()
    atomic_write(desktop, template.replace("@BINARY@", desktop_binary(binary)).encode(), 0o644)
    atomic_write(icon, (PACKAGE / "hopp.svg").read_bytes(), 0o644)
    atomic_write(state, json.dumps({"previous_handler": previous}).encode(), 0o600)
    sounds = state.parent / "sounds"
    if all((SOUNDS / name).is_file() for name in SOUND_FILES):
        for name in SOUND_FILES:
            atomic_write(sounds / name, (SOUNDS / name).read_bytes(), 0o644)
    else:
        print(f"Call sounds not found in {SOUNDS}; Hopp will not ring")
    run_optional("update-desktop-database", str(desktop.parent))
    run_optional("gtk-update-icon-cache", "--ignore-theme-index", str(prefix / "share/icons/hicolor"))
    # Register only the main/default instance as the browser's Hopp handler.
    result = run_optional("xdg-mime", "default", APP, SCHEME)
    if result and result.returncode:
        raise RuntimeError("Files installed, but the desktop could not register Hopp browser links")
    print(f"Installed {binary}\nDesktop entry: {desktop}")
    if hyprland:
        install_hyprland(binary, share_plugin)
    if plugin:
        install_plugin(enable_plugin)


def remove_own_mime_default():
    config = Path(os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config")))
    data = Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share"))) / "applications"
    names = {"mimeapps.list"}
    names.update(name.lower() + "-mimeapps.list" for name in os.environ.get("XDG_CURRENT_DESKTOP", "").split(":") if name)
    for directory in (config, data):
        for name in names:
            path = directory / name
            if not path.is_file():
                continue
            lines = path.read_text().splitlines(keepends=True)
            output, section, changed = [], "", False
            for line in lines:
                if line.strip().startswith("["):
                    section = line.strip()
                key, separator, value = line.partition("=")
                if section == "[Default Applications]" and separator and key.strip() == SCHEME:
                    applications = [item for item in value.strip().split(";") if item and item != APP]
                    if APP in value.strip().split(";"):
                        changed = True
                        if applications:
                            output.append(SCHEME + "=" + ";".join(applications) + ";\n")
                        continue
                output.append(line)
            if changed:
                atomic_write(path, "".join(output).encode(), path.stat().st_mode & 0o777)


def uninstall(prefix, hyprland=True, plugin=True):
    binary, desktop, icon, state = paths(prefix)
    previous = json.loads(state.read_text()).get("previous_handler", "") if state.exists() else ""
    if current_handler() == APP:
        if previous:
            run_optional("xdg-mime", "default", previous, SCHEME)
        else:
            remove_own_mime_default()
    for path in (binary, binary.parent / HELPER, desktop, icon, state):
        path.unlink(missing_ok=True)
    shutil.rmtree(state.parent / "sounds", ignore_errors=True)
    remove_legacy(prefix)
    run_optional("update-desktop-database", str(desktop.parent))
    run_optional("gtk-update-icon-cache", "--ignore-theme-index", str(prefix / "share/icons/hicolor"))
    if hyprland:
        uninstall_hyprland()
    if plugin:
        uninstall_plugin()
    print(f"Removed {binary}; saved account settings retained")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("install", "uninstall"))
    parser.add_argument("--prefix", type=Path, default=Path.home() / ".local")
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--no-hyprland", action="store_true", help="leave ~/.config/hypr untouched")
    parser.add_argument("--no-plugin", action="store_true", help="skip the Omarchy bar plugin")
    parser.add_argument("--enable-plugin", action="store_true", help="also place Hopp in the bar")
    parser.add_argument("--share-plugin", type=Path, help="the built hopp-share-clean Hyprland plugin")
    arguments = parser.parse_args()
    prefix = arguments.prefix.expanduser().resolve()
    try:
        if arguments.command == "install":
            install(prefix, arguments.binary, not arguments.no_hyprland,
                    not arguments.no_plugin, arguments.enable_plugin, arguments.share_plugin)
        else:
            uninstall(prefix, not arguments.no_hyprland, not arguments.no_plugin)
    except (OSError, RuntimeError, ValueError) as error:
        raise SystemExit(str(error)) from None


if __name__ == "__main__":
    main()
