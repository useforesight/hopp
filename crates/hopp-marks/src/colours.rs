//! Ink colours: one per person, the same on every machine and in captured frames.
//!
//! A person's preferred colour comes from their Hopp user id, so it stays the same from
//! call to call. When two people in a call would share one, the second gets the next free
//! colour; [`Colours::for_call`] works out who that is from who's in the call and nothing
//! else, so every machine gets the same answer.

use std::collections::{BTreeSet, HashMap};

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

/// A participant identity without the suffix of the connection it came from.
pub fn canonical_identity(identity: &str) -> &str {
    identity
        .strip_suffix(":audio")
        .or_else(|| identity.strip_suffix(":video"))
        .or_else(|| identity.strip_suffix(":camera"))
        .unwrap_or(identity)
}

/// The Hopp user id in a participant identity (`room:<room>:<user>:audio`), or the
/// identity itself when it isn't one.
pub fn person(identity: &str) -> &str {
    let id = canonical_identity(identity);
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
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Colours(HashMap<String, [u8; 3]>);

impl Colours {
    /// Everyone's colours in a call, from who's in it, in any order. Whoever's user id sorts
    /// first keeps their colour when two people want the same one. Everyone gets the same
    /// answer this way. Starting from yourself would give each person the colour they both
    /// want on their own machine, which would disagree in most calls of three or more.
    pub fn for_call<'a>(people: impl IntoIterator<Item = &'a str>) -> Self {
        let people: BTreeSet<&str> = people.into_iter().map(person).collect();
        let mut colours = Self::default();
        for person in people {
            colours.of(person);
        }
        colours
    }

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

/// `a` moved towards `b` by `t`, from 0 to 1.
pub fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    std::array::from_fn(|i| {
        (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8
    })
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
        assert_eq!(colour("room:a:u1:audio"), colour("room:a:u1:camera"));
        assert_eq!(
            colour("u1"),
            colour("room:a:u1:audio"),
            "the UI knows only your user id"
        );
    }

    /// Every client has to hash the same way, or they won't agree on anyone's colour.
    #[test]
    fn the_hash_does_not_change() {
        assert_eq!(hex(colour("usr_abc")), "#64d2ff");
        assert_eq!(hex(colour("room:r1:usr_abc:audio")), "#64d2ff");
        assert_eq!(hex(colour("usr_xyz")), "#ff375f");
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

    /// Whoever's looking, and in whatever order they saw people join.
    #[test]
    fn every_machine_agrees_on_everyone() {
        let ids: Vec<String> = (0..40).map(|i| format!("usr_{i}")).collect();
        let mut clashes = 0;
        for a in &ids {
            for b in &ids {
                if a == b {
                    continue;
                }
                let on_mine = Colours::for_call([a.as_str(), b.as_str()]);
                let on_theirs = Colours::for_call([b.as_str(), a.as_str()]);
                assert_eq!(on_mine, on_theirs);
                assert_ne!(on_mine.get(a), on_mine.get(b), "{a} and {b}");
                if colour(a) == colour(b) {
                    clashes += 1;
                }
            }
        }
        assert!(clashes > 0, "the ids should include people who clash");

        let people = [
            "room:r:usr_1:audio",
            "room:r:usr_2:audio",
            "room:r:usr_3:video",
            "room:r:usr_4:camera",
        ];
        let reversed: Vec<&str> = people.iter().rev().copied().collect();
        assert_eq!(Colours::for_call(people), Colours::for_call(reversed));
    }

    #[test]
    fn two_connections_of_one_person_are_one_person() {
        let colours = Colours::for_call(["room:r:u1:audio", "room:r:u1:video"]);
        assert_eq!(colours.get("u1"), Some(colour("u1")));
        assert_eq!(colours.0.len(), 1);
    }
}
