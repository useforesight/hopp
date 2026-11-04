//! Stamps: a numbered pin someone drops on the shared screen, with words attached.
//!
//! They go out on their own topic, [`STAMP_TOPIC`], so clients that don't know them ignore
//! them. A stamp is known by who sent it and their id for it. Numbers aren't sent: each
//! client numbers stamps in the order it saw them.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const STAMP_TOPIC: &str = "hopp_stamp";
/// The longest stamp text, in bytes.
pub const STAMP_TEXT_MAX: usize = 2_000;
/// Packets bigger than this are dropped unread.
pub const STAMP_PACKET_MAX: usize = 512 * 1024;
/// The most stamps kept in a call; the oldest goes first.
const MOST_STAMPS: usize = 64;

const DROP: Duration = Duration::from_millis(460);
const FADE: Duration = Duration::from_millis(260);
const CARD_DELAY: Duration = Duration::from_millis(180);
const CARD_REVEAL: Duration = Duration::from_millis(280);
const RING_PERIOD: f32 = 1.6;

/// How far a marker's head sits above its foot, and its radius, in logical pixels.
pub const MARKER_RISE: f32 = 17.8;
pub const MARKER_RADIUS: f32 = 11.;

pub const CARD_MAX_TEXT_WIDTH: f32 = 290.;
pub const CARD_MAX_LINES: f32 = 6.;
pub const CARD_MIN_WIDTH: f32 = 64.;
pub const CARD_PADDING: (f32, f32) = (11., 8.);
pub const CARD_RADIUS: f32 = 8.;
pub const CARD_TITLE_SIZE: f32 = 11.;
pub const CARD_TITLE_HEIGHT: f32 = 15.;
pub const CARD_TEXT_SIZE: f32 = 13.5;
pub const CARD_LINE_HEIGHT: f32 = 19.;
/// Words still being recognised, against the card's other words.
pub const DRAFT_ALPHA: f32 = 0.45;

// The variant names are the wire format's message types, so they keep their prefix.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum StampMessage {
    /// A new stamp at a point on the shared screen. `id` is the sender's own.
    StampPlace {
        id: u64,
        x: f64,
        y: f64,
    },
    /// What it says so far: final `text`, then `draft` words still being recognised.
    /// `done` once the placer has finished.
    StampText {
        id: u64,
        text: String,
        draft: String,
        done: bool,
    },
    StampRemove {
        id: u64,
    },
}

impl StampMessage {
    pub fn valid(&self) -> bool {
        match self {
            Self::StampPlace { x, y, .. } => (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y),
            Self::StampText { text, draft, .. } => text.len() + draft.len() <= STAMP_TEXT_MAX,
            Self::StampRemove { .. } => true,
        }
    }

    pub fn id(&self) -> u64 {
        match self {
            Self::StampPlace { id, .. } | Self::StampText { id, .. } | Self::StampRemove { id } => {
                *id
            }
        }
    }

    /// The stamp message in a packet from the call, unless it's too big, unreadable or invalid.
    pub fn from_packet(payload: &[u8]) -> Option<Self> {
        if payload.len() > STAMP_PACKET_MAX {
            return None;
        }
        serde_json::from_slice::<Self>(payload)
            .ok()
            .filter(Self::valid)
    }
}

/// What you did with the stamp tool.
#[derive(Debug, Clone, PartialEq)]
pub enum StampInput {
    Place {
        x: f64,
        y: f64,
    },
    Typed(String),
    Backspace,
    /// Your stamp's words are final. An empty one is removed.
    Finish,
    /// Take your unfinished stamp away.
    Cancel,
    /// Take all your stamps away.
    ClearMine,
}

/// A pin someone dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct Stamp {
    pub id: u64,
    /// Numbered through the call, in the order stamps were placed, and shown on its marker.
    pub number: u32,
    /// Participant identity; empty for yours.
    pub owner: String,
    pub name: String,
    /// On the shared screen, as fractions from the top left.
    pub point: (f64, f64),
    /// Final words.
    pub text: String,
    /// Words still being recognised.
    pub draft: String,
    /// Still taking words.
    pub listening: bool,
    pub placed: Instant,
    pub removed: Option<Instant>,
}

/// Where a stamp's animation has got to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub alpha: f32,
    /// 0 to 1 as it lands, for the shadow it casts.
    pub landing: f32,
    /// The marker's size against its full size. Overshoots a little as it lands.
    pub grow: f32,
    /// How far above its point the marker is, in logical pixels.
    pub lift: f32,
    /// How far out each breathing ring is, in logical pixels past the head's centre, and
    /// its alpha. Zero alpha when it's not taking words.
    pub rings: [(f32, f32); 2],
    /// The card's alpha, and how far back towards the pin it still is.
    pub card_alpha: f32,
    pub card_slide: f32,
}

fn progress(since: Duration, over: Duration) -> f32 {
    (since.as_secs_f32() / over.as_secs_f32()).clamp(0., 1.)
}

fn ease_out_cubic(t: f32) -> f32 {
    1. - (1. - t).powi(3)
}

fn ease_out_back(t: f32) -> f32 {
    let (c1, c3) = (1.4, 2.4);
    1. + c3 * (t - 1.).powi(3) + c1 * (t - 1.).powi(2)
}

impl Stamp {
    pub fn is_mine(&self) -> bool {
        self.owner.is_empty()
    }

    /// Faded out, so there's nothing to draw.
    pub fn gone(&self, now: Instant) -> bool {
        self.removed
            .is_some_and(|removed| now.duration_since(removed) >= FADE)
    }

    pub fn animating(&self, now: Instant) -> bool {
        !self.gone(now)
            && (self.listening
                || now.duration_since(self.placed) < DROP + CARD_REVEAL
                || self.removed.is_some())
    }

    pub fn pose(&self, now: Instant) -> Pose {
        let age = now.duration_since(self.placed);
        let leaving = self.removed.map_or(1., |removed| {
            1. - progress(now.duration_since(removed), FADE)
        });
        let landing = progress(age, DROP);
        let landed = ease_out_back(landing);
        let alpha = (landing * 3.).min(1.) * leaving;
        let grow = 0.62 + 0.38 * landed;

        let ring = |offset: f32| {
            if !self.listening {
                return (0., 0.);
            }
            let phase = (age.as_secs_f32() / RING_PERIOD + offset).fract();
            (
                MARKER_RADIUS * grow + 3. + 15. * ease_out_cubic(phase),
                0.55 * (1. - phase).powf(1.3) * alpha,
            )
        };

        let reveal = ease_out_cubic(progress(age.saturating_sub(CARD_DELAY), CARD_REVEAL));
        Pose {
            alpha,
            landing,
            grow,
            lift: 22. * (1. - landed),
            rings: [ring(0.), ring(0.5)],
            card_alpha: reveal * alpha,
            card_slide: 8. * (1. - reveal),
        }
    }

    /// What the card says, and whether it's only words still being recognised.
    pub fn words(&self) -> (String, bool) {
        let (said, draft) = (self.text.trim(), self.draft.trim());
        match (said.is_empty(), draft.is_empty()) {
            (false, false) => (format!("{said} {draft}"), false),
            (false, true) => (said.to_owned(), false),
            (true, _) => (draft.to_owned(), !draft.is_empty()),
        }
    }
}

/// Where a card goes: beside the marker's head on whichever side has room, kept 6 logical
/// pixels inside `area`. `head` and `area` are in logical pixels.
pub fn card_position(
    head: (f32, f32),
    radius: f32,
    card: (f32, f32),
    area: (f32, f32),
    slide: f32,
) -> (f32, f32) {
    const INSIDE: f32 = 6.;
    let gap = radius + 9.;
    let room_on_right = area.0 - (head.0 + gap);
    let x = if room_on_right >= card.0 + 8. {
        head.0 + gap - slide
    } else {
        (head.0 - gap - card.0 + slide).max(INSIDE)
    };
    let y = (head.1 - 22.).min(area.1 - card.1 - INSIDE).max(INSIDE);
    (x, y)
}

/// The stamps in a call, yours and everyone else's.
#[derive(Debug)]
pub struct Stamps {
    stamps: Vec<Stamp>,
    count: u32,
    last_id: u64,
    active: Option<u64>,
}

impl Default for Stamps {
    /// Ids start from the clock, so yours never repeat in a call however often your window
    /// closes and opens. Someone else still has your earlier stamps and would ignore a
    /// stamp with an id they've seen.
    fn default() -> Self {
        let last_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64);
        Self {
            stamps: Vec::new(),
            count: 0,
            last_id,
            active: None,
        }
    }
}

impl Stamps {
    pub fn iter(&self) -> impl Iterator<Item = &Stamp> {
        self.stamps.iter()
    }

    /// You've a stamp taking words, so keys go into it.
    pub fn is_typing(&self) -> bool {
        self.active.is_some()
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.stamps.iter().any(|stamp| stamp.animating(now))
    }

    /// Do what you asked, and return what everyone else needs to hear.
    pub fn apply(&mut self, input: StampInput, own_name: &str, now: Instant) -> Vec<StampMessage> {
        self.stamps.retain(|stamp| !stamp.gone(now));
        match input {
            StampInput::Place { x, y } => self.place((x, y), own_name, now),
            StampInput::Typed(typed) => self.edit(|text| {
                if text.len() + typed.len() <= STAMP_TEXT_MAX {
                    text.push_str(&typed);
                }
            }),
            StampInput::Backspace => self.edit(|text| {
                text.pop();
            }),
            StampInput::Finish => self.finish(now).into_iter().collect(),
            StampInput::Cancel => self.cancel(now).into_iter().collect(),
            StampInput::ClearMine => self.clear_mine(now),
        }
    }

    /// Someone else's stamp. `name` is used if it's a new one.
    pub fn receive(&mut self, sender: &str, name: &str, message: StampMessage, now: Instant) {
        self.stamps.retain(|stamp| !stamp.gone(now));
        let id = message.id();
        let found = self
            .stamps
            .iter()
            .position(|stamp| stamp.owner == sender && stamp.id == id);
        match (message, found) {
            (StampMessage::StampPlace { id, x, y }, None) => {
                self.count += 1;
                self.push(Stamp {
                    id,
                    number: self.count,
                    owner: sender.to_owned(),
                    name: name.to_owned(),
                    point: (x, y),
                    text: String::new(),
                    draft: String::new(),
                    listening: true,
                    placed: now,
                    removed: None,
                });
            }
            (
                StampMessage::StampText {
                    text, draft, done, ..
                },
                Some(index),
            ) => {
                let stamp = &mut self.stamps[index];
                stamp.text = text;
                stamp.draft = draft;
                stamp.listening = !done;
            }
            (StampMessage::StampRemove { .. }, Some(index)) => {
                let stamp = &mut self.stamps[index];
                stamp.listening = false;
                stamp.removed.get_or_insert(now);
            }
            _ => {}
        }
    }

    /// They've left, so their stamps won't take any more words.
    pub fn sender_left(&mut self, sender: &str) {
        for stamp in self.stamps.iter_mut().filter(|stamp| stamp.owner == sender) {
            stamp.listening = false;
        }
    }

    fn push(&mut self, stamp: Stamp) {
        if self.stamps.len() >= MOST_STAMPS {
            self.stamps.remove(0);
        }
        self.stamps.push(stamp);
    }

    fn own_active(&mut self) -> Option<&mut Stamp> {
        let id = self.active?;
        self.stamps
            .iter_mut()
            .find(|stamp| stamp.is_mine() && stamp.id == id)
    }

    fn place(&mut self, point: (f64, f64), name: &str, now: Instant) -> Vec<StampMessage> {
        let point = (point.0.clamp(0., 1.), point.1.clamp(0., 1.));
        let mut messages: Vec<StampMessage> = self.finish(now).into_iter().collect();
        self.last_id = self.last_id.wrapping_add(1);
        self.count += 1;
        let id = self.last_id;
        self.push(Stamp {
            id,
            number: self.count,
            owner: String::new(),
            name: name.to_owned(),
            point,
            text: String::new(),
            draft: String::new(),
            listening: true,
            placed: now,
            removed: None,
        });
        self.active = Some(id);
        messages.push(StampMessage::StampPlace {
            id,
            x: point.0,
            y: point.1,
        });
        messages
    }

    fn edit(&mut self, change: impl FnOnce(&mut String)) -> Vec<StampMessage> {
        let Some(stamp) = self.own_active() else {
            return Vec::new();
        };
        change(&mut stamp.text);
        let (id, text, draft) = (stamp.id, stamp.text.clone(), stamp.draft.clone());
        vec![StampMessage::StampText {
            id,
            text,
            draft,
            done: false,
        }]
    }

    fn finish(&mut self, now: Instant) -> Option<StampMessage> {
        // Pushed out by newer stamps while you typed: others may still have it, waiting.
        let Some(stamp) = self.own_active() else {
            return self.cancel(now);
        };
        stamp.text.push_str(&std::mem::take(&mut stamp.draft));
        stamp.listening = false;
        if stamp.text.trim().is_empty() {
            return self.cancel(now);
        }
        let (id, text) = (stamp.id, stamp.text.clone());
        self.active = None;
        Some(StampMessage::StampText {
            id,
            text,
            draft: String::new(),
            done: true,
        })
    }

    fn cancel(&mut self, now: Instant) -> Option<StampMessage> {
        let id = self.active?;
        if let Some(stamp) = self.own_active() {
            stamp.listening = false;
            stamp.removed = Some(now);
        }
        self.active = None;
        Some(StampMessage::StampRemove { id })
    }

    fn clear_mine(&mut self, now: Instant) -> Vec<StampMessage> {
        self.active = None;
        self.stamps
            .iter_mut()
            .filter(|stamp| stamp.is_mine() && stamp.removed.is_none())
            .map(|stamp| {
                stamp.listening = false;
                stamp.removed = Some(now);
                StampMessage::StampRemove { id: stamp.id }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_ms(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    fn place(stamps: &mut Stamps, now: Instant) -> u64 {
        let messages = stamps.apply(StampInput::Place { x: 0.5, y: 0.5 }, "You", now);
        messages.last().expect("a place").id()
    }

    /// The wire format the Linux client speaks, from the Mac port guide.
    #[test]
    fn messages_match_the_other_clients() {
        let cases = [
            (
                StampMessage::StampPlace {
                    id: 3,
                    x: 0.62,
                    y: 0.41,
                },
                r#"{"type":"StampPlace","payload":{"id":3,"x":0.62,"y":0.41}}"#,
            ),
            (
                StampMessage::StampText {
                    id: 3,
                    text: "SAVE10 goes in here".into(),
                    draft: String::new(),
                    done: false,
                },
                r#"{"type":"StampText","payload":{"id":3,"text":"SAVE10 goes in here","draft":"","done":false}}"#,
            ),
            (
                StampMessage::StampRemove { id: 3 },
                r#"{"type":"StampRemove","payload":{"id":3}}"#,
            ),
        ];
        for (message, wire) in cases {
            assert_eq!(serde_json::to_string(&message).unwrap(), wire);
            assert_eq!(StampMessage::from_packet(wire.as_bytes()), Some(message));
        }
    }

    #[test]
    fn bad_packets_are_dropped() {
        let outside = br#"{"type":"StampPlace","payload":{"id":3,"x":1.2,"y":0.4}}"#;
        assert_eq!(StampMessage::from_packet(outside), None);
        assert_eq!(StampMessage::from_packet(b"not json"), None);
        let long = StampMessage::StampText {
            id: 1,
            text: "a".repeat(STAMP_TEXT_MAX),
            draft: "b".into(),
            done: false,
        };
        let bytes = serde_json::to_vec(&long).unwrap();
        assert_eq!(StampMessage::from_packet(&bytes), None);
        let huge = vec![b' '; STAMP_PACKET_MAX + 1];
        assert_eq!(StampMessage::from_packet(&huge), None);
    }

    #[test]
    fn typing_then_enter_finishes_it() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        let id = place(&mut stamps, now);
        assert!(stamps.is_typing());

        stamps.apply(StampInput::Typed("Hi".into()), "You", now);
        let sent = stamps.apply(StampInput::Typed("!".into()), "You", now);
        assert_eq!(
            sent,
            [StampMessage::StampText {
                id,
                text: "Hi!".into(),
                draft: String::new(),
                done: false,
            }],
            "every key sends the whole text"
        );
        let sent = stamps.apply(StampInput::Backspace, "You", now);
        assert!(matches!(&sent[0], StampMessage::StampText { text, .. } if text == "Hi"));

        let sent = stamps.apply(StampInput::Finish, "You", now);
        assert_eq!(
            sent,
            [StampMessage::StampText {
                id,
                text: "Hi".into(),
                draft: String::new(),
                done: true,
            }]
        );
        assert!(!stamps.is_typing());
        assert!(!stamps.iter().next().unwrap().listening);
    }

    #[test]
    fn keys_do_nothing_without_a_stamp_taking_words() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        assert!(stamps
            .apply(StampInput::Typed("a".into()), "You", now)
            .is_empty());
        assert!(stamps.apply(StampInput::Finish, "You", now).is_empty());
        assert!(stamps.apply(StampInput::Cancel, "You", now).is_empty());
    }

    #[test]
    fn placing_another_finishes_the_last() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        let first = place(&mut stamps, now);
        stamps.apply(StampInput::Typed("One".into()), "You", now);
        let sent = stamps.apply(StampInput::Place { x: 0.1, y: 0.1 }, "You", now);
        assert_eq!(sent.len(), 2);
        assert!(matches!(
            &sent[0],
            StampMessage::StampText { id, done: true, .. } if *id == first
        ));
        assert!(matches!(&sent[1], StampMessage::StampPlace { .. }));
        let numbers: Vec<u32> = stamps.iter().map(|stamp| stamp.number).collect();
        assert_eq!(numbers, [1, 2]);
    }

    #[test]
    fn an_empty_stamp_disappears_when_finished() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        let id = place(&mut stamps, now);
        stamps.apply(StampInput::Typed("   ".into()), "You", now);
        let sent = stamps.apply(StampInput::Finish, "You", now);
        assert_eq!(sent, [StampMessage::StampRemove { id }]);
        assert!(!stamps.is_typing());
        assert!(stamps.iter().next().unwrap().gone(at_ms(now, 300)));
    }

    #[test]
    fn escape_takes_your_stamp_away() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        let id = place(&mut stamps, now);
        stamps.apply(StampInput::Typed("Oops".into()), "You", now);
        let sent = stamps.apply(StampInput::Cancel, "You", now);
        assert_eq!(sent, [StampMessage::StampRemove { id }]);
        assert!(!stamps.is_typing());
    }

    #[test]
    fn right_click_clears_only_yours() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        let mine = place(&mut stamps, now);
        stamps.apply(StampInput::Typed("Mine".into()), "You", now);
        stamps.apply(StampInput::Finish, "You", now);
        stamps.receive(
            "them",
            "Mira",
            StampMessage::StampPlace {
                id: 9,
                x: 0.2,
                y: 0.2,
            },
            now,
        );
        let sent = stamps.apply(StampInput::ClearMine, "You", now);
        assert_eq!(sent, [StampMessage::StampRemove { id: mine }]);
        let later = at_ms(now, 300);
        let kept: Vec<_> = stamps.iter().filter(|s| !s.gone(later)).collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].owner, "them");
    }

    #[test]
    fn text_stops_at_the_limit() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        place(&mut stamps, now);
        let full = "a".repeat(STAMP_TEXT_MAX);
        stamps.apply(StampInput::Typed(full), "You", now);
        let sent = stamps.apply(StampInput::Typed("b".into()), "You", now);
        assert!(sent[0].valid());
        assert_eq!(stamps.iter().next().unwrap().text.len(), STAMP_TEXT_MAX);
    }

    #[test]
    fn a_stamp_is_known_by_its_sender_and_id() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        let place_at = |id| StampMessage::StampPlace { id, x: 0.3, y: 0.3 };
        stamps.receive("a", "Ann", place_at(1), now);
        stamps.receive("b", "Bo", place_at(1), now);
        stamps.receive("a", "Ann", place_at(1), now);
        assert_eq!(stamps.iter().count(), 2, "the same again changes nothing");

        let text = StampMessage::StampText {
            id: 1,
            text: "Mine".into(),
            draft: String::new(),
            done: true,
        };
        stamps.receive("b", "Bo", text, now);
        let by: Vec<(&str, &str, bool)> = stamps
            .iter()
            .map(|s| (s.owner.as_str(), s.text.as_str(), s.listening))
            .collect();
        assert_eq!(by, [("a", "", true), ("b", "Mine", false)]);

        stamps.receive("a", "Ann", StampMessage::StampRemove { id: 1 }, now);
        assert!(stamps.iter().next().unwrap().removed.is_some());
        let unknown = StampMessage::StampRemove { id: 77 };
        stamps.receive("c", "Cy", unknown, now);
        assert_eq!(stamps.iter().count(), 2);
    }

    /// A new window has a new `Stamps`, and the id must still be one nobody's seen.
    #[test]
    fn a_new_window_does_not_reuse_ids() {
        let now = Instant::now();
        let first = place(&mut Stamps::default(), now);
        std::thread::sleep(Duration::from_millis(3));
        let second = place(&mut Stamps::default(), now);
        assert!(second > first);
    }

    #[test]
    fn the_oldest_goes_when_there_are_too_many() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        for id in 0..(MOST_STAMPS as u64 + 3) {
            let message = StampMessage::StampPlace { id, x: 0.5, y: 0.5 };
            stamps.receive("a", "Ann", message, now);
        }
        assert_eq!(stamps.iter().count(), MOST_STAMPS);
        assert_eq!(stamps.iter().next().unwrap().id, 3);
        assert_eq!(stamps.iter().last().unwrap().number, MOST_STAMPS as u32 + 3);
    }

    #[test]
    fn finishing_a_stamp_pushed_out_by_newer_ones_tells_everyone() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        let mine = place(&mut stamps, now);
        for id in 0..MOST_STAMPS as u64 {
            let message = StampMessage::StampPlace { id, x: 0.5, y: 0.5 };
            stamps.receive("a", "Ann", message, now);
        }
        assert!(stamps.iter().all(|stamp| !stamp.is_mine()));
        let sent = stamps.apply(StampInput::Finish, "You", now);
        assert_eq!(sent, [StampMessage::StampRemove { id: mine }]);
        assert!(!stamps.is_typing());
    }

    #[test]
    fn stamps_stop_listening_when_their_sender_leaves() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        let message = StampMessage::StampPlace {
            id: 1,
            x: 0.5,
            y: 0.5,
        };
        stamps.receive("a", "Ann", message, now);
        assert!(stamps.animating(at_ms(now, 60_000)), "still taking words");
        stamps.sender_left("a");
        assert!(!stamps.animating(at_ms(now, 60_000)));
    }

    #[test]
    fn it_drops_in_with_a_bounce_then_rests() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        place(&mut stamps, now);
        let stamp = stamps.iter().next().unwrap();

        let start = stamp.pose(now);
        assert_eq!(start.alpha, 0.);
        assert!((start.grow - 0.62).abs() < 0.001, "{}", start.grow);
        assert!((start.lift - 22.).abs() < 0.001, "{}", start.lift);

        let peak = (0..46)
            .map(|step| stamp.pose(at_ms(now, step * 10)).grow)
            .fold(0., f32::max);
        assert!(peak > 1.01, "overshoots: {peak}");

        let landed = stamp.pose(at_ms(now, 500));
        assert!((landed.grow - 1.).abs() < 0.001);
        assert!(landed.lift.abs() < 0.01);
        assert_eq!(landed.alpha, 1.);
        assert!(landed.card_alpha > 0.9 && landed.card_slide < 1.);
    }

    #[test]
    fn it_breathes_only_while_taking_words() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        place(&mut stamps, now);
        let listening = stamps.iter().next().unwrap().pose(at_ms(now, 1_000));
        assert!(listening.rings.iter().all(|(_, alpha)| *alpha > 0.));
        assert_ne!(
            listening.rings[0], listening.rings[1],
            "half a period apart"
        );

        stamps.apply(StampInput::Typed("Hi".into()), "You", now);
        stamps.apply(StampInput::Finish, "You", now);
        let finished = stamps.iter().next().unwrap();
        assert!(finished
            .pose(at_ms(now, 1_000))
            .rings
            .iter()
            .all(|(_, alpha)| *alpha == 0.));
        assert!(!finished.animating(at_ms(now, 1_000)));
    }

    #[test]
    fn it_fades_when_removed() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        place(&mut stamps, now);
        let removed = at_ms(now, 1_000);
        stamps.apply(StampInput::Cancel, "You", removed);
        let stamp = stamps.iter().next().unwrap();
        let halfway = stamp.pose(at_ms(now, 1_130));
        assert!((halfway.alpha - 0.5).abs() < 0.01, "{}", halfway.alpha);
        assert!(stamp.animating(at_ms(now, 1_130)));
        assert!(stamp.gone(at_ms(now, 1_260)));
        assert!(!stamp.animating(at_ms(now, 1_260)));

        // The next thing you do lets go of it.
        stamps.apply(StampInput::Finish, "You", at_ms(now, 2_000));
        assert_eq!(stamps.iter().count(), 0);
    }

    #[test]
    fn draft_words_follow_final_ones() {
        let now = Instant::now();
        let mut stamps = Stamps::default();
        let text = |text: &str, draft: &str| StampMessage::StampText {
            id: 1,
            text: text.into(),
            draft: draft.into(),
            done: false,
        };
        let place = StampMessage::StampPlace {
            id: 1,
            x: 0.5,
            y: 0.5,
        };
        stamps.receive("a", "Ann", place, now);
        let words = |stamps: &Stamps| stamps.iter().next().unwrap().words();
        assert_eq!(words(&stamps), (String::new(), false));
        stamps.receive("a", "Ann", text("", "maybe "), now);
        assert_eq!(words(&stamps), ("maybe".into(), true));
        stamps.receive("a", "Ann", text("Sure", "maybe"), now);
        assert_eq!(words(&stamps), ("Sure maybe".into(), false));
    }

    #[test]
    fn the_card_stays_on_screen() {
        let area = (800., 600.);
        let card = (200., 80.);
        // Room on the right.
        assert_eq!(
            card_position((100., 100.), 11., card, area, 0.),
            (120., 78.)
        );
        // None, so it goes on the left.
        assert_eq!(
            card_position((700., 100.), 11., card, area, 0.),
            (480., 78.)
        );
        // Kept inside the top and the bottom.
        assert_eq!(card_position((100., 10.), 11., card, area, 0.).1, 6.);
        assert_eq!(card_position((100., 590.), 11., card, area, 0.).1, 514.);
        // On a screen too small for it either side, it still starts inside.
        assert_eq!(
            card_position((100., 100.), 11., card, (150., 600.), 0.).0,
            6.
        );
        // It slides out from the pin, on either side.
        assert_eq!(card_position((100., 100.), 11., card, area, 8.).0, 112.);
        assert_eq!(card_position((700., 100.), 11., card, area, 8.).0, 488.);
    }
}
