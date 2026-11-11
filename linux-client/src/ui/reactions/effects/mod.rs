//! The ten reactions, ported from the approved prototypes (`reactions-proto/protos`, as
//! the Mac app plays them in `tauri/src/windows/reactions/effects`). Each is an
//! [`Effect`]: it moves on as its clock does, and draws itself.

mod achievement;
mod clap;
mod confetti;
mod disco;
mod emoji;
mod fireworks;
mod flip;
mod gong;
mod pads;
mod ship;

use super::{
    scene::Canvas,
    stage::{Clock, Stage, Who},
};
use crate::protocol::Reaction;

pub trait Effect {
    /// Move on to `clock.now` seconds since it began. False once it's over.
    fn step(&mut self, stage: &mut Stage, clock: Clock) -> bool;

    /// Draw it as it is now.
    fn draw(&self, stage: &Stage, canvas: &mut Canvas);

    /// The same kind of reaction came again while this one plays: true if this one
    /// took it (another strike of the gong, another clap).
    fn again(&mut self, _stage: &mut Stage, _who: &Who, _reaction: &Reaction) -> bool {
        false
    }

    /// Wind down now: `disco_end` from `who`, or (with `None`) the effect is being
    /// replaced.
    fn end(&mut self, _stage: &mut Stage, _who: Option<&Who>) {}
}

/// The effect for `reaction` from `who`, if it starts one.
pub fn start(stage: &mut Stage, who: &Who, reaction: &Reaction) -> Option<Box<dyn Effect>> {
    let level = reaction.level;
    let text = reaction.text.as_deref().unwrap_or("");
    let choice = reaction.choice.as_deref().unwrap_or("");
    Some(match reaction.kind.as_str() {
        "confetti" => Box::new(confetti::Confetti::new(stage, who, level)),
        "gong" => Box::new(gong::Gong::new(stage, who)),
        "table_flip" => Box::new(flip::Flip::new(stage, who)),
        "emoji" => Box::new(emoji::Emoji::new(stage, who, choice)?),
        "sound" => Box::new(pads::Pad::new(stage, who, choice)?),
        "clap" => Box::new(clap::Clap::new(stage, who)),
        "fireworks" => Box::new(fireworks::Fireworks::new(stage, who, level.max(1))),
        "achievement" => {
            let to = reaction
                .to
                .as_deref()
                .and_then(|to| {
                    // By user id: yours is all you have of your own identity.
                    let to = crate::ink::person(to);
                    stage
                        .people
                        .iter()
                        .find(|person| crate::ink::person(&person.key) == to)
                })
                .cloned();
            Box::new(achievement::Achievement::new(stage, who, text, to))
        }
        "disco" => Box::new(disco::Disco::new(stage, who)),
        "ship_it" => Box::new(ship::Ship::new(stage, who, text)),
        _ => return None,
    })
}
