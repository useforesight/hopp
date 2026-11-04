//! Saying a stamp's note: which window's stamp it's for, whether you held it or only clicked it,
//! and whether it unmuted you. What's heard is `hopp_capture::heard`'s.

use std::time::{Duration, Instant};

use hopp_capture::heard::StampListener;

/// Holding the stamp down this long records what you say until you let go. A shorter click goes
/// on hearing you until you pause, if your microphone's on.
const HOLD: Duration = Duration::from_millis(350);

/// Whose stamps you're holding one of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum StampHolder {
    /// On a screen someone shares with you: it's in that window.
    #[default]
    Viewer,
    /// On your own screen: it's on the overlay, with everyone's.
    Sharer,
}

#[derive(Default)]
pub(crate) struct StampVoice {
    /// Counts presses, so that what's heard for an earlier one is ignored.
    presses: u64,
    holder: StampHolder,
    pressed: Option<Instant>,
    listener: Option<StampListener>,
    unmuted: bool,
}

impl StampVoice {
    /// You pressed: whatever the last press was hearing is over. Returns this press's number.
    pub(crate) fn press(&mut self, holder: StampHolder, now: Instant) -> u64 {
        self.listener = None;
        self.holder = holder;
        self.pressed = Some(now);
        self.presses += 1;
        self.presses
    }

    pub(crate) fn hear_with(&mut self, listener: StampListener) {
        self.listener = Some(listener);
    }

    pub(crate) fn set_unmuted(&mut self) {
        self.unmuted = true;
    }

    /// You let go. Returns whether that was a click with your microphone muted, so it can't hear
    /// you: a click isn't to leave you unmuted for who knows how long. Otherwise the listener
    /// goes on until your words are written down.
    pub(crate) fn release(&mut self, now: Instant) -> bool {
        let Some(pressed) = self.pressed.take() else {
            return false;
        };
        let Some(listener) = &self.listener else {
            return false;
        };
        if now.duration_since(pressed) >= HOLD {
            listener.let_go();
            return false;
        }
        if self.unmuted {
            self.listener = None;
            return true;
        }
        listener.clicked();
        false
    }

    pub(crate) fn holder(&self) -> StampHolder {
        self.holder
    }

    /// There's something to end if its stamp goes.
    pub(crate) fn is_on(&self) -> bool {
        self.pressed.is_some() || self.listener.is_some() || self.unmuted
    }

    /// Something's heard for this press, and it's still the one listening.
    pub(crate) fn is_hearing(&self, press: u64) -> bool {
        self.presses == press && self.listener.is_some()
    }

    /// It's heard all it will. You may still be holding it down.
    pub(crate) fn stop_hearing(&mut self) {
        self.listener = None;
    }

    pub(crate) fn end(&mut self) {
        self.listener = None;
        self.pressed = None;
    }

    /// Whether it unmuted you and that hasn't been given back.
    pub(crate) fn take_unmuted(&mut self) -> bool {
        std::mem::take(&mut self.unmuted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn after(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    /// Its thread finds no database here and stops, which none of this minds.
    fn listener() -> StampListener {
        StampListener::start(
            PathBuf::from("/dev/null/hopp-stamp-voice-test/index.db"),
            "c".into(),
            "me".into(),
            0,
            |_| {},
        )
        .unwrap()
    }

    #[test]
    fn a_click_goes_on_hearing_you_with_your_microphone_on() {
        let start = Instant::now();
        let mut voice = StampVoice::default();
        let press = voice.press(StampHolder::Sharer, start);
        voice.hear_with(listener());
        assert!(voice.is_hearing(press));
        assert!(!voice.release(after(start, 349)));
        assert!(voice.is_hearing(press));
        assert!(voice.is_on());
    }

    #[test]
    fn a_click_with_your_microphone_muted_gives_it_back_and_leaves_typing() {
        let start = Instant::now();
        let mut voice = StampVoice::default();
        let press = voice.press(StampHolder::Sharer, start);
        voice.hear_with(listener());
        voice.set_unmuted();
        assert!(voice.release(after(start, 349)));
        assert!(!voice.is_hearing(press), "it's for typing now");
        assert!(voice.take_unmuted());
        assert!(!voice.is_on());
    }

    #[test]
    fn a_long_press_goes_on_hearing_after_you_let_go() {
        let start = Instant::now();
        let mut voice = StampVoice::default();
        let press = voice.press(StampHolder::Viewer, start);
        voice.hear_with(listener());
        assert!(!voice.release(after(start, 350)));
        assert!(voice.is_hearing(press));
        assert!(voice.is_on());
        voice.stop_hearing();
        assert!(!voice.is_on());
    }

    #[test]
    fn with_no_capture_a_press_is_only_a_press() {
        let start = Instant::now();
        let mut voice = StampVoice::default();
        let press = voice.press(StampHolder::Viewer, start);
        assert!(!voice.is_hearing(press));
        assert!(!voice.release(after(start, 5_000)), "nothing to stop");
        assert!(!voice.release(after(start, 5_001)), "only once");
    }

    #[test]
    fn what_was_heard_for_an_earlier_press_is_ignored() {
        let start = Instant::now();
        let mut voice = StampVoice::default();
        let first = voice.press(StampHolder::Viewer, start);
        voice.hear_with(listener());
        voice.release(after(start, 1_000));
        let second = voice.press(StampHolder::Sharer, after(start, 2_000));
        voice.hear_with(listener());
        assert!(!voice.is_hearing(first));
        assert!(voice.is_hearing(second));
        assert_eq!(voice.holder(), StampHolder::Sharer);
    }

    #[test]
    fn the_microphone_is_given_back_once() {
        let start = Instant::now();
        let mut voice = StampVoice::default();
        voice.press(StampHolder::Viewer, start);
        voice.set_unmuted();
        voice.end();
        assert!(voice.is_on(), "the microphone's still to give back");
        assert!(voice.take_unmuted());
        assert!(!voice.take_unmuted());
        assert!(!voice.is_on());
    }
}
