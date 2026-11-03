//! Hyprland remote input, scoped to an explicitly selected physical output.
//!
//! Virtual input needs compositor support, but never root access or `/dev/uinput`.
//! Logical keys must exist in the host's XKB keymap. Compose sequences, IMEs and
//! characters absent from that map are deliberately rejected instead of guessed.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Seek, Write},
    os::fd::AsFd,
    time::Instant,
};

use anyhow::{Context, Result, bail, ensure};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat},
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use xkbcommon::xkb;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

struct Output {
    proxy: wl_output::WlOutput,
    name: Option<String>,
}

#[derive(Default)]
struct Desktop {
    outputs: HashMap<u32, Output>,
    seat: Option<wl_seat::WlSeat>,
    physical_keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer_manager: Option<ZwlrVirtualPointerManagerV1>,
    keyboard_manager: Option<ZwpVirtualKeyboardManagerV1>,
    keymap_text: Option<String>,
    keymap_error: Option<String>,
}

pub struct NativeInput {
    connection: Connection,
    queue: EventQueue<Desktop>,
    desktop: Desktop,
    output_id: u32,
    pointer: ZwlrVirtualPointerV1,
    keyboard: ZwpVirtualKeyboardV1,
    keymap: xkb::Keymap,
    keys: HashMap<String, MappedKey>,
    buttons: HashSet<u32>,
    locked_modifiers: u32,
    started: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MappedKey {
    code: u32,
    modifiers: u32,
    group: u32,
}

impl NativeInput {
    pub fn connect(output_name: &str) -> Result<Self> {
        ensure!(
            !output_name.trim().is_empty(),
            "Select a named monitor before enabling remote input"
        );
        let connection =
            Connection::connect_to_env().context("Connecting remote input to Wayland")?;
        let mut queue = connection.new_event_queue();
        let handle = queue.handle();
        let _registry = connection.display().get_registry(&handle, ());
        let mut desktop = Desktop::default();
        // Discover globals, receive output names/seat capabilities, then its keymap.
        for _ in 0..3 {
            queue
                .roundtrip(&mut desktop)
                .context("Discovering native input protocols")?;
        }
        let output_id = desktop
            .outputs
            .iter()
            .find(|(_, output)| output.name.as_deref() == Some(output_name))
            .map(|(id, _)| *id)
            .with_context(|| {
                format!("Monitor {output_name:?} is unavailable; remote input remains disabled")
            })?;
        let output = &desktop.outputs[&output_id].proxy;
        let seat = desktop
            .seat
            .as_ref()
            .context("Wayland seat is unavailable")?;
        let pointer_manager = desktop
            .pointer_manager
            .as_ref()
            .context("Output-scoped virtual pointer protocol v2 is unavailable")?;
        let keyboard_manager = desktop
            .keyboard_manager
            .as_ref()
            .context("Virtual keyboard protocol is unavailable")?;
        let keymap_text = desktop.keymap_text.as_ref().with_context(|| {
            desktop
                .keymap_error
                .as_deref()
                .unwrap_or("Compositor did not provide an XKB keymap")
                .to_owned()
        })?;
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_string(
            &context,
            keymap_text.clone(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .context("Parsing the compositor's keyboard layout")?;
        // Wayland XKB maps are NUL terminated, including the advertised byte size.
        let mut keymap_file = tempfile::tempfile().context("Creating virtual keyboard keymap")?;
        keymap_file.write_all(keymap_text.as_bytes())?;
        keymap_file.write_all(&[0])?;
        let keymap_size =
            u32::try_from(keymap_text.len() + 1).context("Keymap exceeds protocol size")?;
        let pointer = pointer_manager.create_virtual_pointer_with_output(
            Some(seat),
            Some(output),
            &handle,
            (),
        );
        let keyboard = keyboard_manager.create_virtual_keyboard(seat, &handle, ());
        keyboard.keymap(
            wl_keyboard::KeymapFormat::XkbV1 as u32,
            keymap_file.as_fd(),
            keymap_size,
        );
        keyboard.modifiers(0, 0, 0, 0);
        queue
            .roundtrip(&mut desktop)
            .context("Initializing virtual input devices")?;
        Ok(Self {
            connection,
            queue,
            desktop,
            output_id,
            pointer,
            keyboard,
            keymap,
            keys: HashMap::new(),
            buttons: HashSet::new(),
            locked_modifiers: 0,
            started: Instant::now(),
        })
    }

    /// Coordinates are relative to the selected output, regardless of its scale,
    /// position, rotation or the number of monitors in the desktop.
    pub fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
        let (x, y) = normalized_position(x, y)?;
        self.check_output()?;
        self.pointer
            .motion_absolute(self.timestamp(), x, y, 1_000_000, 1_000_000);
        self.pointer.frame();
        self.flush()
    }

    /// Accept Linux evdev BTN_* codes (BTN_LEFT = 272, RIGHT = 273, MIDDLE = 274).
    /// Callers must translate Hopp's mouse-button enum before invoking this method.
    pub fn button(&mut self, button: u32, down: bool) -> Result<()> {
        ensure!(
            (272..=279).contains(&button),
            "Unsupported evdev mouse button {button}"
        );
        self.check_output()?;
        let changed = if down {
            self.buttons.insert(button)
        } else {
            self.buttons.remove(&button)
        };
        if changed {
            self.pointer.button(
                self.timestamp(),
                button,
                if down {
                    wl_pointer::ButtonState::Pressed
                } else {
                    wl_pointer::ButtonState::Released
                },
            );
            self.pointer.frame();
        }
        self.flush()
    }

    /// Smooth scroll deltas, positive right/down, in Wayland touchpad coordinates.
    pub fn scroll(&mut self, dx: f64, dy: f64) -> Result<()> {
        ensure!(
            dx.is_finite() && dy.is_finite(),
            "Scroll deltas must be finite"
        );
        // Wayland fixed-point values have a signed 24-bit integer range.
        ensure!(
            dx.abs() <= 1_000_000.0 && dy.abs() <= 1_000_000.0,
            "Scroll delta exceeds supported range"
        );
        self.check_output()?;
        self.pointer.axis_source(wl_pointer::AxisSource::Continuous);
        if dx != 0.0 {
            self.pointer
                .axis(self.timestamp(), wl_pointer::Axis::HorizontalScroll, dx);
        }
        if dy != 0.0 {
            self.pointer
                .axis(self.timestamp(), wl_pointer::Axis::VerticalScroll, dy);
        }
        self.pointer.frame();
        self.flush()
    }

    /// Translate DOM/Hopp logical key strings through the compositor's XKB map.
    /// Releases retain the original scan code even when modifier state changes.
    pub fn key(&mut self, key: &str, down: bool, modifiers: InputModifiers) -> Result<()> {
        self.check_output()?;
        let requested = modifier_mask(&self.keymap, modifiers);
        if down {
            // Repeated network key-down messages do not create duplicate presses;
            // compositor/application keyboard repeat owns the repeat timing.
            if self.keys.contains_key(key) {
                return Ok(());
            }
            let mapped = map_key(&self.keymap, key, requested | self.locked_modifiers)?;
            ensure!(
                !self
                    .keys
                    .values()
                    .any(|pressed| pressed.code == mapped.code),
                "Another logical key already holds this physical key"
            );
            self.keyboard.modifiers(
                mapped.modifiers | requested,
                0,
                self.locked_modifiers,
                mapped.group,
            );
            self.keyboard.key(
                self.timestamp(),
                mapped.code,
                wl_keyboard::KeyState::Pressed as u32,
            );
            self.keys.insert(key.to_owned(), mapped);
            let toggled = lock_mask_for_key(&self.keymap, mapped);
            if toggled != 0 {
                self.locked_modifiers ^= toggled;
                self.keyboard.modifiers(
                    mapped.modifiers | requested,
                    0,
                    self.locked_modifiers,
                    mapped.group,
                );
            }
        } else if let Some(mapped) = remove_pressed_key(
            &self.keymap,
            &mut self.keys,
            key,
            requested | self.locked_modifiers,
        ) {
            // Apply release-event modifiers after key-up to preserve shortcut state.
            self.keyboard.key(
                self.timestamp(),
                mapped.code,
                wl_keyboard::KeyState::Released as u32,
            );
            let held = self.keys.values().fold(0, |mask, key| mask | key.modifiers);
            self.keyboard
                .modifiers(requested | held, 0, self.locked_modifiers, mapped.group);
        } else {
            // Hopp can send a modifier-only key-up with no corresponding key-down.
            self.keyboard
                .modifiers(requested, 0, self.locked_modifiers, 0);
        }
        self.flush()
    }

    /// Call immediately on disconnect, revoked consent or stopped screen sharing.
    pub fn release_all(&mut self) -> Result<()> {
        self.locked_modifiers = 0;
        let time = self.timestamp();
        for (_, key) in self.keys.drain() {
            self.keyboard
                .key(time, key.code, wl_keyboard::KeyState::Released as u32);
        }
        self.keyboard.modifiers(0, 0, 0, 0);
        for button in self.buttons.drain() {
            self.pointer
                .button(time, button, wl_pointer::ButtonState::Released);
        }
        self.pointer.frame();
        self.flush()
    }

    fn timestamp(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    fn flush(&self) -> Result<()> {
        self.connection
            .flush()
            .context("Sending native remote input")
    }

    fn check_output(&mut self) -> Result<()> {
        // A local roundtrip drains output-removal events before any injection.
        self.queue
            .roundtrip(&mut self.desktop)
            .context("Checking the selected monitor")?;
        if !self.desktop.outputs.contains_key(&self.output_id) {
            let _ = self.release_all();
            bail!("Selected monitor was disconnected; remote input disabled");
        }
        Ok(())
    }
}

impl Drop for NativeInput {
    fn drop(&mut self) {
        let _ = self.release_all();
        self.pointer.destroy();
        self.keyboard.destroy();
        let _ = self.connection.flush();
    }
}

fn normalized_position(x: f64, y: f64) -> Result<(u32, u32)> {
    ensure!(
        x.is_finite() && y.is_finite(),
        "Pointer coordinates must be finite"
    );
    ensure!(
        (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
        "Pointer coordinates must be normalized to 0..1"
    );
    // Keep the final pixel inside the selected monitor rather than crossing its edge.
    Ok((
        (x * 1_000_000.0).round().min(999_999.0) as u32,
        (y * 1_000_000.0).round().min(999_999.0) as u32,
    ))
}

fn remove_pressed_key(
    keymap: &xkb::Keymap,
    keys: &mut HashMap<String, MappedKey>,
    key: &str,
    requested: u32,
) -> Option<MappedKey> {
    if let Some(mapped) = keys.remove(key) {
        return Some(mapped);
    }
    // DOM key strings can change while held: Shift+1 presses "!", then
    // releasing Shift before 1 can produce a "1" key-up. Match its original
    // physical key rather than leaving it stuck until the session ends.
    let code = map_key(keymap, key, requested).ok()?.code;
    let original = keys
        .iter()
        .find(|(_, pressed)| pressed.code == code)
        .map(|(original, _)| original.clone())?;
    keys.remove(&original)
}

fn modifier_bit(keymap: &xkb::Keymap, name: &str) -> u32 {
    let index = keymap.mod_get_index(name);
    if index < 32 { 1 << index } else { 0 }
}

fn lock_mask_for_key(keymap: &xkb::Keymap, key: MappedKey) -> u32 {
    let mut state = xkb::State::new(keymap);
    state.update_mask(key.modifiers, 0, 0, 0, 0, key.group);
    state.update_key(xkb::Keycode::new(key.code + 8), xkb::KeyDirection::Down);
    state.serialize_mods(xkb::STATE_MODS_LOCKED)
}

fn modifier_mask(keymap: &xkb::Keymap, modifiers: InputModifiers) -> u32 {
    [
        (modifiers.shift, xkb::MOD_NAME_SHIFT),
        (modifiers.ctrl, xkb::MOD_NAME_CTRL),
        (modifiers.alt, xkb::MOD_NAME_ALT),
        (modifiers.meta, xkb::MOD_NAME_LOGO),
    ]
    .into_iter()
    .filter(|(enabled, _)| *enabled)
    .fold(0, |mask, (_, name)| mask | modifier_bit(keymap, name))
}

fn logical_keysym(key: &str) -> Result<xkb::Keysym> {
    let mut characters = key.chars();
    if let Some(character) = characters.next()
        && characters.next().is_none()
    {
        return Ok(xkb::utf32_to_keysym(character as u32));
    }
    let name = match key {
        "Enter" => "Return",
        "Escape" => "Escape",
        "Backspace" => "BackSpace",
        "Tab" => "Tab",
        "Space" | "Spacebar" => "space",
        "Delete" => "Delete",
        "ArrowLeft" | "Left" => "Left",
        "ArrowRight" | "Right" => "Right",
        "ArrowUp" | "Up" => "Up",
        "ArrowDown" | "Down" => "Down",
        "Home" => "Home",
        "End" => "End",
        "PageUp" => "Prior",
        "PageDown" => "Next",
        "Insert" => "Insert",
        "Shift" => "Shift_L",
        "Control" | "Ctrl" => "Control_L",
        "Alt" => "Alt_L",
        "AltGraph" => "ISO_Level3_Shift",
        "Meta" | "Super" | "OS" => "Super_L",
        "CapsLock" => "Caps_Lock",
        "NumLock" => "Num_Lock",
        "ScrollLock" => "Scroll_Lock",
        "PrintScreen" => "Print",
        "Pause" => "Pause",
        "ContextMenu" => "Menu",
        key if key
            .strip_prefix('F')
            .and_then(|value| value.parse::<u32>().ok())
            .is_some_and(|number| (1..=24).contains(&number)) =>
        {
            key
        }
        _ => bail!("Unsupported logical key {key:?}; IME/compose text is not implemented"),
    };
    let symbol = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
    ensure!(symbol.raw() != 0, "Unknown XKB key {name:?}");
    Ok(symbol)
}

fn map_key(keymap: &xkb::Keymap, key: &str, requested: u32) -> Result<MappedKey> {
    let symbol = logical_keysym(key)?;
    let shift = modifier_bit(keymap, xkb::MOD_NAME_SHIFT);
    let level3 = modifier_bit(keymap, xkb::MOD_NAME_ISO_LEVEL3_SHIFT);
    // Query XKB state rather than assuming level 1 always means Shift: keyboard
    // layouts such as AZERTY assign punctuation and letters at different levels.
    let candidates = [0, shift, level3, shift | level3];
    let mut state = xkb::State::new(keymap);
    for group in 0..keymap.num_layouts() {
        for extra in candidates {
            state.update_mask(requested | extra, 0, 0, 0, 0, group);
            for code in keymap.min_keycode().raw()..=keymap.max_keycode().raw() {
                if code >= 8
                    && state
                        .key_get_syms(xkb::Keycode::new(code))
                        .contains(&symbol)
                {
                    return Ok(MappedKey {
                        code: code - 8,
                        modifiers: extra,
                        group,
                    });
                }
            }
        }
    }
    // Shortcuts can carry a lowercase logical key alongside Shift (e.g. Ctrl+Shift+P).
    // Resolve its unmodified physical key and let the requested modifiers apply.
    if requested != 0 {
        return map_key(keymap, key, 0);
    }
    bail!(
        "Key {key:?} is absent from the host keyboard layout; remote text composition is unavailable"
    )
}

impl Dispatch<wl_registry::WlRegistry, ()> for Desktop {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_output" if version >= 4 => {
                    let proxy = registry.bind(name, 4, handle, name);
                    state.outputs.insert(name, Output { proxy, name: None });
                }
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(7), handle, ()));
                }
                "zwlr_virtual_pointer_manager_v1" if version >= 2 => {
                    state.pointer_manager = Some(registry.bind(name, 2, handle, ()));
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.keyboard_manager = Some(registry.bind(name, 1, handle, ()));
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                state.outputs.remove(&name);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for Desktop {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event
            && let Some(output) = state.outputs.get_mut(id)
        {
            output.name = Some(name);
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Desktop {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
            && capabilities.contains(wl_seat::Capability::Keyboard)
            && state.physical_keyboard.is_none()
        {
            state.physical_keyboard = Some(seat.get_keyboard(handle, ()));
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Desktop {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Keymap { format, fd, size } = event {
            let result = (|| -> Result<String> {
                ensure!(
                    format == WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
                    "Unsupported compositor keymap format"
                );
                ensure!(
                    size > 0 && size <= 16 * 1024 * 1024,
                    "Invalid compositor keymap size"
                );
                let mut bytes = Vec::with_capacity(size as usize);
                let mut file = File::from(fd);
                file.rewind()?;
                file.take(u64::from(size)).read_to_end(&mut bytes)?;
                ensure!(bytes.len() == size as usize, "Incomplete compositor keymap");
                if bytes.last() == Some(&0) {
                    bytes.pop();
                }
                ensure!(!bytes.contains(&0), "Invalid NUL in compositor keymap");
                String::from_utf8(bytes).context("Compositor keymap is not UTF-8")
            })();
            match result {
                Ok(text) => state.keymap_text = Some(text),
                Err(error) => state.keymap_error = Some(error.to_string()),
            }
        }
    }
}

delegate_noop!(Desktop: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(Desktop: ignore ZwlrVirtualPointerV1);
delegate_noop!(Desktop: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(Desktop: ignore ZwpVirtualKeyboardV1);

#[cfg(test)]
mod tests {
    use super::*;

    fn keymap(layout: &str) -> xkb::Keymap {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            layout,
            "",
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .unwrap()
    }

    #[test]
    fn rejects_coordinates_that_can_escape_the_selected_monitor() {
        assert!(normalized_position(f64::NAN, 0.5).is_err());
        assert!(normalized_position(0.5, f64::INFINITY).is_err());
        assert!(normalized_position(-0.01, 0.5).is_err());
        assert!(normalized_position(0.5, 1.01).is_err());
        assert_eq!(normalized_position(1.0, 1.0).unwrap(), (999_999, 999_999));
    }

    #[test]
    fn maps_logical_keys_and_shortcuts_to_evdev() {
        let map = keymap("us");
        assert_eq!(map_key(&map, "a", 0).unwrap().code, 30);
        assert_eq!(map_key(&map, "Enter", 0).unwrap().code, 28);
        assert_eq!(map_key(&map, "ArrowLeft", 0).unwrap().code, 105);
        let shift = modifier_bit(&map, xkb::MOD_NAME_SHIFT);
        let uppercase = map_key(&map, "A", 0).unwrap();
        assert_eq!(uppercase.code, 30);
        assert_ne!(uppercase.modifiers & shift, 0);
        assert_eq!(map_key(&map, "a", shift).unwrap().code, 30);
        assert!(logical_keysym("Unidentified").is_err());
        assert!(logical_keysym("F999").is_err());
        assert!(map_key(&map, "🦀", 0).is_err());
    }

    #[test]
    fn follows_non_us_layout_and_resolves_altgr() {
        let map = keymap("de");
        assert_eq!(map_key(&map, "z", 0).unwrap().code, 21);
        assert_eq!(map_key(&map, "y", 0).unwrap().code, 44);
        assert_eq!(map_key(&map, "ä", 0).unwrap().code, 40);
        let at = map_key(&map, "@", 0).unwrap();
        assert_eq!(at.code, 16);
        assert_ne!(
            at.modifiers & modifier_bit(&map, xkb::MOD_NAME_ISO_LEVEL3_SHIFT),
            0
        );
    }

    #[test]
    fn key_release_survives_modifier_changed_logical_key() {
        let map = keymap("us");
        let pressed = map_key(&map, "!", 0).unwrap();
        let mut keys = HashMap::from([("!".to_owned(), pressed)]);
        assert_eq!(remove_pressed_key(&map, &mut keys, "1", 0), Some(pressed));
        assert!(keys.is_empty());
        assert_eq!(remove_pressed_key(&map, &mut keys, "1", 0), None);
    }

    #[test]
    fn serializes_lock_state_from_the_actual_keymap() {
        let map = keymap("us");
        let caps = lock_mask_for_key(&map, map_key(&map, "CapsLock", 0).unwrap());
        assert_eq!(caps, modifier_bit(&map, xkb::MOD_NAME_CAPS));
        assert_eq!(lock_mask_for_key(&map, map_key(&map, "a", 0).unwrap()), 0);
        assert_eq!(map_key(&map, "A", caps).unwrap().modifiers, 0);
        assert_ne!(
            map_key(&map, "a", caps).unwrap().modifiers & modifier_bit(&map, xkb::MOD_NAME_SHIFT),
            0
        );
    }
}
