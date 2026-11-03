//! Cursors drawn on an iced canvas frame, from sprites rendered when they're created.
//!
//! The arrow, hand, pencil and name tag are separate images, so the name can fade on its own
//! while the cursor rests. Someone else's cursor plays back a moment behind the network so it
//! moves evenly: see [`Pointer`].

use crate::utils::geometry::Position;
use hopp_marks::pointer::{self, Pointer};
use hopp_marks::sprites::{self, Sprite};
use iced::widget::canvas::Frame;
use iced::Rectangle;
use iced_core::image::{Handle, Image};
use std::time::{Duration, Instant};

/// Device pixels per logical pixel the sprites are rendered at: exact on Retina, and halved
/// cleanly on a 1x screen.
const SPRITE_SCALE: f32 = 2.0;

/// Cursor display mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorMode {
    /// Normal arrow cursor
    Normal,
    /// Pointer/hand cursor
    Pointer,
    /// Pencil cursor for drawing mode
    Pencil,
}

/// One sprite, ready to draw. Size and offset are in logical pixels.
#[derive(Debug)]
struct CursorImage {
    handle: Handle,
    size: (f32, f32),
    offset: (f32, f32),
}

impl CursorImage {
    fn new(sprite: Sprite) -> Self {
        Self {
            size: (
                sprite.width as f32 / SPRITE_SCALE,
                sprite.height as f32 / SPRITE_SCALE,
            ),
            offset: (
                sprite.offset.0 / SPRITE_SCALE,
                sprite.offset.1 / SPRITE_SCALE,
            ),
            handle: Handle::from_rgba(sprite.width, sprite.height, sprite.rgba),
        }
    }

    fn draw(&self, frame: &mut Frame, x: f32, y: f32, opacity: f32) {
        frame.draw_image(
            Rectangle {
                x: x + self.offset.0,
                y: y + self.offset.1,
                width: self.size.0,
                height: self.size.1,
            },
            Image::new(self.handle.clone()).opacity(opacity).snap(true),
        );
    }
}

#[derive(Debug)]
struct CursorImages {
    arrow: CursorImage,
    hand: CursorImage,
    pencil: CursorImage,
    tag: CursorImage,
}

impl CursorImages {
    fn render(rgb: [u8; 3], name: &str) -> Option<Self> {
        Some(Self {
            arrow: CursorImage::new(sprites::arrow(rgb, SPRITE_SCALE)?),
            hand: CursorImage::new(sprites::hand(rgb, SPRITE_SCALE)?),
            pencil: CursorImage::new(sprites::pencil(rgb, SPRITE_SCALE)?),
            tag: CursorImage::new(sprites::tag(name, rgb, SPRITE_SCALE)?),
        })
    }
}

/// Where a cursor is showing right now, and how visibly.
#[derive(Debug, Clone, Copy)]
struct Shown {
    position: Position,
    opacity: f32,
    name_opacity: f32,
}

/// An image-based cursor for rendering on iced canvas frames.
#[derive(Debug)]
pub struct Cursor {
    /// Visible name displayed on the cursor
    visible_name: String,
    images: CursorImages,
    /// Your own cursor follows your hand, with nothing to smooth out.
    follows_instantly: bool,
    /// Where it's been sent, while it's showing
    pointer: Option<Pointer>,
    /// Current cursor display mode
    mode: CursorMode,
}

impl Cursor {
    /// Creates a new `Cursor` with the given color and name.
    pub fn new(rgb: [u8; 3], name: &str, follows_instantly: bool) -> Option<Self> {
        Some(Self {
            visible_name: name.to_string(),
            images: CursorImages::render(rgb, name)?,
            follows_instantly,
            pointer: None,
            mode: CursorMode::Normal,
        })
    }

    /// Returns the visible name displayed on this cursor.
    pub fn visible_name(&self) -> &str {
        &self.visible_name
    }

    pub fn set_colour(&mut self, rgb: [u8; 3]) {
        match CursorImages::render(rgb, &self.visible_name) {
            Some(images) => self.images = images,
            None => log::warn!(
                "Cursor::set_colour: couldn't render the cursor for {}",
                self.visible_name
            ),
        }
    }

    /// Sets the cursor display mode.
    pub fn set_mode(&mut self, mode: CursorMode) {
        self.mode = mode;
    }

    /// Sets the cursor position. The overlay passes it again every frame, so a position that
    /// hasn't changed isn't a move.
    pub fn set_position(&mut self, position: Option<Position>) {
        let Some(position) = position else {
            self.pointer = None;
            return;
        };
        let point = (position.x, position.y);
        match &mut self.pointer {
            Some(pointer) if pointer.target() == Some(point) => {}
            Some(pointer) if pointer.visible(Instant::now()) => pointer.moved_to(point),
            pointer => *pointer = Some(Pointer::new(point)),
        }
    }

    /// Hides the cursor once it has rested long enough to have faded out.
    pub fn hide_if_gone(&mut self, now: Instant) {
        if self
            .pointer
            .as_ref()
            .is_some_and(|pointer| !pointer.visible(now))
        {
            self.pointer = None;
        }
    }

    /// Whether drawing it again would show something different.
    pub fn is_animating(&self, now: Instant) -> bool {
        self.pointer
            .as_ref()
            .is_some_and(|pointer| pointer.animating(now))
    }

    fn shown(&self, pen_tip: Option<Position>, now: Instant) -> Option<Shown> {
        let pointer = self.pointer.as_ref()?;
        let drawing = self.mode == CursorMode::Pencil;
        let (position, idle) = match pen_tip.filter(|_| drawing) {
            // While they draw, the pencil sits on the tip of their ink.
            Some(tip) => (tip, Duration::ZERO),
            None => {
                let (x, y) = if self.follows_instantly {
                    pointer.target()?
                } else {
                    pointer.position(now)
                };
                (
                    Position { x, y },
                    now.saturating_duration_since(pointer.moved),
                )
            }
        };
        Some(Shown {
            position,
            opacity: pointer::pointer_opacity(idle),
            name_opacity: pointer::name_opacity(idle, drawing),
        })
    }

    /// Draws the cursor onto an iced canvas frame.
    pub fn draw(
        &self,
        frame: &mut Frame,
        translate: &dyn Fn(Position) -> Position,
        pen_tip: Option<Position>,
        now: Instant,
    ) {
        let Some(shown) = self.shown(pen_tip, now) else {
            return;
        };
        if shown.opacity <= 0. {
            return;
        }
        let position = translate(shown.position);
        let (x, y) = (position.x as f32, position.y as f32);
        let (image, tag_offset) = match self.mode {
            CursorMode::Normal => (&self.images.arrow, sprites::tag_offset_arrow()),
            CursorMode::Pointer => (&self.images.hand, sprites::tag_offset_hand()),
            CursorMode::Pencil => (&self.images.pencil, sprites::tag_offset_pencil()),
        };
        image.draw(frame, x, y, shown.opacity);
        if shown.name_opacity > 0. {
            let slide = pointer::NAME_SLIDE * (1. - shown.name_opacity);
            self.images.tag.draw(
                frame,
                x + tag_offset.0,
                y + tag_offset.1 + slide,
                shown.name_opacity,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLUE: [u8; 3] = [0x0a, 0x84, 0xff];

    fn at(x: f64, y: f64) -> Position {
        Position { x, y }
    }

    fn cursor() -> Cursor {
        let mut cursor = Cursor::new(BLUE, "Mira", false).expect("cursor");
        cursor.set_position(Some(at(0.5, 0.5)));
        cursor
    }

    #[test]
    fn a_position_sent_again_is_not_a_move() {
        let mut cursor = cursor();
        let moved = cursor.pointer.as_ref().unwrap().moved;
        std::thread::sleep(Duration::from_millis(2));
        cursor.set_position(Some(at(0.5, 0.5)));
        assert_eq!(cursor.pointer.as_ref().unwrap().moved, moved);
        cursor.set_position(Some(at(0.6, 0.5)));
        assert!(cursor.pointer.as_ref().unwrap().moved > moved);
    }

    #[test]
    fn a_resting_cursor_loses_its_name_then_goes() {
        let mut cursor = cursor();
        let start = cursor.pointer.as_ref().unwrap().moved;
        let shown = |cursor: &Cursor, ms| {
            cursor
                .shown(None, start + Duration::from_millis(ms))
                .expect("showing")
        };
        assert_eq!(shown(&cursor, 1_000).name_opacity, 1.);
        assert_eq!(shown(&cursor, 3_000).name_opacity, 0.);
        assert_eq!(shown(&cursor, 3_000).opacity, 1.);
        assert_eq!(shown(&cursor, 8_400).opacity, 0.);

        cursor.hide_if_gone(start + Duration::from_millis(8_000));
        assert!(cursor.pointer.is_some());
        cursor.hide_if_gone(start + Duration::from_millis(8_400));
        assert!(cursor.pointer.is_none());
        assert!(cursor.shown(None, start).is_none());
    }

    #[test]
    fn it_comes_back_where_it_is_sent_rather_than_gliding_there() {
        let mut cursor = cursor();
        cursor.set_position(None);
        cursor.set_position(Some(at(0.9, 0.9)));
        let shown = cursor.shown(None, Instant::now()).expect("showing");
        assert_eq!(shown.position.x, 0.9);
    }

    #[test]
    fn the_pencil_sits_on_the_tip_of_the_ink_with_its_name() {
        let mut cursor = cursor();
        let start = cursor.pointer.as_ref().unwrap().moved;
        let later = start + Duration::from_secs(5);
        let tip = Some(at(0.2, 0.3));

        // Only the pencil follows the ink.
        let arrow = cursor.shown(tip, later).expect("showing");
        assert_eq!(arrow.position.x, 0.5);
        assert_eq!(arrow.name_opacity, 0.);

        cursor.set_mode(CursorMode::Pencil);
        let drawing = cursor.shown(tip, later).expect("showing");
        assert_eq!((drawing.position.x, drawing.position.y), (0.2, 0.3));
        assert_eq!(drawing.name_opacity, 1.);

        // Between strokes the pencil stays with its name, until the cursor goes.
        let between = cursor.shown(None, later).expect("showing");
        assert_eq!(between.position.x, 0.5);
        assert_eq!(between.name_opacity, 1.);
        let gone = cursor
            .shown(None, start + Duration::from_millis(8_400))
            .expect("showing");
        assert_eq!(gone.name_opacity, 0.);
    }

    #[test]
    fn your_own_cursor_does_not_lag() {
        let mut own = Cursor::new(BLUE, "Me", true).expect("cursor");
        own.set_position(Some(at(0.1, 0.1)));
        std::thread::sleep(Duration::from_millis(300));
        own.set_position(Some(at(0.9, 0.9)));
        let now = Instant::now();
        assert_eq!(own.shown(None, now).expect("showing").position.x, 0.9);

        // Someone else's starts from where it rested.
        let mut theirs = Cursor::new(BLUE, "Mira", false).expect("cursor");
        theirs.set_position(Some(at(0.1, 0.1)));
        std::thread::sleep(Duration::from_millis(300));
        theirs.set_position(Some(at(0.9, 0.9)));
        let now = Instant::now();
        assert!(theirs.shown(None, now).expect("showing").position.x < 0.5);
    }

    #[test]
    fn a_new_colour_keeps_where_it_is() {
        let mut cursor = cursor();
        let moved = cursor.pointer.as_ref().unwrap().moved;
        cursor.set_mode(CursorMode::Pencil);
        cursor.set_colour([0xff, 0x3b, 0x30]);
        assert_eq!(cursor.pointer.as_ref().unwrap().moved, moved);
        assert_eq!(cursor.mode, CursorMode::Pencil);
    }

    /// Paints through the real renderer, on the real GPU, with no window.
    struct Screen {
        renderer: iced::Renderer,
    }

    impl Screen {
        fn new() -> Self {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
            let adapter = pollster::block_on(
                instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
            )
            .expect("adapter");
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                    .expect("device");
            // What the windows choose on a Mac.
            let format = wgpu::TextureFormat::Bgra8Unorm;
            let engine = iced_wgpu::Engine::new(
                &adapter,
                device,
                queue,
                format,
                None,
                iced_wgpu::graphics::Shell::headless(),
            );
            crate::components::fonts::load_fonts();
            Self {
                renderer: iced::Renderer::Primary(iced_wgpu::Renderer::new(
                    engine,
                    crate::components::fonts::GEIST_REGULAR,
                    iced::Pixels::from(13),
                )),
            }
        }

        fn paint(
            &mut self,
            size: (u32, u32),
            scale: f32,
            background: iced::Color,
            paint: impl FnOnce(&mut Frame),
        ) -> image::RgbaImage {
            use iced_wgpu::core::Renderer as _;
            use iced_wgpu::graphics::geometry::Renderer as _;

            let viewport = iced_wgpu::graphics::Viewport::with_physical_size(
                iced::Size::new(size.0, size.1),
                scale,
            );
            let logical = viewport.logical_size();
            self.renderer.reset(Rectangle::with_size(logical));
            let mut frame = Frame::new(&self.renderer, logical);
            paint(&mut frame);
            self.renderer.draw_geometry(frame.into_geometry());
            let iced::Renderer::Primary(renderer) = &mut self.renderer else {
                panic!("not on the GPU");
            };
            let pixels = renderer.screenshot(&viewport, background);
            image::RgbaImage::from_raw(size.0, size.1, pixels).expect("pixels")
        }
    }

    fn near(pixel: &image::Rgba<u8>, expected: [u8; 3], tolerance: i16) -> bool {
        (0..3).all(|i| (i16::from(pixel[i]) - i16::from(expected[i])).abs() <= tolerance)
    }

    /// Where the cursors land, whether they fade and whether they stay sharp, as actually
    /// rendered. Needs a GPU, so run it by hand and look at `/tmp/hopp-cursors.png`:
    /// `cargo test --target aarch64-apple-darwin --lib cursors_through_the_gpu -- --ignored`
    #[test]
    #[ignore]
    fn cursors_through_the_gpu() {
        const DARK: [u8; 3] = [26, 27, 38];
        let background = iced::Color::from_rgb8(DARK[0], DARK[1], DARK[2]);
        let same = |position: Position| position;
        let size = (400, 200);
        let mut screen = Screen::new();
        let cursor = cursor();
        let start = cursor.pointer.as_ref().unwrap().moved;
        let at_time = |ms| start + Duration::from_millis(ms);
        // A cursor drawn with its tip at this point, wherever `cursor` thinks it is.
        let translate_to = |x: f64, y: f64| move |_: Position| Position { x, y };

        // Retina: the arrow's body and the tag's padding land where they should, in colour.
        let fresh = screen.paint(size, 2., background, |frame| {
            cursor.draw(frame, &translate_to(40., 40.), None, at_time(100));
        });
        fresh.save("/tmp/hopp-cursors.png").expect("save");
        let inside_arrow = fresh.get_pixel(86, 95);
        let inside_tag = fresh.get_pixel(104, 129);
        assert!(near(inside_arrow, BLUE, 3), "arrow {inside_arrow:?}");
        assert!(near(inside_tag, BLUE, 3), "tag {inside_tag:?}");

        // The name goes first...
        let resting = screen.paint(size, 2., background, |frame| {
            cursor.draw(frame, &translate_to(40., 40.), None, at_time(3_000));
        });
        assert!(near(resting.get_pixel(86, 95), BLUE, 3));
        assert!(near(resting.get_pixel(104, 129), DARK, 3), "name gone");

        // ...then the cursor fades, halfway at 8.2 s...
        let fading = screen.paint(size, 2., background, |frame| {
            cursor.draw(frame, &translate_to(40., 40.), None, at_time(8_200));
        });
        let half = [0, 1, 2].map(|i| ((u16::from(BLUE[i]) + u16::from(DARK[i])) / 2) as u8);
        let pixel = fading.get_pixel(86, 95);
        assert!(near(pixel, half, 12), "halfway {pixel:?} vs {half:?}");

        // ...and it's gone.
        let gone = screen.paint(size, 2., background, |frame| {
            cursor.draw(frame, &translate_to(40., 40.), None, at_time(8_400));
        });
        assert!(gone.pixels().all(|pixel| near(pixel, DARK, 0)));

        // Between pixels it's as sharp as on one: the same picture, on the nearest whole pixel.
        for (point, shift) in [(40.2, 0), (40.3, 1)] {
            let between = screen.paint(size, 2., background, |frame| {
                cursor.draw(frame, &translate_to(point, point), None, at_time(100));
            });
            let same_picture = (0..size.1 - shift).all(|y| {
                (0..size.0 - shift)
                    .all(|x| fresh.get_pixel(x, y) == between.get_pixel(x + shift, y + shift))
            });
            assert!(same_picture, "blurred at {point}");
        }

        // A 1x screen: half the pixels, the same place.
        let plain = screen.paint((200, 100), 1., background, |frame| {
            cursor.draw(frame, &translate_to(40., 40.), None, at_time(100));
        });
        let inside_arrow = plain.get_pixel(43, 47);
        assert!(near(inside_arrow, BLUE, 3), "1x arrow {inside_arrow:?}");

        // The pencil's lead is on the point, not the corner of its picture.
        let mut pencil = Cursor::new(BLUE, "Mira", false).expect("cursor");
        pencil.set_mode(CursorMode::Pencil);
        pencil.set_position(Some(at(0.5, 0.5)));
        let drawn = screen.paint(size, 2., background, |frame| {
            pencil.draw(frame, &same, Some(at(100., 80.)), Instant::now());
        });
        // Just up and right of the lead is the pencil's body: (2.6, 19.4) to (5.5, 16.5).
        let body = drawn.get_pixel(205, 155);
        assert!(!near(body, DARK, 20), "pencil body {body:?}");
        let clear = drawn.get_pixel(190, 175);
        assert!(
            near(clear, DARK, 40),
            "below and left of the lead {clear:?}"
        );
    }
}
