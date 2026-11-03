//! People's colours, from the active Omarchy theme's own palette (`colors.toml`), so
//! speakers and faces match the rest of the desktop and change with the theme.

use std::{path::PathBuf, sync::Mutex};

use gpui_kit::{App, Hsla, Rgba};
use gpui_omarchy::ActiveTheme;

/// Theme colours for people, in the order they're handed out. Red is left out: it reads
/// as an error. The bright variants follow for calls bigger than six.
const ORDER: [&str; 12] = [
    "blue",
    "magenta",
    "green",
    "orange",
    "cyan",
    "yellow",
    "bright_blue",
    "bright_magenta",
    "bright_green",
    "bright_yellow",
    "bright_cyan",
    "brown",
];

/// The palette last read, and the theme it was read for.
static CACHE: Mutex<Option<(String, Vec<Hsla>)>> = Mutex::new(None);

fn theme_file() -> Option<PathBuf> {
    let home = std::env::home_dir()?;
    [
        ".local/state/omarchy/current/theme/colors.toml",
        ".config/omarchy/current/theme/colors.toml",
    ]
    .into_iter()
    .map(|relative| home.join(relative))
    .find(|path| path.is_file())
}

fn hex(value: &str) -> Option<Hsla> {
    let digits = value.trim().trim_matches('"').strip_prefix('#')?;
    let packed = u32::from_str_radix(digits.get(..6)?, 16).ok()?;
    Some(Hsla::from(Rgba {
        r: ((packed >> 16) & 0xff) as f32 / 255.,
        g: ((packed >> 8) & 0xff) as f32 / 255.,
        b: (packed & 0xff) as f32 / 255.,
        a: 1.,
    }))
}

/// The people colours a `colors.toml` defines, in [`ORDER`], without repeats.
fn parse(toml: &str) -> Vec<Hsla> {
    let entries: Vec<(&str, Hsla)> = toml
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.trim(), hex(value)?))
        })
        .collect();
    let mut colors: Vec<Hsla> = Vec::new();
    for name in ORDER {
        if let Some((_, color)) = entries.iter().find(|(key, _)| *key == name)
            && !colors.contains(color)
        {
            colors.push(*color);
        }
    }
    colors
}

/// The theme's people colours; empty if the theme has no palette file.
fn colors(cx: &App) -> Vec<Hsla> {
    let theme = cx.omarchy().name.to_string();
    let Ok(mut cache) = CACHE.lock() else {
        return Vec::new();
    };
    if let Some((name, colors)) = cache.as_ref()
        && *name == theme
    {
        return colors.clone();
    }
    let colors = theme_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|toml| parse(&toml))
        .unwrap_or_default();
    *cache = Some((theme, colors.clone()));
    colors
}

/// The `index`th person's colour. Without a palette file, the theme accent turned
/// around the colour wheel.
pub fn nth(index: usize, cx: &App) -> Hsla {
    let colors = colors(cx);
    if let Some(color) = colors.get(index % colors.len().max(1)) {
        return *color;
    }
    const OFFSETS: [f32; 6] = [0., 0.1, 0.2, 0.62, 0.72, 0.82];
    let mut color = cx.omarchy().accent;
    color.h = (color.h + OFFSETS[index % OFFSETS.len()]).fract();
    color
}

/// A stable colour for someone known only by name (avatars, face tiles).
pub fn for_name(name: &str, cx: &App) -> Hsla {
    let index = name.bytes().fold(0usize, |value, byte| {
        value.wrapping_mul(31).wrapping_add(byte as usize)
    });
    nth(index, cx)
}

/// Colours for everyone in a transcript: the call's participants first, in order,
/// then anyone else as they first speak. Nobody shares a colour until the palette runs
/// out.
pub struct Speakers(Vec<String>);

impl Speakers {
    pub fn new<'a>(
        participants: impl IntoIterator<Item = &'a String>,
        speakers: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let mut order: Vec<String> = Vec::new();
        for name in participants.into_iter().map(String::as_str).chain(speakers) {
            if !order.iter().any(|known| known == name) {
                order.push(name.to_owned());
            }
        }
        Self(order)
    }

    pub fn color(&self, name: &str, cx: &App) -> Hsla {
        match self.0.iter().position(|known| known == name) {
            Some(index) => nth(index, cx),
            None => for_name(name, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_people_colours_in_order_without_red_or_repeats() {
        let palette = parse(
            r##"accent = "#7aa2f7"
red = "#f7768e"
green = "#9ece6a"
blue = "#7aa2f7"
magenta = "#ad8ee6"
bright_blue = "#7aa2f7"
"##,
        );
        assert_eq!(palette.len(), 3, "red is skipped; bright_blue repeats blue");
        assert_eq!(palette[0], hex("#7aa2f7").unwrap());
        assert_eq!(palette[1], hex("#ad8ee6").unwrap());
        assert_eq!(palette[2], hex("#9ece6a").unwrap());
        assert!(parse("nonsense").is_empty());
    }

    #[test]
    fn speakers_get_their_own_slot_participants_first() {
        let participants = vec!["Alex".to_owned(), "Sam".to_owned()];
        let speakers = Speakers::new(&participants, ["Sam", "Guest", "Alex"]);
        assert_eq!(speakers.0, ["Alex", "Sam", "Guest"]);
    }
}
