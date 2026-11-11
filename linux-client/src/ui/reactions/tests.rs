//! Every reaction played through: their sounds checked, the biggest show held to a
//! frame's time, and pictures of them for a person to look at.
//!
//! With `HOPP_REACTIONS_PREVIEW=<dir>` set, the pictures go there as PNGs (1440×900, the
//! prototypes' shots' size, with the call window where theirs is) and the sounds as WAVs.
use std::{path::PathBuf, time::Instant};

use super::{Player, particles::MOST, scene, stage::Who, synth};
use crate::protocol::Reaction;

const W: f32 = 1440.;
const H: f32 = 900.;
/// Where the call window is in the prototypes' shots.
const CALL: [f32; 4] = [1052., 137., 300., 618.];

fn me() -> Who {
    Who::new("room:r:u-sam", "Sam Rivera", [0x0a, 0x84, 0xff], true)
}

fn alex() -> Who {
    Who::new("room:r:u-alex", "Alex Morgan", [0xbf, 0x5a, 0xf2], false)
}

fn player(seed: u64) -> Player {
    let mut player = Player::new(W, H, seed);
    player.stage.call = CALL;
    player.stage.people = vec![
        me(),
        alex(),
        Who::new("room:r:u-riley", "Riley Chen", [0x30, 0xd1, 0x58], false),
    ];
    player
}

fn reaction(kind: &str, choice: Option<&str>, level: u8, text: Option<&str>) -> Reaction {
    Reaction {
        choice: choice.map(Into::into),
        level,
        text: text.map(Into::into),
        ..Reaction::new(kind)
    }
}

/// One of everything there is to send, by a name for its files.
fn everything() -> Vec<(String, Reaction)> {
    let mut all = vec![
        ("confetti".into(), reaction("confetti", None, 0, None)),
        ("confetti-4".into(), reaction("confetti", None, 4, None)),
        ("gong".into(), reaction("gong", None, 0, None)),
        ("table_flip".into(), reaction("table_flip", None, 0, None)),
        ("clap".into(), reaction("clap", None, 0, None)),
        ("fireworks-3".into(), reaction("fireworks", None, 3, None)),
        ("fireworks-10".into(), reaction("fireworks", None, 10, None)),
        (
            "achievement".into(),
            Reaction {
                to: Some("room:r:u-alex:audio".into()),
                ..reaction("achievement", None, 0, Some("Fixed the flaky test"))
            },
        ),
        (
            "achievement-plain".into(),
            reaction("achievement", None, 0, None),
        ),
        ("disco".into(), reaction("disco", None, 0, None)),
        ("ship_it".into(), reaction("ship_it", None, 0, Some("v2.0"))),
    ];
    for choice in crate::protocol::REACTION_EMOJI {
        all.push((
            format!("emoji-{choice}"),
            reaction("emoji", Some(choice), 0, None),
        ));
    }
    for choice in crate::protocol::REACTION_SOUNDS {
        all.push((
            format!("sound-{choice}"),
            reaction("sound", Some(choice), 0, None),
        ));
    }
    all
}

fn preview_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("HOPP_REACTIONS_PREVIEW")?);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

#[test]
fn every_reaction_plays_and_leaves_the_screen_clear() {
    for (name, reaction) in everything() {
        let mut player = player(7);
        player.play(&alex(), &reaction);
        let mut most = 0;
        let mut frames = 0;
        for step in 1..=60 * 60 {
            player.advance(step as f32 / 60.);
            let frame = player.frame();
            most = most.max(player.stage.particles.len());
            if !frame.is_empty() {
                frames += 1;
            }
            let _ = player.stage.take_sounds();
            if player.idle() {
                break;
            }
        }
        assert!(frames > 0, "{name} showed nothing");
        assert!(most <= MOST, "{name} had {most} particles");
        assert!(player.idle(), "{name} never ended");
        assert!(!player.busy(), "{name} left something on screen");
    }
}

#[test]
fn a_disco_ends_early_only_for_whoever_started_it() {
    let mut player = player(3);
    player.play(&alex(), &Reaction::new("disco"));
    player.advance(1.);
    player.play(&me(), &Reaction::new("disco_end"));
    for step in 61..=120 {
        player.advance(step as f32 / 60.);
    }
    assert!(player.busy(), "someone else ended Alex's disco");
    player.play(&alex(), &Reaction::new("disco_end"));
    for step in 121..=60 * 4 {
        player.advance(step as f32 / 60.);
    }
    assert!(!player.busy(), "Alex's disco went on after they ended it");
}

/// Everything that makes a sound, rendered: never silent, never clipping.
#[test]
fn every_reaction_sounds_and_never_clips() {
    let dir = preview_dir();
    let mut report = Vec::new();
    for (name, reaction) in everything() {
        let mut player = player(11);
        player.play(&alex(), &reaction);
        let mut sounds = Vec::new();
        for step in 0..=60 * 12 {
            if step > 0 {
                player.advance(step as f32 / 60.);
            }
            sounds.extend(player.stage.take_sounds().0);
        }
        assert!(!sounds.is_empty(), "{name} made no sound");
        // Played as they come: each from when it was made, mixed.
        let mut loudest = 0_i32;
        let mut energy = 0_f64;
        let mut count = 0_usize;
        for (index, sound) in sounds.iter().enumerate() {
            let samples = sound.graph.render();
            for &sample in &samples {
                loudest = loudest.max(i32::from(sample).abs());
                energy += f64::from(sample).powi(2);
            }
            count += samples.len();
            if let Some(dir) = &dir {
                std::fs::write(
                    dir.join(format!("{name}-{index}.wav")),
                    synth::wav(&samples),
                )
                .unwrap();
            }
        }
        let peak_db = 20. * (f64::from(loudest) / 32768.).log10();
        let rms_db = 10. * (energy / count.max(1) as f64 / 32768_f64.powi(2)).log10();
        report.push(format!(
            "{name:<22} {:>2} sound(s) peak {peak_db:>6.1} dBFS rms {rms_db:>6.1} dBFS",
            sounds.len()
        ));
        assert!(loudest < 32767, "{name} clips");
        assert!(
            peak_db > -40.,
            "{name} is next to silent ({peak_db:.1} dBFS)"
        );
    }
    println!("{}", report.join("\n"));
}

/// The biggest show at once, fireworks at ten with the gong hit again and again, within
/// a 60 Hz frame's time to step and lay out.
#[test]
fn the_biggest_show_keeps_to_its_frame_budget() {
    let mut player = player(5);
    player.play(&alex(), &reaction("fireworks", None, 10, None));
    player.play(&me(), &reaction("fireworks", None, 10, None));
    let mut times = Vec::new();
    let mut most = 0;
    let mut items = 0;
    for step in 1..=60 * 10 {
        let now = step as f32 / 60.;
        // A gong as often as the limits allow one person (eight in three seconds), and
        // confetti from the other.
        if step % 23 == 0 {
            player.play(&alex(), &reaction("gong", None, 0, None));
            player.play(&me(), &reaction("confetti", None, 4, None));
        }
        let started = Instant::now();
        player.advance(now);
        let frame = player.frame();
        times.push(started.elapsed().as_secs_f64() * 1000.);
        most = most.max(player.stage.particles.len());
        items = items.max(frame.items.len());
        let _ = player.stage.take_sounds();
    }
    times.sort_by(f64::total_cmp);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let p99 = times[times.len() * 99 / 100];
    let worst = times[times.len() - 1];
    println!(
        "frame: mean {mean:.2} ms, p99 {p99:.2} ms, worst {worst:.2} ms; \
         {most} particles at most, {items} items"
    );
    assert!(most <= MOST);
    // Unoptimised test builds run several times slower than the app.
    let budget = if cfg!(debug_assertions) { 40. } else { 4. };
    assert!(p99 < budget, "p99 {p99:.2} ms over {budget} ms");
}

/// A dark desktop with a grey call window where the prototypes' is.
fn backdrop() -> resvg::tiny_skia::Pixmap {
    use resvg::tiny_skia::{
        Color, GradientStop, LinearGradient, Paint, Pixmap, Point, Rect, SpreadMode, Transform,
    };
    let mut pixmap = Pixmap::new(W as u32, H as u32).unwrap();
    let mut paint = Paint::default();
    paint.shader = LinearGradient::new(
        Point::from_xy(0., 0.),
        Point::from_xy(0., H),
        vec![
            GradientStop::new(0., Color::from_rgba8(0xa8, 0x4c, 0x62, 0xff)),
            GradientStop::new(1., Color::from_rgba8(0x3a, 0x26, 0x48, 0xff)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap();
    pixmap.fill_rect(
        Rect::from_xywh(0., 0., W, H).unwrap(),
        &paint,
        Transform::identity(),
        None,
    );
    paint.set_color_rgba8(0x80, 0x78, 0x74, 0xe0);
    let [x, y, w, h] = CALL;
    pixmap.fill_rect(
        Rect::from_xywh(x, y, w, h).unwrap(),
        &paint,
        Transform::identity(),
        None,
    );
    pixmap
}

fn save(pixmap: &resvg::tiny_skia::Pixmap, path: &std::path::Path) {
    let rgba: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let colour = pixel.demultiply();
            [colour.red(), colour.green(), colour.blue(), colour.alpha()]
        })
        .collect();
    image::save_buffer(
        path,
        &rgba,
        pixmap.width(),
        pixmap.height(),
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
}

/// Pictures of each reaction part way through. Always painted, so the painter is
/// exercised; saved only for a person to look at.
#[test]
fn previews() {
    let dir = preview_dir();
    let backdrop = backdrop();
    let wanted: &[&str] = if dir.is_some() {
        &[]
    } else {
        &["gong", "clap", "achievement", "disco"]
    };
    for (name, reaction) in everything() {
        if !wanted.is_empty() && !wanted.contains(&name.as_str()) {
            continue;
        }
        let mut player = player(2);
        player.play(&alex(), &reaction);
        let mut step = 0;
        for at in [0.3_f32, 0.8, 1.5, 2.5, 4.] {
            while (step as f32) < at * 60. {
                step += 1;
                player.advance(step as f32 / 60.);
                // A slow clap is many claps.
                if reaction.kind == "clap" && step % 30 == 0 {
                    player.play(&me(), &reaction);
                }
                let _ = player.stage.take_sounds();
            }
            let frame = player.frame();
            let pixmap = scene::preview(&frame, W as u32, H as u32, 1., Some(&backdrop));
            if let Some(dir) = &dir {
                save(&pixmap, &dir.join(format!("{name}-{at:.1}s.png")));
            }
        }
    }
}
