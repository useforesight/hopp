//! Off the record in the Mac app: which pause the call is in and what to tell it. The
//! rules every client shares are in `hopp_capture::pause`; this is one client's view of
//! them, the same as Hopp for Linux keeps.

use std::time::{Duration, Instant};

use hopp_capture::pause::Pause;

/// What can be asked for: by saying the phrase, or with the two buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Said,
    More,
    End,
}

/// What taking a newer pause changed here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adopted {
    /// Off the record for this long from now. `began` unless it already was.
    Off {
        seq: u64,
        left: Duration,
        began: bool,
    },
    /// Back on the record.
    On,
}

#[derive(Debug, Default)]
pub struct OffRecord {
    /// The newest pause seen from anyone in the call, this client included.
    pause: Option<Pause>,
    /// When it ends on this computer's clock, while off the record.
    ends: Option<Instant>,
    /// The last one published as this client's own attribute.
    told: Option<Pause>,
}

impl OffRecord {
    pub fn is_off(&self) -> bool {
        self.ends.is_some()
    }

    /// The pause that asking for this makes, if it changes anything.
    pub fn asked(&self, ask: Ask, now_ms: i64) -> Option<Pause> {
        match ask {
            Ask::Said => Pause::said(self.pause, now_ms),
            Ask::More => Some(Pause::more(self.pause, now_ms)),
            Ask::End => self.ends.map(|_| Pause::end(self.pause)),
        }
    }

    /// Take `pause` if it's newer than the one held. The time left is worked out here,
    /// once, and counted down on this computer's own clock from then.
    pub fn adopt(&mut self, pause: Pause, now_ms: i64, now: Instant) -> Option<Adopted> {
        if self.pause >= Some(pause) {
            return None;
        }
        self.pause = Some(pause);
        let Some(left) = pause.left_ms(now_ms) else {
            return self.ends.take().map(|_| Adopted::On);
        };
        let left = Duration::from_millis(left as u64);
        let began = self.ends.is_none();
        self.ends = Some(now + left);
        Some(Adopted::Off {
            seq: pause.seq,
            left,
            began,
        })
    }

    /// The timer for pause `seq` ran out. True if that puts the call back on the record:
    /// not if a newer pause replaced it, or more time was added.
    pub fn over(&mut self, seq: u64, now: Instant) -> bool {
        let over = self.pause.is_some_and(|pause| pause.seq == seq)
            && self.ends.is_some_and(|ends| ends <= now);
        if over {
            self.ends = None;
        }
        over
    }

    /// The pause to publish as this client's own, if the call hasn't been told it yet.
    /// A client that never paused and isn't off the record has nothing to say.
    pub fn to_tell(&mut self) -> Option<Pause> {
        let pause = self.pause.filter(|pause| self.told != Some(*pause))?;
        if self.ends.is_none() && self.told.is_none() {
            return None;
        }
        self.told = Some(pause);
        Some(pause)
    }

    /// A full reconnect comes back without the attribute.
    pub fn reconnected(&mut self) {
        self.told = None;
    }

    /// The call is over here. True if it was off the record.
    pub fn leave(&mut self) -> bool {
        std::mem::take(self).ends.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hopp_capture::pause::LENGTH_MS;

    const TWO_MINUTES: Duration = Duration::from_millis(LENGTH_MS as u64);

    /// Ask, and take what that makes, as the room service does.
    fn ask(call: &mut OffRecord, ask: Ask, now_ms: i64, now: Instant) -> Option<Adopted> {
        let pause = call.asked(ask, now_ms)?;
        call.adopt(pause, now_ms, now)
    }

    #[test]
    fn saying_it_goes_off_the_record_and_tells_the_call() {
        let now = Instant::now();
        let mut call = OffRecord::default();
        assert_eq!(call.to_tell(), None, "nothing to say before any pause");
        assert_eq!(
            ask(&mut call, Ask::Said, 1_000, now),
            Some(Adopted::Off {
                seq: 1,
                left: TWO_MINUTES,
                began: true
            })
        );
        assert!(call.is_off());
        let told = call.to_tell().unwrap();
        assert_eq!(told.encode(), "1:121000");
        assert_eq!(call.to_tell(), None, "told once");
        // This client's own capture, or another's, hears the same words a moment later.
        assert_eq!(ask(&mut call, Ask::Said, 3_000, now), None);
    }

    #[test]
    fn more_time_adds_to_what_is_left() {
        let now = Instant::now();
        let mut call = OffRecord::default();
        ask(&mut call, Ask::Said, 0, now);
        let later = now + Duration::from_secs(60);
        assert_eq!(
            ask(&mut call, Ask::More, 60_000, later),
            Some(Adopted::Off {
                seq: 2,
                left: Duration::from_secs(180),
                began: false
            })
        );
        assert_eq!(call.to_tell().map(|pause| pause.seq), Some(2));
        // The first pause's timer changes nothing, the second's ends it.
        let first_ends = now + TWO_MINUTES;
        assert!(!call.over(1, first_ends));
        assert!(!call.over(2, first_ends));
        assert!(call.over(2, later + Duration::from_secs(180)));
        assert!(!call.is_off());
        assert!(!call.over(2, later + Duration::from_secs(181)), "only once");
    }

    #[test]
    fn resume_only_means_something_off_the_record() {
        let now = Instant::now();
        let mut call = OffRecord::default();
        assert_eq!(call.asked(Ask::End, 0), None);
        ask(&mut call, Ask::More, 0, now);
        call.to_tell();
        assert_eq!(ask(&mut call, Ask::End, 5_000, now), Some(Adopted::On));
        assert!(!call.is_off());
        assert_eq!(call.to_tell().unwrap().encode(), "2:0");
        assert_eq!(call.asked(Ask::End, 6_000), None);
    }

    #[test]
    fn follows_the_newest_pause_from_anyone() {
        let now = Instant::now();
        let mut call = OffRecord::default();
        let theirs = Pause::parse("3:120000").unwrap();
        assert_eq!(
            call.adopt(theirs, 0, now),
            Some(Adopted::Off {
                seq: 3,
                left: TWO_MINUTES,
                began: true
            })
        );
        // Passed on as this client's own, for whoever joins after they've left.
        assert_eq!(call.to_tell(), Some(theirs));
        // The same one again, or an older one, changes nothing.
        assert_eq!(call.adopt(theirs, 0, now), None);
        assert_eq!(call.adopt(Pause::parse("2:900000").unwrap(), 0, now), None);
        // They went back on the record.
        let back = Pause::parse("4:0").unwrap();
        assert_eq!(call.adopt(back, 1_000, now), Some(Adopted::On));
        assert_eq!(call.to_tell(), Some(back));
    }

    #[test]
    fn a_pause_that_already_ran_out_is_kept_but_not_passed_on() {
        let now = Instant::now();
        let mut call = OffRecord::default();
        // Joining a call that was off the record an hour ago.
        assert_eq!(
            call.adopt(Pause::parse("7:1000").unwrap(), 3_600_000, now),
            None
        );
        assert!(!call.is_off());
        assert_eq!(call.to_tell(), None);
        // The next one here counts on from theirs.
        assert_eq!(
            call.asked(Ask::More, 3_600_000).map(|pause| pause.seq),
            Some(8)
        );
    }

    #[test]
    fn a_reconnect_tells_the_call_again_while_off_the_record() {
        let now = Instant::now();
        let mut call = OffRecord::default();
        ask(&mut call, Ask::Said, 0, now);
        let told = call.to_tell();
        call.reconnected();
        assert_eq!(call.to_tell(), told);
        // Back on the record by the next reconnect: nothing left to pass on.
        assert!(call.over(1, now + TWO_MINUTES));
        call.reconnected();
        assert_eq!(call.to_tell(), None);
    }

    #[test]
    fn leaving_forgets_the_pause() {
        let now = Instant::now();
        let mut call = OffRecord::default();
        assert!(!call.leave());
        ask(&mut call, Ask::Said, 0, now);
        call.to_tell();
        assert!(call.leave());
        assert!(!call.is_off());
        assert_eq!(call.to_tell(), None);
        // The next call starts its own count.
        assert_eq!(call.asked(Ask::More, 0).map(|pause| pause.seq), Some(1));
    }
}
