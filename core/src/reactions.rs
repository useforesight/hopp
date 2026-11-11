//! Reactions: confetti, a gong, fireworks… that play on everyone's screen in the call. The
//! core sends and receives them (see `socket_lib::Reaction`); the app plays them.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use socket_lib::Reaction;
use winit::window::Window;

/// Most reactions from one person in [`WINDOW`]; the rest are dropped, so nobody can bury
/// a call in them.
const MOST: usize = 8;
const WINDOW: Duration = Duration::from_secs(3);
/// Claps come quickly, to build to an ovation, and are counted on their own.
const MOST_CLAPS: usize = 5;
const CLAP_WINDOW: Duration = Duration::from_secs(1);

/// What each person sent lately, to keep them within the limits.
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
        let sent = self.sent.entry((sender.to_string(), clap)).or_default();
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

/// A window's frame in points from the top left of the main screen: x, y, width, height.
pub fn window_frame(window: &Window) -> Option<[f64; 4]> {
    if !window.is_visible().unwrap_or(true) || window.is_minimized().unwrap_or(false) {
        return None;
    }
    let scale = window.scale_factor();
    let position = window.outer_position().ok()?.to_logical::<f64>(scale);
    let size = window.outer_size().to_logical::<f64>(scale);
    Some([position.x, position.y, size.width, size.height])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reaction(kind: &str) -> Reaction {
        Reaction {
            kind: kind.to_string(),
            choice: None,
            level: 0,
            text: None,
            to: None,
        }
    }

    #[test]
    fn one_person_cant_flood_the_call() {
        let mut limit = RateLimit::default();
        let start = Instant::now();
        let gong = reaction("gong");
        for i in 0..MOST {
            let at = start + Duration::from_millis(i as u64 * 100);
            assert!(limit.allow("a", &gong, at), "reaction {i}");
        }
        let soon = start + Duration::from_millis(900);
        assert!(!limit.allow("a", &gong, soon), "too many too quickly");
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
        let clap = reaction("clap");
        for i in 0..MOST_CLAPS {
            assert!(limit.allow("a", &clap, start + Duration::from_millis(i as u64 * 50)));
        }
        assert!(!limit.allow("a", &clap, start + Duration::from_millis(400)));
        assert!(
            limit.allow("a", &reaction("gong"), start + Duration::from_millis(400)),
            "claps don't use up the rest"
        );
        assert!(limit.allow("a", &clap, start + CLAP_WINDOW));
    }
}
