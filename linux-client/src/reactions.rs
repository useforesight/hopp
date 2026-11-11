//! Reactions: confetti, a gong, fireworks… that play on everyone's screen in the call. The
//! engine sends and receives them (see [`crate::protocol::Reaction`]) within [`RateLimit`];
//! `ui::reactions` plays them.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::protocol::Reaction;

/// Most reactions from one person in [`WINDOW`]; the rest are dropped, so nobody can bury
/// a call in them.
const MOST: usize = 8;
const WINDOW: Duration = Duration::from_secs(3);
/// Claps come quickly, to build to an ovation, and are counted on their own.
const MOST_CLAPS: usize = 5;
const CLAP_WINDOW: Duration = Duration::from_secs(1);

/// What each person sent lately, to keep them within the limits. The Mac client keeps the
/// same ones.
#[derive(Default)]
pub struct RateLimit {
    sent: HashMap<(String, bool), VecDeque<Instant>>,
}

impl RateLimit {
    /// Whether this reaction from `sender` is within the limits, counting it if it is.
    pub fn allow(&mut self, sender: &str, reaction: &Reaction, now: Instant) -> bool {
        let clap = reaction.kind == "clap";
        let (most, window) = if clap {
            (MOST_CLAPS, CLAP_WINDOW)
        } else {
            (MOST, WINDOW)
        };
        let sender = crate::protocol::canonical_identity(sender).to_string();
        let sent = self.sent.entry((sender, clap)).or_default();
        while sent
            .front()
            .is_some_and(|at| now.duration_since(*at) >= window)
        {
            sent.pop_front();
        }
        if sent.len() >= most {
            return false;
        }
        sent.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_person_cant_flood_the_call() {
        let mut limit = RateLimit::default();
        let start = Instant::now();
        let gong = Reaction::new("gong");
        for i in 0..MOST {
            let at = start + Duration::from_millis(i as u64 * 100);
            assert!(limit.allow("a", &gong, at), "reaction {i}");
        }
        let soon = start + Duration::from_millis(900);
        assert!(!limit.allow("a", &gong, soon), "too many too quickly");
        assert!(
            !limit.allow("a:audio", &gong, soon),
            "nor from another of their connections"
        );
        assert!(limit.allow("b", &gong, soon), "someone else still can");
        assert!(
            limit.allow("a", &gong, start + WINDOW),
            "the first has left the window"
        );
    }

    #[test]
    fn claps_have_their_own_quicker_limit() {
        let mut limit = RateLimit::default();
        let start = Instant::now();
        let clap = Reaction::new("clap");
        for i in 0..MOST_CLAPS {
            assert!(limit.allow("a", &clap, start + Duration::from_millis(i as u64 * 50)));
        }
        assert!(!limit.allow("a", &clap, start + Duration::from_millis(400)));
        assert!(
            limit.allow(
                "a",
                &Reaction::new("gong"),
                start + Duration::from_millis(400)
            ),
            "claps don't use up the rest"
        );
        assert!(limit.allow("a", &clap, start + CLAP_WINDOW));
    }
}
