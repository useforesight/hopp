//! Who is speaking, as the call tiles show it.
//!
//! LiveKit reports the speakers each time the set changes, and someone pausing between
//! words briefly drops out of it. Each person stays lit for [`HOLD`] after they were last
//! reported, so a name doesn't flicker on every breath.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// How long someone stays lit after they were last heard.
pub const HOLD: Duration = Duration::from_millis(300);

/// Tile key for you, as `faces::faces_for` uses it.
pub const YOU: &str = "you";

#[derive(Debug, Default)]
pub struct Speaking {
    /// Speaking now, by canonical identity (or [`YOU`]).
    current: Vec<String>,
    /// Stopped speaking, and when.
    stopped: HashMap<String, Instant>,
}

impl Speaking {
    /// The latest speakers. Returns how long until someone's hold ends and the tiles
    /// need drawing again, if anyone has just stopped.
    pub fn update(&mut self, speakers: Vec<String>, now: Instant) -> Option<Duration> {
        for key in std::mem::take(&mut self.current) {
            if !speakers.contains(&key) {
                self.stopped.insert(key, now);
            }
        }
        for key in &speakers {
            self.stopped.remove(key);
        }
        self.current = speakers;
        self.next_expiry(now)
    }

    pub fn is_speaking(&self, key: &str, now: Instant) -> bool {
        self.current.iter().any(|current| current == key)
            || self
                .stopped
                .get(key)
                .is_some_and(|at| now.saturating_duration_since(*at) < HOLD)
    }

    /// Lets go of holds that have ended; returns whether any did.
    pub fn expire(&mut self, now: Instant) -> bool {
        let before = self.stopped.len();
        self.stopped
            .retain(|_, at| now.saturating_duration_since(*at) < HOLD);
        before != self.stopped.len()
    }

    fn next_expiry(&self, now: Instant) -> Option<Duration> {
        self.stopped
            .values()
            .map(|at| HOLD.saturating_sub(now.saturating_duration_since(*at)))
            .min()
    }

    pub fn clear(&mut self) {
        self.current.clear();
        self.stopped.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn a_speaker_stays_lit_briefly_after_they_stop() {
        let start = Instant::now();
        let mut speaking = Speaking::default();
        assert_eq!(speaking.update(vec!["mira".into()], start), None);
        assert!(speaking.is_speaking("mira", start));
        assert!(!speaking.is_speaking("jonas", start));
        let wait = speaking.update(vec![], start + ms(1000));
        assert_eq!(wait, Some(HOLD));
        assert!(speaking.is_speaking("mira", start + ms(1299)));
        assert!(!speaking.is_speaking("mira", start + ms(1300)));
        assert!(!speaking.expire(start + ms(1200)));
        assert!(speaking.expire(start + ms(1300)));
    }

    #[test]
    fn speaking_again_within_the_hold_carries_on_without_a_gap() {
        let start = Instant::now();
        let mut speaking = Speaking::default();
        speaking.update(vec![YOU.into()], start);
        speaking.update(vec![], start + ms(100));
        assert_eq!(speaking.update(vec![YOU.into()], start + ms(200)), None);
        assert!(speaking.is_speaking(YOU, start + ms(900)));
    }
}
