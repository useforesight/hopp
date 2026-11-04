//! `hopp --marks-demo`: what people's cursors and ink look like over your screen, without
//! a call. Two made-up teammates over the focused monitor for a few seconds: one moves
//! their cursor about (fed in uneven bursts, as the network delivers it) and clicks, the
//! other draws. Clicks go through to your apps underneath.

use std::{
    rc::Rc,
    time::{Duration, Instant},
};

use gpui_kit::{
    AppContext, Bounds, Context, IntoElement, ParentElement, Pixels, Render, Styled, Window,
    canvas, div,
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
};

use super::{
    marks::{self, Pointer, Scene, ScenePointer, Tool},
    session::{Ripple, Stroke},
};
use crate::protocol::Point;

const POINTER: &str = "demo:jonas";
const PEN: &str = "demo:mira";
/// How long one drawing takes, and how long it stays before it fades.
const DRAW: f32 = 3.2;
const HOLD: f32 = 1.4;

struct MarksDemo {
    start: Instant,
    pointer: Pointer,
    pen: Pointer,
    next_update: Instant,
    seed: u64,
    strokes: Vec<Stroke>,
    drawing: Option<usize>,
    ripples: Vec<Ripple>,
    stamps: Vec<marks::Stamp>,
    next_click: f32,
    sprites: Rc<std::cell::RefCell<marks::Sprites>>,
}

/// Where the moving cursor is `t` seconds in: a slow loop round the screen.
fn wander(t: f32) -> Point {
    Point {
        x: f64::from(0.5 + 0.28 * (t * 0.55).sin() + 0.06 * (t * 1.7).cos()),
        y: f64::from(0.5 + 0.22 * (t * 0.83).sin()),
    }
}

/// The pen `s` of the way through drawing a looped underline.
fn handwriting(s: f32, round: u32) -> Point {
    let a = s * std::f32::consts::TAU * 2.4;
    let left = 0.18 + 0.05 * (round % 3) as f32;
    Point {
        x: f64::from(left + 0.42 * s + 0.035 * a.cos()),
        y: f64::from(0.3 + 0.12 * (round % 2) as f32 + 0.06 * a.sin() - 0.04 * s),
    }
}

impl MarksDemo {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            pointer: Pointer::new(wander(0.)),
            pen: Pointer::new(handwriting(0., 0)),
            next_update: now,
            seed: 0x9e37_79b9_7f4a_7c15,
            strokes: Vec::new(),
            drawing: None,
            ripples: Vec::new(),
            // A note someone left, and one still listening.
            stamps: vec![
                marks::Stamp {
                    id: 1,
                    number: 1,
                    owner: PEN.into(),
                    name: "Mira Chen".into(),
                    point: Point { x: 0.62, y: 0.62 },
                    text: "Clamp the target here".into(),
                    draft: String::new(),
                    listening: false,
                    voice: false,
                    placed: now,
                    removed: None,
                },
                marks::Stamp {
                    id: 2,
                    number: 2,
                    owner: POINTER.into(),
                    name: "Jonas Weber".into(),
                    point: Point { x: 0.3, y: 0.7 },
                    text: String::new(),
                    draft: String::new(),
                    listening: true,
                    voice: false,
                    placed: now,
                    removed: None,
                },
            ],
            next_click: 1.5,
            sprites: Default::default(),
        }
    }

    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed % 10_000) as f32 / 10_000.
    }

    /// Catch the made-up network up to `now`.
    fn advance(&mut self, now: Instant) {
        // Cursor updates every 8 to 25 ms, now and then two at once.
        while self.next_update <= now {
            let t = self.next_update.duration_since(self.start).as_secs_f32();
            self.pointer.moved_to(wander(t));
            let gap = if self.random() < 0.12 {
                0.
            } else {
                0.008 + 0.017 * self.random()
            };
            self.next_update += Duration::from_secs_f32(gap.max(0.001));
        }
        let t = now.duration_since(self.start).as_secs_f32();
        if t >= self.next_click {
            self.ripples.push(Ripple {
                sender: POINTER.into(),
                point: self.pointer.position(now),
                at: now,
            });
            self.next_click += 2.6;
        }
        self.ripples
            .retain(|ripple| now.duration_since(ripple.at) < Duration::from_secs(2));
        // One drawing after another: drawn, held, then faded as its owner clears it.
        let cycle = DRAW + HOLD;
        let round = (t / cycle) as u32;
        let into = t - round as f32 * cycle;
        if self
            .drawing
            .is_none_or(|index| self.strokes[index].id != u64::from(round))
        {
            for stroke in &mut self.strokes {
                if stroke.fading.is_none() {
                    stroke.fading = Some(now);
                }
            }
            self.strokes.retain(|stroke| !stroke.faded(now));
            self.strokes.push(Stroke {
                sender: PEN.into(),
                id: u64::from(round),
                points: Vec::new(),
                ended: None,
                fading: None,
            });
            self.drawing = Some(self.strokes.len() - 1);
        }
        if let Some(stroke) = self.drawing.and_then(|index| self.strokes.get_mut(index)) {
            let s = (into / DRAW).min(1.);
            // Points as a pen sends them, about every 12 ms.
            let wanted = (s * DRAW / 0.012) as usize + 1;
            while stroke.points.len() < wanted {
                let k = stroke.points.len() as f32 * 0.012 / DRAW;
                let point = handwriting(k.min(1.), round);
                stroke.points.push(point);
                self.pen.moved_to(point);
            }
            if s >= 1. && stroke.ended.is_none() {
                stroke.ended = Some(now);
            }
        }
    }

    fn scene(&self) -> Scene {
        let pen = self.drawing.and_then(|index| self.strokes.get(index));
        let pen_tip = pen
            .filter(|stroke| stroke.ended.is_none())
            .and_then(|stroke| stroke.points.last().copied());
        Scene {
            paths: self.strokes.clone(),
            ripples: self.ripples.clone(),
            stamps: self.stamps.clone(),
            pointers: vec![
                ScenePointer {
                    sender: POINTER.into(),
                    pointer: self.pointer.clone(),
                    name: "Jonas Weber".into(),
                    tool: Tool::Arrow,
                    pen_tip: None,
                },
                ScenePointer {
                    sender: PEN.into(),
                    pointer: self.pen.clone(),
                    name: "Mira Chen".into(),
                    tool: Tool::Pencil,
                    pen_tip,
                },
            ],
            colours: Rc::new(|sender: &str| {
                if sender == PEN {
                    crate::ink::PALETTE[4]
                } else {
                    crate::ink::PALETTE[1]
                }
            }),
        }
    }
}

impl Render for MarksDemo {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.advance(Instant::now());
        let scene = self.scene();
        let sprites = self.sprites.clone();
        div().size_full().child(
            canvas(
                |bounds, _, _| bounds,
                move |_, bounds: Bounds<Pixels>, window, cx| {
                    let mut sprites = sprites.borrow_mut();
                    marks::paint_scene(window, cx, &mut sprites, &scene, bounds, bounds);
                    window.request_animation_frame();
                },
            )
            .size_full(),
        )
    }
}

/// Show the demo over the focused monitor for `seconds`, then exit.
pub fn run(seconds: u64) {
    gpui_kit::application().run(move |cx| {
        gpui_omarchy::init(cx);
        let options = gpui_kit::WindowOptions {
            kind: gpui_kit::WindowKind::LayerShell(LayerShellOptions {
                namespace: "hopp-marks-demo".into(),
                layer: Layer::Overlay,
                anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                exclusive_zone: Some(gpui_kit::px(-1.)),
                exclusive_edge: None,
                margin: None,
                keyboard_interactivity: KeyboardInteractivity::None,
            }),
            app_id: Some("hopp.marks-demo".into()),
            focus: false,
            window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };
        if let Ok(handle) = cx.open_window(options, |_, cx| {
            cx.new(|cx| {
                let view = MarksDemo::new();
                cx.notify();
                view
            })
        }) {
            let _ = handle.update(cx, |_, window, _| window.set_input_region(Some(&[])));
        }
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_secs(seconds))
                .await;
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}
