//! Stamps drawn on an iced canvas frame: the marker that drops in, the rings that breathe
//! while it takes words, and the card beside it.
//!
//! A canvas has no blur, so shadows are stacks of faint shapes. It also always draws text over
//! images over shapes, whatever the order you ask for, so a card clips its words to the parts
//! no newer card covers. See [`uncovered`].

use super::cursor::{CursorImage, SPRITE_SCALE};
use crate::components::fonts::{GEIST_MEDIUM, GEIST_REGULAR};
use crate::utils::geometry::Position;
use hopp_marks::colours;
use hopp_marks::sprites;
use hopp_marks::stamps::{
    card_position, Pose, Stamp, CARD_LINE_HEIGHT, CARD_MAX_LINES, CARD_MAX_TEXT_WIDTH,
    CARD_MIN_WIDTH, CARD_PADDING, CARD_RADIUS, CARD_TEXT_SIZE, CARD_TITLE_HEIGHT, CARD_TITLE_SIZE,
    DRAFT_ALPHA, MARKER_RADIUS, MARKER_RISE,
};
use iced::widget::canvas::{Frame, LineCap, Path, Stroke, Text};
use iced::{Color, Font, Pixels, Point, Rectangle, Size};
use iced_core::alignment::Vertical;
use iced_core::text::{Alignment, LineHeight, Paragraph as _, Shaping, Wrapping};
use std::collections::HashMap;
use std::time::Instant;

const CARD_INK: [u8; 3] = [0x1c, 0x1d, 0x26];
/// How far someone's colour moves towards the card's ink, so their name reads on white.
const NAME_DEEPENED: f32 = 0.22;
const SHADOW_LAYERS: usize = 6;
/// A card this faded no longer hides the words under it.
const CARD_HIDES_FROM: f32 = 0.5;

/// Where a stamp is known from: who sent it and their id for it.
pub(super) type StampKey = (String, u64);

/// What it takes to draw a stamp, kept so a frame doesn't render or measure anything.
#[derive(Debug)]
pub(super) struct StampVisual {
    rgb: [u8; 3],
    number: u32,
    marker: CursorImage,
    words: String,
    /// Words still being recognised or a prompt, not final words.
    dim: bool,
    /// Their words, or "taking words" dots after the prompt.
    prompt_dots: bool,
    /// Measured words and name, before the card's padding.
    words_size: (f32, f32),
    name_width: f32,
}

fn measure(text: &str, font: Font, size: f32, line_height: f32) -> (f32, f32) {
    let paragraph = iced_wgpu::graphics::text::Paragraph::with_text(iced_core::text::Text {
        content: text,
        bounds: Size::new(CARD_MAX_TEXT_WIDTH, f32::INFINITY),
        size: Pixels(size),
        line_height: LineHeight::Absolute(Pixels(line_height)),
        font,
        align_x: Alignment::Default,
        align_y: Vertical::Top,
        shaping: Shaping::Advanced,
        // What the canvas does when it draws text.
        wrapping: Wrapping::WordOrGlyph,
    });
    let bounds = paragraph.min_bounds();
    (bounds.width, bounds.height)
}

/// What a stamp shows with no words yet, if it's still taking them.
fn prompt(stamp: &Stamp) -> Option<(&'static str, bool)> {
    if !stamp.listening {
        None
    } else if !stamp.is_mine() {
        // They may be typing or talking, depending on what they're using.
        Some(("Adding a note", true))
    } else if stamp.voice {
        Some(("Listening", true))
    } else {
        Some(("Type a note", true))
    }
}

impl StampVisual {
    pub(super) fn new(stamp: &Stamp, rgb: [u8; 3]) -> Option<Self> {
        let mut visual = Self {
            rgb,
            number: stamp.number,
            marker: CursorImage::new(sprites::marker(rgb, stamp.number, SPRITE_SCALE)?),
            words: String::new(),
            dim: false,
            prompt_dots: false,
            words_size: (0., 0.),
            name_width: measure(
                &stamp.name,
                GEIST_MEDIUM,
                CARD_TITLE_SIZE,
                CARD_TITLE_HEIGHT,
            )
            .0,
        };
        visual.set_words(stamp);
        Some(visual)
    }

    /// Catches up with what the stamp says and what colour its placer is now.
    pub(super) fn update(&mut self, stamp: &Stamp, rgb: [u8; 3]) {
        if self.rgb != rgb {
            match sprites::marker(rgb, self.number, SPRITE_SCALE) {
                Some(marker) => {
                    self.marker = CursorImage::new(marker);
                    self.rgb = rgb;
                }
                None => log::warn!("StampVisual::update: couldn't render marker"),
            }
        }
        self.set_words(stamp);
    }

    fn set_words(&mut self, stamp: &Stamp) {
        let (words, draft_only) = stamp.words();
        let (words, dim, prompt_dots) = if !words.is_empty() {
            (words, draft_only, false)
        } else if let Some((prompt, dots)) = prompt(stamp) {
            (prompt.to_owned(), true, dots)
        } else {
            (String::new(), false, false)
        };
        if words == self.words && prompt_dots == self.prompt_dots {
            self.dim = dim;
            return;
        }
        self.words_size = if words.is_empty() {
            (0., 0.)
        } else if prompt_dots {
            // Room for the most dots, so the card doesn't breathe with them.
            measure_words(&format!("{words}..."))
        } else {
            measure_words(&words)
        };
        self.words = words;
        self.dim = dim;
        self.prompt_dots = prompt_dots;
    }
}

fn measure_words(words: &str) -> (f32, f32) {
    measure(words, GEIST_REGULAR, CARD_TEXT_SIZE, CARD_LINE_HEIGHT)
}

fn with_alpha(rgb: [u8; 3], alpha: f32) -> Color {
    let [r, g, b] = rgb;
    Color::from_rgba8(r, g, b, alpha)
}

fn black(alpha: f32) -> Color {
    Color {
        a: alpha,
        ..Color::BLACK
    }
}

fn rounded(rect: Rectangle, radius: f32) -> Path {
    Path::rounded_rectangle(rect.position(), rect.size(), radius.into())
}

/// A soft shadow of `rect`: from its edge out and in by half of `blur`, fading across that.
fn soft_shadow(frame: &mut Frame, rect: Rectangle, radius: f32, blur: f32, alpha: f32) {
    let layer = black(alpha / SHADOW_LAYERS as f32);
    for step in 0..SHADOW_LAYERS {
        let from_outside = (step as f32 + 0.5) / SHADOW_LAYERS as f32;
        let spread = blur / 2. * (1. - 2. * from_outside);
        let shape = Rectangle {
            x: rect.x - spread,
            y: rect.y - spread,
            width: rect.width + spread * 2.,
            height: rect.height + spread * 2.,
        };
        if shape.width > 0. && shape.height > 0. {
            frame.fill(&rounded(shape, (radius + spread).max(0.)), layer);
        }
    }
}

/// The parts of `rect` not under any of `covers`.
fn uncovered(rect: Rectangle, covers: &[Rectangle]) -> Vec<Rectangle> {
    let mut parts = vec![rect];
    for cover in covers {
        let mut left = Vec::with_capacity(parts.len());
        for part in parts {
            let Some(hidden) = part.intersection(cover) else {
                left.push(part);
                continue;
            };
            let (right, bottom) = (part.x + part.width, part.y + part.height);
            let (hidden_right, hidden_bottom) = (hidden.x + hidden.width, hidden.y + hidden.height);
            let sides = [
                // Above and below it, full width.
                Rectangle::new(part.position(), Size::new(part.width, hidden.y - part.y)),
                Rectangle::new(
                    Point::new(part.x, hidden_bottom),
                    Size::new(part.width, bottom - hidden_bottom),
                ),
                // Either side of it.
                Rectangle::new(
                    Point::new(part.x, hidden.y),
                    Size::new(hidden.x - part.x, hidden.height),
                ),
                Rectangle::new(
                    Point::new(hidden_right, hidden.y),
                    Size::new(right - hidden_right, hidden.height),
                ),
            ];
            left.extend(
                sides
                    .into_iter()
                    .filter(|side| side.width > 0. && side.height > 0.),
            );
        }
        parts = left;
    }
    parts
}

/// A stamp worked out for this frame.
struct Placed<'a> {
    stamp: &'a Stamp,
    visual: &'a StampVisual,
    pose: Pose,
    /// The point it marks, where its shadow lands.
    tip: Point,
    /// The marker's foot and head, which are above `tip` until it lands.
    foot: Point,
    head: Point,
    card: Option<Rectangle>,
}

fn place<'a>(
    stamp: &'a Stamp,
    visual: &'a StampVisual,
    tip: Position,
    area: Size,
    now: Instant,
) -> Placed<'a> {
    let pose = stamp.pose(now);
    let tip = Point::new(tip.x as f32, tip.y as f32);
    let foot = Point::new(tip.x, tip.y - pose.lift);
    let head = Point::new(foot.x, foot.y - MARKER_RISE * pose.grow);
    let radius = MARKER_RADIUS * pose.grow;

    let card = (!visual.words.is_empty() && pose.card_alpha > 0.01).then(|| {
        let (pad_x, pad_y) = CARD_PADDING;
        let width = visual
            .words_size
            .0
            .max(visual.name_width)
            .max(CARD_MIN_WIDTH)
            + pad_x * 2.;
        let words_height = visual.words_size.1.min(CARD_MAX_LINES * CARD_LINE_HEIGHT);
        let size = (width, CARD_TITLE_HEIGHT + words_height + pad_y * 2.);
        let (x, y) = card_position(
            (head.x, head.y),
            radius,
            size,
            (area.width, area.height),
            pose.card_slide,
        );
        Rectangle::new(Point::new(x, y), Size::new(size.0, size.1))
    });

    Placed {
        stamp,
        visual,
        pose,
        tip,
        foot,
        head,
        card,
    }
}

impl Placed<'_> {
    fn paint_marker(&self, frame: &mut Frame) {
        let Pose {
            alpha,
            landing,
            grow,
            rings,
            ..
        } = self.pose;
        // Its shadow on the screen widens as it lands.
        let spread = 0.3 + 0.7 * landing;
        soft_shadow(
            frame,
            Rectangle::new(
                Point::new(self.tip.x - 6. * spread, self.tip.y - 1.5),
                Size::new(12. * spread, 3.),
            ),
            1.5,
            4.,
            0.45 * alpha * landing,
        );
        for (radius, ring_alpha) in rings {
            if ring_alpha > 0. {
                frame.stroke(
                    &Path::circle(self.head, radius),
                    Stroke::default()
                        .with_width(1.8)
                        .with_line_cap(LineCap::Round)
                        .with_color(with_alpha(self.visual.rgb, ring_alpha)),
                );
            }
        }
        self.visual
            .marker
            .draw_scaled(frame, self.foot.x, self.foot.y, grow, alpha);
    }

    fn paint_card_body(&self, frame: &mut Frame, card: Rectangle) {
        let shown = self.pose.card_alpha;
        let lower = |by: f32| Rectangle {
            y: card.y + by,
            ..card
        };
        soft_shadow(frame, lower(4.), CARD_RADIUS, 10., 0.14 * shown);
        soft_shadow(frame, lower(1.), CARD_RADIUS, 2.5, 0.3 * shown);
        let shape = rounded(card, CARD_RADIUS);
        frame.fill(&shape, Color::from_rgba(1., 1., 1., 0.98 * shown));
        frame.stroke(
            &shape,
            Stroke::default()
                .with_width(1.)
                .with_color(black(0.06 * shown)),
        );
    }

    fn words_now(&self, now: Instant) -> String {
        let visual = self.visual;
        if visual.prompt_dots {
            let age = now.duration_since(self.stamp.placed).as_millis();
            let dots = 1 + (age / 380 % 3) as usize;
            format!("{}{}", visual.words, ".".repeat(dots))
        } else {
            visual.words.clone()
        }
    }

    /// The name and words, in the parts of the card that aren't hidden.
    fn paint_card_words(
        &self,
        frame: &mut Frame,
        card: Rectangle,
        covers: &[Rectangle],
        now: Instant,
    ) {
        let shown = self.pose.card_alpha;
        let (pad_x, pad_y) = CARD_PADDING;
        let name_ink = colours::mix(self.visual.rgb, CARD_INK, NAME_DEEPENED);
        let inside = Rectangle::new(
            Point::new(card.x + pad_x, card.y + pad_y),
            Size::new(card.width - pad_x * 2., card.height - pad_y * 2.),
        );
        let words_top = inside.y + CARD_TITLE_HEIGHT;
        let words_height = inside.height - CARD_TITLE_HEIGHT;
        // Past the most lines a card shows, you're still typing at the bottom.
        let overflow = self.visual.words_size.1 - words_height;
        let scrolled = if self.stamp.listening && overflow > 0. {
            overflow
        } else {
            0.
        };
        let ink_alpha = if self.visual.dim { DRAFT_ALPHA } else { 1. };
        let name = self.stamp.name.as_str();
        let words = self.words_now(now);

        let words_area = Rectangle::new(
            Point::new(inside.x, words_top),
            Size::new(CARD_MAX_TEXT_WIDTH.max(inside.width), words_height),
        );
        let title_area = Rectangle::new(
            Point::new(inside.x, inside.y - 1.),
            Size::new(inside.width, CARD_TITLE_HEIGHT),
        );
        for part in uncovered(title_area, covers) {
            frame.with_clip(part, |frame| {
                frame.fill_text(Text {
                    content: name.to_owned(),
                    position: Point::new(inside.x, inside.y - 1.),
                    max_width: CARD_MAX_TEXT_WIDTH,
                    color: with_alpha(name_ink, shown),
                    size: Pixels(CARD_TITLE_SIZE),
                    line_height: LineHeight::Absolute(Pixels(CARD_TITLE_HEIGHT)),
                    font: GEIST_MEDIUM,
                    align_x: Alignment::Default,
                    align_y: Vertical::Top,
                    shaping: Shaping::Advanced,
                });
            });
        }
        for part in uncovered(words_area, covers) {
            frame.with_clip(part, |frame| {
                frame.fill_text(Text {
                    content: words.clone(),
                    position: Point::new(inside.x, words_top - scrolled),
                    max_width: CARD_MAX_TEXT_WIDTH,
                    color: with_alpha(CARD_INK, ink_alpha * shown),
                    size: Pixels(CARD_TEXT_SIZE),
                    line_height: LineHeight::Absolute(Pixels(CARD_LINE_HEIGHT)),
                    font: GEIST_REGULAR,
                    align_x: Alignment::Default,
                    align_y: Vertical::Top,
                    shaping: Shaping::Advanced,
                });
            });
        }
    }
}

/// Draws every stamp, oldest underneath. `translate` turns a point on the shared screen into
/// a place on this frame.
pub(super) fn paint_stamps<'a>(
    frame: &mut Frame,
    stamps: impl Iterator<Item = &'a Stamp>,
    visuals: &HashMap<StampKey, StampVisual>,
    translate: &dyn Fn(Position) -> Position,
    now: Instant,
) {
    let area = frame.size();
    let placed: Vec<Placed> = stamps
        .filter(|stamp| !stamp.gone(now))
        .filter_map(|stamp| {
            let visual = visuals.get(&(stamp.owner.clone(), stamp.id))?;
            let tip = translate(Position {
                x: stamp.point.0,
                y: stamp.point.1,
            });
            Some(place(stamp, visual, tip, area, now))
        })
        .collect();

    for (index, stamp) in placed.iter().enumerate() {
        stamp.paint_marker(frame);
        let Some(card) = stamp.card else {
            continue;
        };
        stamp.paint_card_body(frame, card);
        let covers: Vec<Rectangle> = placed[index + 1..]
            .iter()
            .filter(|newer| newer.pose.card_alpha >= CARD_HIDES_FROM)
            .filter_map(|newer| newer.card)
            .collect();
        stamp.paint_card_words(frame, card, &covers, now);
    }
}

#[cfg(test)]
mod tests {
    use super::super::cursor::tests::{near, Screen};
    use super::*;
    use hopp_marks::stamps::{StampInput, StampMessage, Stamps};
    use std::time::Duration;

    const WIDTH: f32 = 800.;
    const HEIGHT: f32 = 500.;
    const SAMPLE: &str = "Change this to the new pricing, and make sure the discount code still \
                          works when someone comes back from the checkout page.";

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
        Rectangle::new(Point::new(x, y), Size::new(width, height))
    }

    fn area_of(parts: &[Rectangle]) -> f32 {
        parts.iter().map(|part| part.width * part.height).sum()
    }

    #[test]
    fn words_show_only_where_no_newer_card_covers_them() {
        let card = rect(0., 0., 100., 50.);
        assert_eq!(uncovered(card, &[]), [card]);
        assert_eq!(uncovered(card, &[rect(200., 0., 10., 10.)]), [card]);
        assert!(uncovered(card, &[rect(-5., -5., 200., 200.)]).is_empty());

        // A corner: an L is left, and all of it.
        let parts = uncovered(card, &[rect(60., 30., 100., 100.)]);
        assert_eq!(area_of(&parts), 100. * 50. - 40. * 20.);

        // In the middle: four pieces around it, none overlapping.
        let parts = uncovered(card, &[rect(20., 10., 10., 10.)]);
        assert_eq!(parts.len(), 4);
        assert_eq!(area_of(&parts), 100. * 50. - 10. * 10.);

        // Two cards, overlapping each other as well.
        let covers = [rect(0., 0., 50., 50.), rect(25., 0., 50., 50.)];
        assert_eq!(area_of(&uncovered(card, &covers)), 25. * 50.);
    }

    #[test]
    fn a_stamp_with_no_words_says_how_it_is_getting_them() {
        let now = Instant::now();
        let place = StampInput::Place { x: 0.5, y: 0.5 };
        let only = |stamps: &Stamps| prompt(stamps.iter().next().unwrap());

        // Clicked, then held down.
        let mut stamps = Stamps::default();
        stamps.apply(place.clone(), "Ada", now);
        assert_eq!(only(&stamps), Some(("Type a note", true)));
        stamps.apply(StampInput::Listen, "Ada", now);
        assert_eq!(only(&stamps), Some(("Listening", true)));

        // Finished, it has nothing to ask.
        stamps.apply(StampInput::Typed("Here".to_owned()), "Ada", now);
        stamps.apply(StampInput::Finish, "Ada", now);
        assert_eq!(only(&stamps), None);

        // Theirs could be either.
        let mut stamps = Stamps::default();
        let theirs = StampMessage::StampPlace {
            id: 1,
            x: 0.5,
            y: 0.5,
        };
        stamps.receive("room:r:lee:video", "Lee", theirs, now);
        assert_eq!(only(&stamps), Some(("Adding a note", true)));
    }

    /// Someone else's stamp saying `text`, placed at `placed`.
    fn say(
        stamps: &mut Stamps,
        id: u64,
        name: &str,
        (x, y): (f64, f64),
        text: &str,
        placed: Instant,
    ) {
        let sender = format!("room:r:{name}:video");
        stamps.receive(&sender, name, StampMessage::StampPlace { id, x, y }, placed);
        let done = StampMessage::StampText {
            id,
            text: text.to_owned(),
            draft: String::new(),
            done: true,
        };
        stamps.receive(&sender, name, done, placed);
    }

    fn visuals_of(stamps: &Stamps) -> HashMap<StampKey, StampVisual> {
        stamps
            .iter()
            .map(|stamp| {
                let rgb = if stamp.is_mine() {
                    [0x0a, 0x84, 0xff]
                } else {
                    colours::colour(&stamp.owner)
                };
                let visual = StampVisual::new(stamp, rgb).expect("visual");
                ((stamp.owner.clone(), stamp.id), visual)
            })
            .collect()
    }

    fn fractions(position: Position) -> Position {
        Position {
            x: position.x * f64::from(WIDTH),
            y: position.y * f64::from(HEIGHT),
        }
    }

    /// Everything a stamp can be, in one picture.
    fn every_kind_of_stamp(start: Instant) -> Stamps {
        let mut stamps = Stamps::default();
        say(
            &mut stamps,
            1,
            "Mira",
            (0.1, 0.2),
            "This should be blue",
            start,
        );
        say(&mut stamps, 2, "Jonas", (0.1, 0.55), SAMPLE, start);
        // Six lines and more.
        let long = SAMPLE.repeat(5);
        say(&mut stamps, 3, "Sam Taylor", (0.52, 0.3), &long, start);
        // Against the right edge, the top and the bottom.
        say(
            &mut stamps,
            4,
            "Lee",
            (0.97, 0.75),
            "No room on my right",
            start,
        );
        say(
            &mut stamps,
            5,
            "Lee",
            (0.5, 0.02),
            "Top of the screen",
            start,
        );
        say(&mut stamps, 6, "Lee", (0.3, 0.99), "Bottom too", start);
        // Yours, waiting for words.
        stamps.apply(StampInput::Place { x: 0.72, y: 0.92 }, "You", start);
        // Someone's words still being recognised.
        let drafting = StampMessage::StampPlace {
            id: 7,
            x: 0.08,
            y: 0.92,
        };
        stamps.receive("room:r:Ann:video", "Ann", drafting, start);
        let draft = StampMessage::StampText {
            id: 7,
            text: String::new(),
            draft: "maybe move this".into(),
            done: false,
        };
        stamps.receive("room:r:Ann:video", "Ann", draft, start);
        // Someone who hasn't said anything yet.
        let waiting = StampMessage::StampPlace {
            id: 8,
            x: 0.3,
            y: 0.08,
        };
        stamps.receive("room:r:Bo:video", "Bo", waiting, start);
        stamps
    }

    /// What stamps look like, as actually rendered. Needs a GPU, so run it by hand and look at
    /// `/tmp/hopp-stamps-*.png`:
    /// `cargo test --target aarch64-apple-darwin --lib stamps_through_the_gpu -- --ignored`
    #[test]
    #[ignore]
    fn stamps_through_the_gpu() {
        const DARK: [u8; 3] = [26, 27, 38];
        const LIGHT: [u8; 3] = [243, 243, 247];
        const PHYSICAL: (u32, u32) = (WIDTH as u32 * 2, HEIGHT as u32 * 2);
        let colour = |[r, g, b]: [u8; 3]| iced::Color::from_rgb8(r, g, b);
        let mut screen = Screen::new();

        let start = Instant::now();
        let stamps = every_kind_of_stamp(start);
        let visuals = visuals_of(&stamps);
        // Between the rings' pulses, a while after everything has landed.
        let later = start + Duration::from_millis(1_250);
        for (name, background) in [("dark", DARK), ("light", LIGHT)] {
            let picture = screen.paint(PHYSICAL, 2., colour(background), |frame| {
                paint_stamps(frame, stamps.iter(), &visuals, &fractions, later);
            });
            picture
                .save(format!("/tmp/hopp-stamps-{name}.png"))
                .expect("save");
        }

        // It drops in over time, left to right, and its card slides out after it.
        let mut dropping = Stamps::default();
        for (index, age) in [30, 90, 150, 220, 320, 460, 800u64].into_iter().enumerate() {
            let placed = later - Duration::from_millis(age);
            let x = 0.03 + index as f64 * 0.135;
            say(&mut dropping, index as u64, "Mira", (x, 0.5), "Hi", placed);
        }
        let visuals = visuals_of(&dropping);
        let picture = screen.paint(PHYSICAL, 2., colour(DARK), |frame| {
            paint_stamps(frame, dropping.iter(), &visuals, &fractions, later);
        });
        picture.save("/tmp/hopp-stamp-drop.png").expect("save");

        // Yours as it hears you, and a note you're typing that follows the pointer.
        let mut yours = Stamps::default();
        yours.apply(StampInput::Place { x: 0.12, y: 0.25 }, "You", start);
        yours.apply(StampInput::Listen, "You", start);
        let ghost = yours.ghost((0.45, 0.6), "Move this up", "You", later);
        let mut visuals = visuals_of(&yours);
        visuals.insert(
            (ghost.owner.clone(), ghost.id),
            StampVisual::new(&ghost, [0x0a, 0x84, 0xff]).expect("visual"),
        );
        let picture = screen.paint(PHYSICAL, 2., colour(DARK), |frame| {
            paint_stamps(
                frame,
                yours.iter().chain([&ghost]),
                &visuals,
                &fractions,
                later,
            );
        });
        picture.save("/tmp/hopp-stamp-yours.png").expect("save");

        // A short card, alone, on the dark screen: where everything lands.
        let mut alone = Stamps::default();
        say(&mut alone, 1, "Mira", (0.25, 0.5), "Blue", start);
        let visuals = visuals_of(&alone);
        let stamp = alone.iter().next().unwrap();
        let picture = screen.paint(PHYSICAL, 2., colour(DARK), |frame| {
            paint_stamps(frame, alone.iter(), &visuals, &fractions, later);
        });
        picture.save("/tmp/hopp-stamp-alone.png").expect("save");
        let (tip_x, tip_y) = (200., 250.);
        let physical = |x: f32, y: f32| picture.get_pixel((x * 2.) as u32, (y * 2.) as u32);
        let rgb = colours::colour(&stamp.owner);

        // Its head is its colour, off to the side of the number.
        let head = physical(tip_x + MARKER_RADIUS - 3., tip_y - MARKER_RISE);
        assert!(near(head, rgb, 6), "head {head:?} vs {rgb:?}");
        // The card is white, beside the head.
        let beside = tip_x + MARKER_RADIUS + 9. + 4.;
        let top = tip_y - MARKER_RISE - 22.;
        let card = physical(beside, top + 4.);
        assert!(near(card, [250, 250, 250], 6), "card {card:?}");
        // There are words on it, inside its padding, so nothing was clipped away.
        let ink_in = |picture: &image::RgbaImage, area: Rectangle| {
            let mut count = 0;
            for y in (area.y * 2.) as u32..((area.y + area.height) * 2.) as u32 {
                for x in (area.x * 2.) as u32..((area.x + area.width) * 2.) as u32 {
                    if near(picture.get_pixel(x, y), CARD_INK, 90) {
                        count += 1;
                    }
                }
            }
            count
        };
        let words = rect(
            beside - 4. + CARD_PADDING.0,
            top + CARD_PADDING.1 + CARD_TITLE_HEIGHT,
            40.,
            CARD_LINE_HEIGHT,
        );
        let inked = ink_in(&picture, words);
        assert!(inked > 40, "words: {inked} dark pixels");

        // A newer card hides an older card's words, though a canvas draws text over everything:
        // none in the empty part of the card on top, and the rest still showing below it.
        let mut covered = Stamps::default();
        say(&mut covered, 1, "Mira", (0.25, 0.5), SAMPLE, start);
        let wide_name = "A very long name that widens the card";
        say(&mut covered, 2, wide_name, (0.25, 0.5), "Short", start);
        let visuals = visuals_of(&covered);
        let picture = screen.paint(PHYSICAL, 2., colour(DARK), |frame| {
            paint_stamps(frame, covered.iter(), &visuals, &fractions, later);
        });
        picture.save("/tmp/hopp-stamps-covered.png").expect("save");
        let text_x = beside - 4. + CARD_PADDING.0;
        let words_y = top + CARD_PADDING.1 + CARD_TITLE_HEIGHT;
        let beside_short = rect(text_x + 60., words_y, 80., CARD_LINE_HEIGHT);
        assert_eq!(ink_in(&picture, beside_short), 0, "showing through");
        let third_line = rect(
            text_x,
            words_y + CARD_LINE_HEIGHT * 2.,
            200.,
            CARD_LINE_HEIGHT,
        );
        assert!(
            ink_in(&picture, third_line) > 40,
            "older words clipped away"
        );
    }
}
