//! Ink colours: one per person, the same in the viewer and in captured frames.
//!
//! A person's preferred colour comes from their Hopp user id, so it stays the same from
//! call to call. When two people in a call would share one, the second gets the next free
//! colour; [`Colours`] hands them out in the order people first draw.

use std::collections::HashMap;

/// Distinct on light and dark screens.
pub const PALETTE: [[u8; 3]; 8] = [
    [0xff, 0x3b, 0x30], // red
    [0x0a, 0x84, 0xff], // blue
    [0x30, 0xd1, 0x58], // green
    [0xff, 0x9f, 0x0a], // orange
    [0xbf, 0x5a, 0xf2], // purple
    [0xff, 0x37, 0x5f], // pink
    [0x64, 0xd2, 0xff], // cyan
    [0xff, 0xd6, 0x0a], // yellow
];

/// The Hopp user id in a participant identity (`room:<room>:<user>:audio`), or the
/// identity itself when it isn't one.
pub fn person(identity: &str) -> &str {
    let id = crate::protocol::canonical_identity(identity);
    id.strip_prefix("room:")
        .and_then(|rest| rest.split(':').nth(1))
        .unwrap_or(id)
}

/// A person's preferred colour, from their user id or participant identity.
pub fn colour(identity: &str) -> [u8; 3] {
    // FNV-1a: stable across runs and builds, unlike the standard hasher.
    let hash = person(identity)
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        });
    PALETTE[(hash % PALETTE.len() as u64) as usize]
}

/// Colours given out in one call.
#[derive(Default, Clone)]
pub struct Colours(HashMap<String, [u8; 3]>);

impl Colours {
    /// This person's colour: their own unless someone already has it, then the next free.
    pub fn of(&mut self, identity: &str) -> [u8; 3] {
        let who = person(identity).to_owned();
        if let Some(colour) = self.0.get(&who) {
            return *colour;
        }
        let preferred = colour(&who);
        let start = PALETTE.iter().position(|c| *c == preferred).unwrap_or(0);
        let candidates = || (0..PALETTE.len()).map(|step| PALETTE[(start + step) % PALETTE.len()]);
        let taken = |c: &[u8; 3]| self.0.values().any(|taken| taken == c);
        let close = |c: &[u8; 3]| self.0.values().any(|taken| similar(*taken, *c));
        // Free and unlike anyone else's, or else just free.
        let chosen = candidates()
            .find(|c| !taken(c) && !close(c))
            .or_else(|| candidates().find(|c| !taken(c)))
            .unwrap_or(preferred);
        self.0.insert(who, chosen);
        chosen
    }

    /// The colour already given to this person, if any.
    pub fn get(&self, identity: &str) -> Option<[u8; 3]> {
        self.0.get(person(identity)).copied()
    }
}

/// Palette colours that are easy to mix up at a glance on a busy screen.
fn similar(a: [u8; 3], b: [u8; 3]) -> bool {
    const PAIRS: [(usize, usize); 3] = [(1, 6), (0, 5), (3, 7)]; // blue/cyan, red/pink, orange/yellow
    let index = |c: [u8; 3]| PALETTE.iter().position(|p| *p == c);
    match (index(a), index(b)) {
        (Some(a), Some(b)) => PAIRS.contains(&(a, b)) || PAIRS.contains(&(b, a)),
        _ => false,
    }
}

pub fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_person_has_one_colour_whatever_the_track() {
        assert_eq!(colour("room:a:u1:audio"), colour("room:a:u1:video"));
        assert_eq!(
            colour("u1"),
            colour("room:a:u1:audio"),
            "the UI knows only your user id"
        );
    }

    #[test]
    fn people_in_one_call_get_different_colours() {
        let mut colours = Colours::default();
        let all: Vec<_> = (0..8).map(|i| colours.of(&format!("person-{i}"))).collect();
        for (i, a) in all.iter().enumerate() {
            assert!(!all[i + 1..].contains(a), "{all:?}");
        }
        assert_eq!(colours.of("person-3"), all[3]);
        // With room to spare, nobody gets a colour like someone else's.
        let mut few = Colours::default();
        let four: Vec<_> = (0..4).map(|i| few.of(&format!("p{i}"))).collect();
        for (i, a) in four.iter().enumerate() {
            assert!(four[i + 1..].iter().all(|b| !similar(*a, *b)), "{four:?}");
        }
        assert_eq!(colours.of("room:r:person-3:audio"), all[3]);
    }
}
