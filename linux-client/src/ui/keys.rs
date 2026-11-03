//! Keyboard translation for forwarded input.
use gpui_kit::Keystroke;

/// GPUI key names to the DOM `KeyboardEvent.key` values Hopp peers expect.
pub fn dom_key(key: &Keystroke) -> String {
    match key.key.as_str() {
        "enter" => "Enter",
        "escape" => "Escape",
        "backspace" => "Backspace",
        "tab" => "Tab",
        "space" => " ",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "delete" => "Delete",
        "insert" => "Insert",
        "shift" => "Shift",
        "control" | "ctrl" => "Control",
        "alt" => "Alt",
        "super" | "meta" => "Meta",
        "capslock" => "CapsLock",
        key if key
            .strip_prefix('f')
            .and_then(|value| value.parse::<u32>().ok())
            .is_some_and(|value| (1..=24).contains(&value)) =>
        {
            return key.to_uppercase();
        }
        _ => {
            return key
                .key_char
                .clone()
                .filter(|character| character.chars().count() == 1)
                .unwrap_or_else(|| key.key.clone());
        }
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_translation_preserves_dom_navigation_and_typed_characters() {
        let key = |name: &str, character: Option<&str>| Keystroke {
            key: name.into(),
            key_char: character.map(str::to_owned),
            modifiers: Default::default(),
        };
        assert_eq!(dom_key(&key("left", None)), "ArrowLeft");
        assert_eq!(dom_key(&key("space", Some(" "))), " ");
        assert_eq!(dom_key(&key("a", Some("ä"))), "ä");
        assert_eq!(dom_key(&key("f12", None)), "F12");
    }
}
