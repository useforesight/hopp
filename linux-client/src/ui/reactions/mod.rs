//! Reactions played over the whole screen, with sound: confetti, a gong, a flipped table,
//! emoji, the soundboard, a slow clap, fireworks, achievements, disco and ship it (see
//! [`crate::reactions`] for how they travel).
//!
//! A [`Player`] runs the effects on a [`stage::Stage`] and turns them into a
//! [`scene::Frame`] to paint; [`layer`] paints it on a click-through layer over the
//! monitor with the call window, open only while something plays. The sounds are made by
//! [`synth`] and played by [`sound`]. The tray of reactions to send is [`tray`].

pub mod effects;
pub mod layer;
pub mod looks;
pub mod particles;
pub mod scene;
pub mod sound;
pub mod stage;
pub mod synth;
pub mod tray;

use std::time::{Duration, Instant};

use gpui_kit::{Context, EventEmitter};

use crate::protocol::Reaction;
use effects::Effect;
use scene::{Canvas, Frame};
use stage::{Clock, Stage, Who};

/// Nothing has played for this long and nothing's left on screen: the layer can go.
pub const IDLE_AFTER: f32 = 12.;
/// Whatever's still on screen by now, it goes.
pub const GONE_AFTER: f32 = 45.;
/// Particles move in 60 Hz steps, as the prototypes' do.
const TICK: f32 = 1. / 60.;

/// A reaction to play: who sent it, and who's in the call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Played {
    /// The sender's canonical identity.
    pub sender: String,
    pub name: String,
    pub rgb: [u8; 3],
    pub mine: bool,
    pub reaction: Reaction,
    /// Everyone else in the call: identity, name and colour.
    pub people: Vec<(String, String, [u8; 3])>,
    /// You: identity, name and colour.
    pub me: (String, String, [u8; 3]),
}

struct Running {
    kind: String,
    started: f32,
    before: f32,
    effect: Box<dyn Effect>,
}

/// The effects playing, on their stage.
pub struct Player {
    pub stage: Stage,
    running: Vec<Running>,
    last_play: Option<f32>,
    ticked: f32,
}

impl Player {
    pub fn new(w: f32, h: f32, seed: u64) -> Self {
        Self {
            stage: Stage::new(w, h, seed),
            running: Vec::new(),
            last_play: None,
            ticked: 0.,
        }
    }

    pub fn play(&mut self, who: &Who, reaction: &Reaction) {
        self.last_play = Some(self.stage.now);
        if reaction.kind == "disco_end" {
            for running in self
                .running
                .iter_mut()
                .filter(|running| running.kind == "disco")
            {
                running.effect.end(&mut self.stage, Some(who));
            }
            return;
        }
        for running in self
            .running
            .iter_mut()
            .filter(|running| running.kind == reaction.kind)
        {
            if running.effect.again(&mut self.stage, who, reaction) {
                return;
            }
        }
        if let Some(effect) = effects::start(&mut self.stage, who, reaction) {
            self.running.push(Running {
                kind: reaction.kind.clone(),
                started: self.stage.now,
                before: 0.,
                effect,
            });
        }
    }

    /// Move everything on to `now` seconds since the layer started.
    pub fn advance(&mut self, now: f32) {
        let stage = &mut self.stage;
        stage.now = now.max(stage.now);
        // Whole 60 Hz steps, rounded so frames at 60 Hz take one each however they jitter.
        let due = ((stage.now - self.ticked) / TICK).round().max(0.);
        let ticks = due.min(6.) as u32;
        self.ticked = if due > 6. {
            stage.now
        } else {
            self.ticked + due * TICK
        };
        stage.ticks = ticks;
        for _ in 0..ticks {
            stage.particles.tick(stage.h);
        }
        self.running.retain_mut(|running| {
            let clock = Clock {
                before: running.before,
                now: stage.now - running.started,
            };
            running.before = clock.now;
            running.effect.step(stage, clock)
        });
        stage.tidy();
    }

    pub fn frame(&mut self) -> Frame {
        let mut canvas = Canvas::default();
        self.stage.particles.draw(&mut canvas);
        for running in &self.running {
            running.effect.draw(&self.stage, &mut canvas);
        }
        self.stage.draw_flashes(&mut canvas);
        let mut banners = Vec::new();
        self.stage.draw_banners(&mut banners);
        Frame {
            items: canvas.items,
            banners,
            shake: self.stage.shake_offset(),
        }
    }

    pub fn busy(&self) -> bool {
        !self.running.is_empty() || self.stage.busy()
    }

    /// Whether the layer can go: quiet a while with nothing left, or quiet a long while.
    pub fn idle(&self) -> bool {
        let quiet = self
            .last_play
            .map_or(f32::INFINITY, |at| self.stage.now - at);
        quiet >= IDLE_AFTER && (!self.busy() || quiet >= GONE_AFTER)
    }

    /// Everything off at once.
    pub fn stop(&mut self) {
        self.running.clear();
        self.stage.clear();
    }
}

/// The reactions playing in this call, for the layer to paint. Stepped by the layer's
/// frames; by a timer when no layer could open, so the sounds still play.
pub struct Reactions {
    pub player: Player,
    speaker: sound::Speaker,
    started: Instant,
    painted: Option<Instant>,
    /// Whether reactions make sounds (the tray's switch).
    pub sounds: bool,
}

/// Nothing is playing any more and nothing's left to show.
pub struct Idle;

impl EventEmitter<Idle> for Reactions {}

impl Reactions {
    pub fn new(size: [f32; 2], sounds: bool, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let alive = this.update(cx, |this, cx| {
                    let painting = this
                        .painted
                        .is_some_and(|at| at.elapsed() < Duration::from_millis(150));
                    if !painting {
                        this.advance();
                    }
                    if this.player.idle() {
                        cx.emit(Idle);
                    } else if this.player.busy() {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |since| since.as_nanos() as u64);
        Self {
            player: Player::new(size[0], size[1], seed),
            speaker: sound::Speaker::default(),
            started: Instant::now(),
            painted: None,
            sounds,
        }
    }

    /// Play `played` with the call window at `call` (x, y, width, height on this screen).
    pub fn play(&mut self, played: &Played, call: Option<[f32; 4]>, cx: &mut Context<Self>) {
        self.advance();
        let stage = &mut self.player.stage;
        if let Some(call) = call {
            stage.call = call;
        }
        let (me_key, me_name, me_rgb) = &played.me;
        let me = Who::new(me_key, me_name, *me_rgb, true);
        stage.people = std::iter::once(me.clone())
            .chain(
                played
                    .people
                    .iter()
                    .map(|(key, name, rgb)| Who::new(key, name, *rgb, false)),
            )
            .collect();
        let who = if played.mine {
            me
        } else {
            Who::new(&played.sender, &played.name, played.rgb, false)
        };
        self.player.play(&who, &played.reaction);
        self.flush();
        cx.notify();
    }

    pub fn set_sounds(&mut self, on: bool) {
        if self.sounds && !on {
            self.speaker.hush_all();
        }
        self.sounds = on;
    }

    /// The screen changed size.
    pub fn resize(&mut self, size: [f32; 2]) {
        self.player.stage.w = size[0];
        self.player.stage.h = size[1];
    }

    fn advance(&mut self) {
        self.player.advance(self.started.elapsed().as_secs_f32());
        self.flush();
    }

    /// The frame to paint now.
    pub fn frame(&mut self) -> Frame {
        self.painted = Some(Instant::now());
        self.advance();
        self.player.frame()
    }

    fn flush(&mut self) {
        let (sounds, hushed) = self.player.stage.take_sounds();
        for tag in hushed {
            self.speaker.hush(&tag);
        }
        if self.sounds {
            for sound in sounds {
                self.speaker.play(sound);
            }
        }
    }

    /// The call is over: everything stops at once.
    pub fn stop(&mut self) {
        self.player.stop();
        self.speaker.hush_all();
    }
}

impl Drop for Reactions {
    fn drop(&mut self) {
        self.speaker.hush_all();
    }
}

#[cfg(test)]
mod tests;
