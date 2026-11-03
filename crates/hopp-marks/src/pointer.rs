//! A remote cursor, played back a moment behind the network so it moves at an even speed.
//!
//! Updates arrive about 60 to 120 times a second, unevenly and sometimes in pairs; each is
//! given a time no closer to the previous one than half the usual gap, and the cursor is
//! drawn where the path was a little over one gap ago, moving in a straight line between
//! updates. So it never surges after a late update or stalls waiting for the next.
//!
//! It also knows how a resting cursor goes away: the name hides first, then the cursor.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// A point on the shared screen, in any linear coordinates.
pub type Point = (f64, f64);

/// The name hides after the cursor rests this long, over [`NAME_FADE`].
pub const NAME_IDLE: Duration = Duration::from_millis(2_500);
pub const NAME_FADE: Duration = Duration::from_millis(350);
/// The cursor fades after it rests this long, over [`POINTER_FADE`].
pub const POINTER_IDLE: Duration = Duration::from_secs(8);
pub const POINTER_FADE: Duration = Duration::from_millis(400);
/// A cursor resting this long has gone.
pub const POINTER_GONE: Duration = POINTER_IDLE.saturating_add(POINTER_FADE);
/// How far the name slides as it hides, in logical pixels.
pub const NAME_SLIDE: f32 = 3.;

fn progress(since: Duration, over: Duration) -> f32 {
    (since.as_secs_f32() / over.as_secs_f32()).clamp(0., 1.)
}

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

/// How opaque a cursor is after resting for `idle`.
pub fn pointer_opacity(idle: Duration) -> f32 {
    1. - progress(idle.saturating_sub(POINTER_IDLE), POINTER_FADE)
}

/// How opaque the name is after the cursor rests for `idle`. It stays while they draw.
pub fn name_opacity(idle: Duration, drawing: bool) -> f32 {
    let name = if drawing {
        1.
    } else {
        1. - progress(idle.saturating_sub(NAME_IDLE), NAME_FADE)
    };
    ease_in_out(name) * pointer_opacity(idle)
}

#[derive(Clone, Debug)]
pub struct Pointer {
    samples: VecDeque<(Instant, Point)>,
    /// The usual time between updates, in seconds.
    gap: f32,
    /// The longest recent wait between updates, in seconds, slowly forgotten.
    slow: f32,
    /// How far behind the network it plays, in seconds: `lag` at `lag_at`, moving
    /// towards `lag_to` gradually, since a sudden change would show as the cursor
    /// speeding up or slowing down.
    lag: f32,
    lag_to: f32,
    lag_at: Instant,
    arrived: Instant,
    pub moved: Instant,
}

impl Pointer {
    pub fn new(point: Point) -> Self {
        Self::new_at(point, Instant::now())
    }

    fn new_at(point: Point, now: Instant) -> Self {
        Self {
            samples: VecDeque::from([(now, point)]),
            gap: 0.016,
            slow: 0.016,
            lag: 0.035,
            lag_to: 0.035,
            lag_at: now,
            arrived: now,
            moved: now,
        }
    }

    /// Where the network last said it was.
    pub fn target(&self) -> Option<Point> {
        self.samples.back().map(|(_, point)| *point)
    }

    /// The lag at `now`: playback runs up to 10% slow while it grows and 3% fast while
    /// it shrinks.
    fn lag_at(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.lag_at).as_secs_f32();
        let rate = if self.lag_to > self.lag { 0.1 } else { 0.03 };
        self.lag + (self.lag_to - self.lag).clamp(-rate * elapsed, rate * elapsed)
    }

    fn delay(&self, now: Instant) -> Duration {
        Duration::from_secs_f32(self.lag_at(now))
    }

    /// Aim the lag at what the network needs: past the longest recent wait, so playback
    /// never catches up with the last update and stops.
    fn adjust_lag(&mut self, now: Instant) {
        self.lag = self.lag_at(now);
        self.lag_at = now;
        self.lag_to = (self.gap * 2.2).max(self.slow * 1.3).clamp(0.016, 0.09);
    }

    pub fn moved_to(&mut self, point: Point) {
        self.moved_to_at(point, Instant::now());
    }

    fn moved_to_at(&mut self, point: Point, now: Instant) {
        let since = now.duration_since(self.arrived).as_secs_f32();
        self.arrived = now;
        self.moved = now;
        let previous = self.samples.back().copied();
        if since > 0.25 || previous.is_none() {
            // It was resting: start from where it rests, one gap ago, not from whenever
            // it last moved.
            let rest = previous.map_or(point, |(_, point)| point);
            let gap = Duration::from_secs_f32(self.gap);
            self.samples.clear();
            self.samples
                .push_back((now.checked_sub(gap).unwrap_or(now), rest));
            self.samples.push_back((now, point));
            return;
        }
        self.gap += (since.clamp(0.004, 0.05) - self.gap) * 0.05;
        self.slow = (self.slow * 0.985).max(since.min(0.065));
        self.adjust_lag(now);
        // When it was probably sent: the network's jitter averaged out of the arrival
        // times (a late update and the next arriving together are still spaced out), but
        // a pause taken as it comes.
        let last = self.samples.back().map_or(now, |(at, _)| *at);
        let gap = Duration::from_secs_f32(self.gap);
        let predicted = last + gap;
        let at = if now > predicted + gap * 4 {
            now
        } else if now >= predicted {
            predicted + (now - predicted).mul_f32(0.08)
        } else {
            predicted - (predicted - now).mul_f32(0.08)
        };
        let delay = self.delay(now);
        let at = at
            .max(last + gap * 2 / 5)
            // Not so far back that playback has already passed it.
            .max(now.checked_sub(delay * 4 / 5).unwrap_or(now))
            .min(now + delay / 2);
        self.samples.push_back((at, point));
        // Keep what playback still needs.
        let shown = now.checked_sub(delay + Duration::from_millis(100));
        while self.samples.len() > 2 && shown.is_some_and(|shown| self.samples[1].0 < shown) {
            self.samples.pop_front();
        }
    }

    pub fn position(&self, now: Instant) -> Point {
        let at = now.checked_sub(self.delay(now)).unwrap_or(now);
        let Some(&(_, last)) = self.samples.back() else {
            return (0., 0.);
        };
        let mut previous: Option<(Instant, Point)> = None;
        for &(time, point) in &self.samples {
            if time > at {
                return match previous {
                    None => point,
                    Some((from_time, from)) => {
                        let span = time.duration_since(from_time).as_secs_f64();
                        let t = (at.duration_since(from_time).as_secs_f64() / span.max(1e-6))
                            .clamp(0., 1.);
                        (
                            from.0 + (point.0 - from.0) * t,
                            from.1 + (point.1 - from.1) * t,
                        )
                    }
                };
            }
            previous = Some((time, point));
        }
        last
    }

    /// Whether the cursor is still visible.
    pub fn visible(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.moved) < POINTER_GONE
    }

    /// Whether painting it again would show something different.
    pub fn animating(&self, now: Instant) -> bool {
        let idle = now.saturating_duration_since(self.moved);
        let playing = self.samples.back().is_some_and(|(at, _)| {
            now.checked_sub(self.delay(now))
                .is_none_or(|shown| shown < *at)
        });
        let settling = Duration::from_millis(50);
        playing
            || (NAME_IDLE..NAME_IDLE + NAME_FADE + settling).contains(&idle)
            || (POINTER_IDLE..POINTER_GONE + settling).contains(&idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pointer_glides_to_where_it_was_sent() {
        let start = Instant::now();
        let mut pointer = Pointer::new_at((0., 0.5), start);
        // After resting, it starts from where it rests rather than jumping.
        let moved = start + Duration::from_secs(1);
        pointer.moved_to_at((1., 0.5), moved);
        let early = pointer.position(moved).0;
        assert!(early < 0.01, "{early}");
        // And gets there once playback catches up.
        let later = moved + Duration::from_millis(200);
        assert_eq!(pointer.position(later).0, 1.);
        assert_eq!(pointer.target(), Some((1., 0.5)));
    }

    #[test]
    fn a_pointer_moves_evenly_through_uneven_updates() {
        // Steady movement sent every 16 ms or so, delivered after 0 to 20 ms (in order,
        // so a late one arrives together with the next).
        let start = Instant::now();
        let speed = 0.4; // screen widths a second
        let mut pointer = Pointer::new_at((0., 0.5), start);
        let mut seed = 7u64;
        let mut random = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 33) as f32 / (1u64 << 31) as f32
        };
        let (mut sent, mut arrived) = (0f32, 0f32);
        let mut updates = Vec::new();
        while sent < 1.5 {
            arrived = (sent + 0.02 * random() * random()).max(arrived);
            updates.push((sent, arrived));
            sent += 0.014 + 0.004 * random();
        }
        let mut next = 0;
        let mut steps = Vec::new();
        let mut last: Option<f64> = None;
        // Painted at 120 Hz.
        for frame in 0..170 {
            let now = start + Duration::from_secs_f32(frame as f32 / 120.);
            while next < updates.len() && start + Duration::from_secs_f32(updates[next].1) <= now {
                let (sent, arrived) = updates[next];
                let x = f64::from(sent * speed);
                let at = start + Duration::from_secs_f32(arrived);
                pointer.moved_to_at((x, 0.5), at);
                next += 1;
            }
            let x = pointer.position(now).0;
            // Once it's under way.
            if frame > 20 {
                if let Some(previous) = last {
                    steps.push(x - previous);
                }
            }
            last = Some(x);
        }
        let mean = steps.iter().sum::<f64>() / steps.len() as f64;
        let expected = f64::from(speed) / 120.;
        assert!(
            (mean - expected).abs() < expected * 0.1,
            "{mean} vs {expected}"
        );
        let worst = steps
            .iter()
            .map(|step| (step - mean).abs() / mean)
            .fold(0., f64::max);
        assert!(
            worst < 0.3,
            "frame-to-frame steps vary by up to {worst:.2} of the mean"
        );
        // And it changes speed gradually: no frame jolts against the one before.
        let jolt = steps
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs() / mean)
            .fold(0., f64::max);
        assert!(jolt < 0.2, "a jolt of {jolt:.2} of the mean between frames");
    }

    #[test]
    fn a_resting_cursor_loses_its_name_then_goes() {
        let at = Duration::from_millis;
        assert_eq!(name_opacity(at(0), false), 1.);
        assert_eq!(name_opacity(at(2_500), false), 1.);
        let hiding = name_opacity(at(2_675), false);
        assert!(hiding > 0. && hiding < 1., "{hiding}");
        assert_eq!(name_opacity(at(2_850), false), 0.);

        assert_eq!(pointer_opacity(at(7_999)), 1.);
        let fading = pointer_opacity(at(8_200));
        assert!(fading > 0. && fading < 1., "{fading}");
        assert_eq!(pointer_opacity(at(8_400)), 0.);
    }

    #[test]
    fn the_name_stays_while_they_draw() {
        let at = Duration::from_millis;
        assert_eq!(name_opacity(at(5_000), true), 1.);
        // Until the cursor itself goes.
        assert_eq!(name_opacity(at(8_400), true), 0.);
    }

    #[test]
    fn it_only_needs_frames_while_something_changes() {
        let start = Instant::now();
        let pointer = Pointer::new_at((0.5, 0.5), start);
        let at = |ms| start + Duration::from_millis(ms);
        assert!(!pointer.animating(at(1_000)), "resting, name showing");
        assert!(pointer.animating(at(2_600)), "name hiding");
        assert!(!pointer.animating(at(5_000)), "resting, name gone");
        assert!(pointer.animating(at(8_100)), "cursor fading");
        assert!(pointer.visible(at(8_399)));
        assert!(!pointer.visible(at(8_400)));
        assert!(!pointer.animating(at(9_000)), "gone");
    }
}
