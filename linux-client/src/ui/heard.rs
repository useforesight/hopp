//! "Heard you": what shows the moment the capture hears someone say the wake word.
//!
//! An agent following the call wakes only once the person has finished, and then takes
//! up to a minute to answer. Until this, nothing showed in between, so a slow answer
//! looked the same as a word that was never heard. Two things now show within a second
//! or two of the word, from live text:
//!
//! - **A light runs round their tile:** two quick laps, then slower ones while they go
//!   on talking.
//! - **Their words are echoed back** in a small bubble beside the call window, pointing
//!   at their tile, with the wake word lit up, so you can see the question was heard
//!   right. With no call window on screen (faces docked round a shared screen), the
//!   bubble sits under the bar and says who it heard.
//!
//! Both go [`HOLD`] after their last word. Nothing here knows when the agent answers.
//! If the final text turns out not to have the word, both go at once.
use std::time::{Duration, Instant};

use gpui_kit::{
    AppContext, Bounds, Context, Entity, FontWeight, Hsla, IntoElement, ParentElement, Pixels,
    Render, Styled, Window, WindowHandle, canvas, div,
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    point, px, size,
};
use gpui_omarchy::ActiveTheme;

use super::session::Session;

/// How long the light and the bubble stay after the last word was heard.
pub const HOLD: Duration = Duration::from_millis(3_500);
/// The light comes in and goes out over these.
const FADE_IN: f32 = 0.15;
const FADE_OUT: f32 = 0.6;
/// Two quick laps on hearing the word, then slower ones.
const QUICK_LAP: f32 = 0.7;
const SLOW_LAP: f32 = 1.8;
/// The light's thickness, and how much of the tile's edge its tail covers.
const LINE: f32 = 3.;
const TAIL: f32 = 0.34;

/// The bubble's window: see-through, with the bubble against the edge nearest the tile.
pub const WIDTH: f32 = 250.;
pub const HEIGHT: f32 = 150.;
/// Between the call window and the bubble's point, and the point's length.
const GAP: f32 = 2.;
const POINT: f32 = 6.;
/// The most words the bubble shows: about six lines. Earlier ones make way.
const WORDS: usize = 30;

/// Someone said the wake word, and what they have said since.
#[derive(Clone, Debug)]
pub struct Heard {
    /// Whose tile: a canonical identity, or `speaking::YOU`.
    pub key: String,
    pub name: String,
    pub yours: bool,
    /// Their words from the wake word's line on, each marked if it is the wake word.
    pub words: Vec<(String, bool)>,
    pub since: Instant,
    /// When their words last changed.
    pub updated: Instant,
}

/// Where the light is on its way round a tile, and how bright.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glow {
    /// The head's place along the edge, 0–1, clockwise from the top left.
    pub turn: f32,
    pub strength: f32,
}

impl Heard {
    pub fn over(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.updated) >= HOLD
    }

    pub fn glow(&self, now: Instant) -> Option<Glow> {
        if self.over(now) {
            return None;
        }
        let age = now.saturating_duration_since(self.since).as_secs_f32();
        let left = (HOLD - now.saturating_duration_since(self.updated)).as_secs_f32();
        let laps = if age < 2. * QUICK_LAP {
            age / QUICK_LAP
        } else {
            2. + (age - 2. * QUICK_LAP) / SLOW_LAP
        };
        Some(Glow {
            turn: laps.fract(),
            strength: (age / FADE_IN).min(1.) * (left / FADE_OUT).min(1.),
        })
    }
}

/// The point `turn` (0–1) of the way round a square of `side` with rounded corners,
/// clockwise from the end of the top-left corner.
pub fn edge_point(side: f32, radius: f32, turn: f32) -> (f32, f32) {
    use std::f32::consts::FRAC_PI_2;
    let radius = radius.clamp(0., side / 2.);
    let straight = side - 2. * radius;
    let arc = FRAC_PI_2 * radius;
    let mut along = turn.rem_euclid(1.) * 4. * (straight + arc);
    // Each edge, then the corner it runs into: top, right, bottom, left.
    let starts = [
        (radius, 0.),
        (side, radius),
        (side - radius, side),
        (0., side - radius),
    ];
    let directions = [(1., 0.), (0., 1.), (-1., 0.), (0., -1.)];
    let corners = [
        (side - radius, radius),
        (side - radius, side - radius),
        (radius, side - radius),
        (radius, radius),
    ];
    for edge in 0..4 {
        if along <= straight {
            return (
                starts[edge].0 + directions[edge].0 * along,
                starts[edge].1 + directions[edge].1 * along,
            );
        }
        along -= straight;
        if along <= arc {
            let angle = (edge as f32 - 1.) * FRAC_PI_2 + along / radius.max(f32::EPSILON);
            return (
                corners[edge].0 + radius * angle.cos(),
                corners[edge].1 + radius * angle.sin(),
            );
        }
        along -= arc;
    }
    starts[0]
}

/// The light on a tile of `side`: a bright head with a fading tail, on a faint ring.
/// Drawn as overlapping dots, which GPUI keeps smooth on any graphics driver.
pub fn light(side: f32, radius: f32, glow: Glow, cx: &gpui_kit::App) -> gpui_kit::Div {
    let theme = cx.omarchy();
    let (accent, bright) = (theme.accent, theme.bright);
    div().absolute().inset_0().child(
        canvas(
            |bounds, _, _| bounds,
            move |_, bounds: Bounds<Pixels>, window, _| {
                let inset = LINE / 2.;
                let (inner, corner) = (side - LINE, (radius - inset).max(0.));
                window.paint_quad(gpui_kit::quad(
                    bounds,
                    px(radius),
                    accent.opacity(0.),
                    px(1.),
                    accent.opacity(0.3 * glow.strength),
                    Default::default(),
                ));
                let perimeter = 4. * (inner - 2. * corner) + std::f32::consts::TAU * corner;
                let dots = ((TAIL * perimeter) / 1.5).ceil().max(1.) as usize;
                // Tail first, so the head is drawn over it.
                for dot in (0..dots).rev() {
                    let behind = dot as f32 / dots as f32;
                    let (x, y) = edge_point(inner, corner, glow.turn - behind * TAIL);
                    let fade = (1. - behind).powf(1.6) * glow.strength;
                    let color = if dot == 0 {
                        bright.opacity(fade)
                    } else {
                        tail_color(accent, behind).opacity(fade)
                    };
                    let origin = point(
                        bounds.origin.x + px(inset + x - LINE / 2.),
                        bounds.origin.y + px(inset + y - LINE / 2.),
                    );
                    window.paint_quad(
                        gpui_kit::fill(
                            Bounds {
                                origin,
                                size: size(px(LINE), px(LINE)),
                            },
                            color,
                        )
                        .corner_radii(gpui_kit::Corners::all(px(LINE / 2.))),
                    );
                }
            },
        )
        .absolute()
        .inset_0(),
    )
}

/// The accent at the head, turning a little round the colour wheel along the tail.
fn tail_color(accent: Hsla, behind: f32) -> Hsla {
    let mut color = accent;
    color.h = (color.h + 0.14 * behind).fract();
    color
}

/// The words to show. A long ask keeps its latest words; the first is then "…".
pub fn shown(words: &[(String, bool)]) -> Vec<(String, bool)> {
    let skipped = words.len().saturating_sub(WORDS);
    let mut shown: Vec<(String, bool)> = Vec::new();
    if skipped > 0 {
        shown.push(("…".into(), false));
    }
    shown.extend(words[skipped..].iter().cloned());
    shown
}

/// Which side of the call window the bubble sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Above,
    Below,
}

/// Where the bubble's window goes: beside the call window, level with the speaker's tile.
#[derive(Clone, Debug, PartialEq)]
pub struct Spot {
    pub side: Side,
    /// From the monitor's top-left corner.
    pub left: f32,
    pub top: f32,
    /// The monitor's output name.
    pub output: String,
}

/// The bubble's place beside a call window at `window` (from its monitor's corner) on a
/// monitor of `monitor` size, for the tile whose middle is `tile` in the window. Beside
/// a column of tiles, on the side with more room; above or below a row of them.
pub fn beside(
    window: super::hypr::Rect,
    monitor: (f64, f64),
    tile: (f32, f32),
    row: bool,
) -> (Side, f32, f32) {
    let (x, y) = (window.x as f32, window.y as f32);
    let (width, height) = (window.width as f32, window.height as f32);
    let (side, left, top) = if row {
        let left = x + tile.0 - WIDTH / 2.;
        if y + height / 2. > monitor.1 as f32 / 2. {
            (Side::Above, left, y - GAP - HEIGHT)
        } else {
            (Side::Below, left, y + height + GAP)
        }
    } else {
        let top = y + tile.1 - HEIGHT / 2.;
        if x + width / 2. > monitor.0 as f32 / 2. {
            (Side::Left, x - GAP - WIDTH, top)
        } else {
            (Side::Right, x + width + GAP, top)
        }
    };
    (
        side,
        left.clamp(0., (monitor.0 as f32 - WIDTH).max(0.)),
        top.clamp(0., (monitor.1 as f32 - HEIGHT).max(0.)),
    )
}

pub struct HeardView {
    session: Entity<Session>,
    /// `None`: there is no call window on screen to sit beside, so the bubble is at the
    /// top of the screen and says who it heard.
    side: Option<Side>,
}

impl HeardView {
    fn new(session: Entity<Session>, side: Option<Side>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self { session, side }
    }
}

/// The little point on the bubble's edge towards the tile: a triangle of thin strips.
fn pointer(side: Side, color: Hsla) -> gpui_kit::Div {
    let strips = (0..POINT as usize).map(|step| {
        // Widest against the bubble.
        let across = 2. * (POINT - step as f32);
        let strip = div().flex_shrink_0().bg(color);
        match side {
            Side::Left | Side::Right => strip.w(px(1.)).h(px(across)),
            Side::Above | Side::Below => strip.h(px(1.)).w(px(across)),
        }
    });
    let point = div().flex().flex_shrink_0().items_center();
    match side {
        Side::Left => point.flex_row().children(strips),
        Side::Right => point.flex_row_reverse().children(strips),
        Side::Above => point.flex_col().children(strips),
        Side::Below => point.flex_col_reverse().children(strips),
    }
}

impl Render for HeardView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let Some(heard) = self.session.read(cx).heard.clone() else {
            return div().size_full();
        };
        let lit = super::palette::nth(1, cx);
        let said = shown(&heard.words).into_iter().map(|(word, wake)| {
            let word_box = div().child(word);
            if wake {
                word_box.text_color(lit).font_weight(FontWeight::BOLD)
            } else {
                word_box
            }
        });
        let fill = theme.background.opacity(0.94);
        let mut bubble = div()
            .max_w(px(WIDTH - POINT))
            .px(px(10.))
            .py(px(7.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .rounded(px(super::style::RADIUS))
            .bg(fill)
            .border_1()
            .border_color(theme.foreground.opacity(0.14));
        if self.side.is_none() {
            bubble = bubble.child(
                div()
                    .text_size(px(10.))
                    .text_color(theme.secondary.opacity(0.75))
                    .child(if heard.yours {
                        "Heard you".to_owned()
                    } else {
                        format!("Heard {}", heard.name)
                    }),
            );
        }
        let bubble = bubble.child(
            div()
                .flex()
                .flex_wrap()
                .gap_x(px(4.))
                .text_size(px(12.))
                .line_height(px(17.))
                .text_color(theme.bright)
                .children(said),
        );
        // The window is a fixed size and see-through. The bubble is as big as its words,
        // against the edge nearest the tile.
        let root = div()
            .size_full()
            .flex()
            .items_center()
            .font_family(theme.font.clone());
        match self.side {
            None => root.flex_col().justify_start().child(bubble),
            Some(side) => {
                let point = pointer(side, fill);
                match side {
                    Side::Left => root.flex_row().justify_end().child(bubble).child(point),
                    Side::Right => root.flex_row().justify_start().child(point).child(bubble),
                    Side::Above => root.flex_col().justify_end().child(bubble).child(point),
                    Side::Below => root.flex_col().justify_start().child(point).child(bubble),
                }
            }
        }
    }
}

/// Open the bubble at `spot`, beside the call window, or with none at the top of the
/// screen under the bar. Clicks go through it. Falls back to an ordinary small window on
/// compositors without layer-shell (the screenshot harness), drawn as for `side`.
pub fn open(
    session: Entity<Session>,
    spot: Option<Spot>,
    side: Option<Side>,
    cx: &mut gpui_kit::App,
) -> Option<WindowHandle<HeardView>> {
    let bounds = Some(gpui_kit::WindowBounds::Windowed(Bounds {
        origin: Default::default(),
        size: size(px(WIDTH), px(HEIGHT)),
    }));
    let side = spot.as_ref().map(|spot| spot.side).or(side);
    let display_id = spot
        .as_ref()
        .and_then(|spot| super::overlay::display_named(&spot.output, cx));
    // Beside the call window only on its own monitor; otherwise under the bar.
    let spot = spot.filter(|_| display_id.is_some());
    let (anchor, margin, exclusive_zone) = match &spot {
        Some(spot) => (
            Anchor::TOP | Anchor::LEFT,
            (px(spot.top), px(0.), px(0.), px(spot.left)),
            // Measured from the monitor's corner, not from under the bar.
            Some(px(-1.)),
        ),
        None => (Anchor::TOP, (px(8.), px(0.), px(0.), px(0.)), None),
    };
    let layer = gpui_kit::WindowOptions {
        window_bounds: bounds,
        kind: gpui_kit::WindowKind::LayerShell(LayerShellOptions {
            // The call card's namespace, which the Hyprland rules already keep out of
            // your screen share.
            namespace: "hopp-call".into(),
            layer: Layer::Overlay,
            anchor,
            exclusive_zone,
            exclusive_edge: None,
            margin: Some(margin),
            keyboard_interactivity: KeyboardInteractivity::None,
        }),
        display_id,
        app_id: Some("hopp.heard".into()),
        focus: false,
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let layer_session = session.clone();
    let layer_side = side.filter(|_| spot.is_some());
    let handle = match cx.open_window(layer, |_, cx| {
        cx.new(|cx| HeardView::new(layer_session, layer_side, cx))
    }) {
        Ok(handle) => handle,
        Err(_) => cx
            .open_window(
                gpui_kit::WindowOptions {
                    window_bounds: bounds,
                    app_id: Some("hopp.heard".into()),
                    focus: false,
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| HeardView::new(session, side, cx)),
            )
            .ok()?,
    };
    let _ = handle.update(cx, |_, window, _| window.set_input_region(Some(&[])));
    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(since: Instant, updated: Instant) -> Heard {
        Heard {
            key: "you".into(),
            name: "Kieran".into(),
            yours: true,
            words: vec![("Jeepers".into(), true)],
            since,
            updated,
        }
    }

    #[test]
    fn the_light_laps_twice_quickly_then_slows_and_goes_after_the_hold() {
        let start = Instant::now();
        let ms = Duration::from_millis;
        let talking = |at: u64| heard(start, start + ms(at)).glow(start + ms(at)).unwrap();
        assert_eq!(talking(0).strength, 0., "fades in");
        assert_eq!(talking(1_000).strength, 1.);
        assert!((talking(350).turn - 0.5).abs() < 0.01, "half a quick lap");
        assert!(
            (talking(1_400 + 900).turn - 0.5).abs() < 0.01,
            "half a slow lap"
        );
        // Quiet since the word: it dims at the end of the hold, then goes.
        let quiet = heard(start, start);
        let late = quiet.glow(start + HOLD - ms(300)).unwrap();
        assert!((late.strength - 0.5).abs() < 0.01);
        assert_eq!(quiet.glow(start + HOLD), None);
        assert!(quiet.over(start + HOLD));
        assert!(!quiet.over(start + HOLD - ms(1)));
    }

    #[test]
    fn the_light_stays_on_the_tiles_edge_all_the_way_round() {
        for (side, radius) in [(140., 9.), (64., 32.), (100., 0.)] {
            let mut last = edge_point(side, radius, 0.);
            assert_eq!(last, (radius, 0.));
            let (mut travelled, steps) = (0., 2_000);
            for step in 1..=steps {
                let (x, y) = edge_point(side, radius, step as f32 / steps as f32);
                assert!((0. ..=side).contains(&x) && (0. ..=side).contains(&y));
                let moved = ((x - last.0).powi(2) + (y - last.1).powi(2)).sqrt();
                assert!(moved < side * 4. / steps as f32 * 1.5, "no jumps: {moved}");
                travelled += moved;
                last = (x, y);
            }
            let perimeter = 4. * (side - 2. * radius) + std::f32::consts::TAU * radius;
            assert!(
                (travelled - perimeter).abs() < 0.5,
                "{travelled} of {perimeter}"
            );
        }
        // Halfway round the first corner, the top right.
        let turn =
            (80. + std::f32::consts::FRAC_PI_4 * 10.) / (4. * 80. + std::f32::consts::TAU * 10.);
        let (x, y) = edge_point(100., 10., turn);
        assert!(
            (x - 97.07).abs() < 0.1 && (y - 2.93).abs() < 0.1,
            "{x}, {y}"
        );
    }

    #[test]
    fn the_bubble_sits_beside_the_call_window_level_with_the_tile() {
        use crate::ui::hypr::Rect;
        let monitor = (3200., 1350.);
        let rect = |x, y, width, height| Rect {
            x,
            y,
            width,
            height,
        };
        // Bottom right, the default: to its left, its middle level with the tile's.
        let column = rect(2858., 322., 332., 1018.);
        assert_eq!(
            beside(column, monitor, (166., 486.), false),
            (Side::Left, 2858. - GAP - WIDTH, 322. + 486. - HEIGHT / 2.)
        );
        // Dragged to the left of the screen: to its right.
        let (side, left, _) = beside(rect(10., 322., 332., 1018.), monitor, (166., 486.), false);
        assert_eq!((side, left), (Side::Right, 342. + GAP));
        // A row at the bottom: above the tile. At the top: below it.
        let row = rect(2764., 1216., 426., 124.);
        assert_eq!(
            beside(row, monitor, (62., 62.), true),
            (Side::Above, 2764. + 62. - WIDTH / 2., 1216. - GAP - HEIGHT)
        );
        let (side, _, top) = beside(rect(2764., 47., 426., 124.), monitor, (62., 62.), true);
        assert_eq!((side, top), (Side::Below, 171. + GAP));
        // Never off the screen.
        let (_, left, top) = beside(rect(3100., 1300., 426., 124.), monitor, (400., 62.), true);
        assert_eq!((left, top), (3200. - WIDTH, 1300. - GAP - HEIGHT));
    }

    #[test]
    fn a_long_ask_keeps_its_latest_words() {
        let short = vec![("Jeepers,".to_owned(), true), ("why?".to_owned(), false)];
        assert_eq!(shown(&short), short);
        let long: Vec<(String, bool)> = (0..WORDS + 5)
            .map(|word| (word.to_string(), false))
            .collect();
        let shown = shown(&long);
        assert_eq!(shown.len(), WORDS + 1);
        assert_eq!(shown[0].0, "…");
        assert_eq!(shown[1].0, "5");
    }
}
