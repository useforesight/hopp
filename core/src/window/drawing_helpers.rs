use std::time::{Duration, Instant};

use iced::widget::{canvas, container, text, Space};
use iced::{Alignment, Background, Border, Color, Length, Padding, Rectangle, Theme};
use iced_wgpu::core::mouse;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use fontdb::Database;
use resvg::{tiny_skia, usvg};

use crate::components::fonts::GEIST_REGULAR;
use crate::graphics::graphics_context::click_animation::ClickAnimationRenderer;
use crate::graphics::graphics_context::participant::ParticipantsManager;
use crate::utils::geometry::Position;
use hopp_marks::stamps::StampInput;

pub(crate) const LOCAL_PARTICIPANT_IDENTITY: &str = "local";
pub(crate) const CURSOR_LOGICAL_SIZE: f64 = 30.0;

pub(crate) const CURSOR_ICON_PENCIL: &[u8] =
    include_bytes!("../../resources/icons/local-participant-pencil.svg");
pub(crate) const CURSOR_ICON_POINTER: &[u8] =
    include_bytes!("../../resources/icons/local-participant-cursor.svg");
pub(crate) const CURSOR_ICON_POINT: &[u8] =
    include_bytes!("../../resources/icons/local-participant-pointer.svg");

/// How often a straight line being dragged out is sent to others. Each update replaces the line,
/// so it takes a few messages, and the exact end point is always sent when the stroke ends.
const STRAIGHT_LINE_PUBLISH_INTERVAL: Duration = Duration::from_millis(33);

/// What others need to hear about the local participant's stroke. These map to messages every
/// Hopp version understands, so someone on an older version sees the same result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StrokeEvent {
    Start { x: f64, y: f64, path_id: u64 },
    AddPoint { x: f64, y: f64 },
    End { x: f64, y: f64 },
    ClearPaths(Vec<u64>),
}

/// The stroke the local participant is drawing, shared by the window for drawing on someone
/// else's screen and the overlay for drawing on your own.
#[derive(Debug, Default)]
pub(crate) struct LocalStroke {
    path_id: u64,
    start: Option<Position>,
    last_straight_publish: Option<Instant>,
    held_back_line_end: Option<Position>,
}

impl LocalStroke {
    pub(crate) fn is_active(&self) -> bool {
        self.start.is_some()
    }

    pub(crate) fn begin(
        &mut self,
        participants: &mut ParticipantsManager,
        point: Position,
    ) -> StrokeEvent {
        self.path_id += 1;
        self.start = Some(point);
        self.last_straight_publish = None;
        self.held_back_line_end = None;
        participants.draw_start(LOCAL_PARTICIPANT_IDENTITY, point, self.path_id);
        StrokeEvent::Start {
            x: point.x,
            y: point.y,
            path_id: self.path_id,
        }
    }

    pub(crate) fn extend(
        &mut self,
        participants: &mut ParticipantsManager,
        point: Position,
        straight: bool,
    ) -> Vec<StrokeEvent> {
        let Some(start) = self.start else {
            return Vec::new();
        };

        if !straight {
            participants.draw_add_point(LOCAL_PARTICIPANT_IDENTITY, point);
            let mut events = self.publish_held_back_line(start);
            events.push(StrokeEvent::AddPoint {
                x: point.x,
                y: point.y,
            });
            return events;
        }

        participants.draw_straighten(LOCAL_PARTICIPANT_IDENTITY, point);
        if self
            .last_straight_publish
            .is_some_and(|published_at| published_at.elapsed() < STRAIGHT_LINE_PUBLISH_INTERVAL)
        {
            self.held_back_line_end = Some(point);
            return Vec::new();
        }
        self.last_straight_publish = Some(Instant::now());
        self.held_back_line_end = None;

        self.straight_line_events(start, point)
    }

    pub(crate) fn end(
        &mut self,
        participants: &mut ParticipantsManager,
        point: Position,
        straight: bool,
    ) -> Vec<StrokeEvent> {
        let Some(start) = self.start.take() else {
            return Vec::new();
        };

        let mut events = if straight {
            participants.draw_straighten(LOCAL_PARTICIPANT_IDENTITY, point);
            self.restart_events(start)
        } else {
            self.publish_held_back_line(start)
        };
        participants.draw_end(LOCAL_PARTICIPANT_IDENTITY, point);
        events.push(StrokeEvent::End {
            x: point.x,
            y: point.y,
        });
        events
    }

    /// Stops following the pointer. Whatever cancelled the stroke is responsible for its ink.
    pub(crate) fn cancel(&mut self) {
        self.start = None;
    }

    /// Takes back the last stroke, unless one is being drawn.
    pub(crate) fn undo(&mut self, participants: &mut ParticipantsManager) -> Option<StrokeEvent> {
        if self.is_active() {
            return None;
        }
        participants
            .draw_undo_last(LOCAL_PARTICIPANT_IDENTITY)
            .map(|path_id| StrokeEvent::ClearPaths(vec![path_id]))
    }

    /// Freehand ink carries on from the end of the line, so others need its latest position first,
    /// even if the throttle held that update back.
    fn publish_held_back_line(&mut self, start: Position) -> Vec<StrokeEvent> {
        match self.held_back_line_end.take() {
            Some(end) => self.straight_line_events(start, end),
            None => Vec::new(),
        }
    }

    fn straight_line_events(&self, start: Position, end: Position) -> Vec<StrokeEvent> {
        let mut events = self.restart_events(start);
        events.push(StrokeEvent::AddPoint { x: end.x, y: end.y });
        events
    }

    /// There's no message for changing a stroke, so it's cleared and started again.
    fn restart_events(&self, start: Position) -> Vec<StrokeEvent> {
        vec![
            StrokeEvent::ClearPaths(vec![self.path_id]),
            StrokeEvent::Start {
                x: start.x,
                y: start.y,
                path_id: self.path_id,
            },
        ]
    }
}

const DRAWING_HINT_DURATION: Duration = Duration::from_secs(5);
const DRAWING_HINT_FADE_DURATION: Duration = Duration::from_millis(400);

/// A reminder of the drawing controls that shows once when drawing starts, then fades away. It
/// sits above everything without getting in the way, nothing waits for it to go.
#[derive(Debug, Default)]
pub(crate) struct DrawingHint {
    shown_at: Option<Instant>,
}

impl DrawingHint {
    pub(crate) fn show_once(&mut self) {
        self.shown_at.get_or_insert_with(Instant::now);
    }

    fn opacity(&self) -> f32 {
        let Some(shown_at) = self.shown_at else {
            return 0.0;
        };
        let fade_elapsed = shown_at.elapsed().saturating_sub(DRAWING_HINT_DURATION);
        1.0 - (fade_elapsed.as_secs_f32() / DRAWING_HINT_FADE_DURATION.as_secs_f32()).min(1.0)
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.opacity() > 0.0
    }

    pub(crate) fn view<'a, Message: 'a>(
        &self,
        escape_exits: bool,
    ) -> iced::Element<'a, Message, Theme, iced::Renderer> {
        let exit = if escape_exits {
            "Esc to exit  ·  "
        } else {
            ""
        };
        let undo = if cfg!(target_os = "macos") {
            "Cmd+Z"
        } else {
            "Ctrl+Z"
        };
        self.pill(format!(
            "{exit}Right-click to clear  ·  Shift for a straight line  ·  {undo} to undo"
        ))
    }

    pub(crate) fn stamp_view<'a, Message: 'a>(
        &self,
    ) -> iced::Element<'a, Message, Theme, iced::Renderer> {
        self.pill(
            "Click to place a stamp  ·  Type, then press Enter  ·  Esc to cancel  ·  \
             Right-click to clear yours"
                .to_owned(),
        )
    }

    fn pill<'a, Message: 'a>(
        &self,
        controls: String,
    ) -> iced::Element<'a, Message, Theme, iced::Renderer> {
        let opacity = self.opacity();
        if opacity <= 0.0 {
            return Space::new().into();
        }

        container(
            container(text(controls).size(13).font(GEIST_REGULAR).color(Color {
                a: opacity,
                ..Color::WHITE
            }))
            .padding(Padding {
                top: 7.0,
                right: 14.0,
                bottom: 7.0,
                left: 14.0,
            })
            .style(move |_theme: &Theme| container::Style {
                background: Some(Background::Color(Color::from_rgba(
                    0.09,
                    0.09,
                    0.11,
                    0.88 * opacity,
                ))),
                border: Border {
                    radius: 100.0.into(),
                    color: Color::from_rgba(1.0, 1.0, 1.0, 0.14 * opacity),
                    width: 1.0,
                },
                ..Default::default()
            }),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::End)
        .padding(24.0)
        .into()
    }
}

/// What a key press means to a stamp taking words. Shortcuts aren't words, though Option is:
/// that's how a lot of keyboards type things like `#` and `é`.
pub(crate) fn stamp_key(
    key: &Key,
    text: Option<&str>,
    modifiers: ModifiersState,
) -> Option<StampInput> {
    if modifiers.super_key() || modifiers.control_key() {
        return None;
    }
    match key {
        Key::Named(NamedKey::Enter) => Some(StampInput::Finish),
        Key::Named(NamedKey::Escape) => Some(StampInput::Cancel),
        Key::Named(NamedKey::Backspace) => Some(StampInput::Backspace),
        _ => {
            // Arrow and function keys come through as text in the Mac's private-use range.
            let typed: String = text?
                .chars()
                .filter(|character| {
                    !character.is_control() && !('\u{F700}'..='\u{F8FF}').contains(character)
                })
                .collect();
            (!typed.is_empty()).then_some(StampInput::Typed(typed))
        }
    }
}

pub(crate) fn is_undo_shortcut(key: &Key, modifiers: ModifiersState) -> bool {
    let command_held = if cfg!(target_os = "macos") {
        modifiers.super_key()
    } else {
        modifiers.control_key()
    };
    command_held
        && !modifiers.shift_key()
        && matches!(key, Key::Character(character) if character.eq_ignore_ascii_case("z"))
}

pub(crate) fn rasterize_svg_to_rgba(svg_bytes: &[u8], px_size: u32) -> (Vec<u8>, u32, u32) {
    let fontdb = std::sync::Arc::new(Database::new());
    let usvg_options = usvg::Options {
        fontdb,
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(svg_bytes, &usvg_options)
        .expect("rasterize_svg_to_rgba: failed to parse cursor SVG");
    let svg_size = tree.size();
    let max_dim = svg_size.width().max(svg_size.height());
    let scale = if max_dim > 0.0 {
        px_size as f32 / max_dim
    } else {
        1.0
    };
    let w = (svg_size.width() * scale).ceil().max(1.0) as u32;
    let h = (svg_size.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("rasterize_svg_to_rgba: pixmap");
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut rgba = pixmap.data().to_vec();
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as f32;
        if a > 0.0 && a < 255.0 {
            let inv = 255.0 / a;
            px[0] = (px[0] as f32 * inv).round().min(255.0) as u8;
            px[1] = (px[1] as f32 * inv).round().min(255.0) as u8;
            px[2] = (px[2] as f32 * inv).round().min(255.0) as u8;
        }
    }
    (rgba, w, h)
}

#[cfg(target_os = "macos")]
pub(crate) fn create_macos_cursor(
    rgba: &[u8],
    pixel_w: u32,
    pixel_h: u32,
    logical_w: f64,
    logical_h: f64,
    hotspot_x: f64,
    hotspot_y: f64,
) -> objc2::rc::Retained<objc2_app_kit::NSCursor> {
    use objc2::rc::Retained;
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageRep, NSCursor, NSImage, NSImageRep};
    use objc2_foundation::{NSPoint, NSSize};

    unsafe {
        let planes_ptr: *mut *mut u8 = std::ptr::null_mut();
        let rep: Retained<NSBitmapImageRep> = objc2::msg_send![
            NSBitmapImageRep::alloc(),
            initWithBitmapDataPlanes: planes_ptr,
            pixelsWide: pixel_w as isize,
            pixelsHigh: pixel_h as isize,
            bitsPerSample: 8_isize,
            samplesPerPixel: 4_isize,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: objc2_app_kit::NSDeviceRGBColorSpace,
            bytesPerRow: (pixel_w * 4) as isize,
            bitsPerPixel: 32_isize
        ];

        let bitmap_data: *mut u8 = objc2::msg_send![&rep, bitmapData];
        std::ptr::copy_nonoverlapping(rgba.as_ptr(), bitmap_data, rgba.len());

        let image = NSImage::new();
        let rep_as_imagerep: &NSImageRep =
            &*((&rep as &NSBitmapImageRep) as *const NSBitmapImageRep as *const NSImageRep);
        image.addRepresentation(rep_as_imagerep);
        image.setSize(NSSize::new(logical_w, logical_h));

        NSCursor::initWithImage_hotSpot(
            NSCursor::alloc(),
            &image,
            NSPoint::new(hotspot_x, hotspot_y),
        )
    }
}

pub(crate) struct ParticipantOverlay<'a> {
    pub(crate) participants: &'a ParticipantsManager,
    pub(crate) click_animation_renderer: Option<&'a ClickAnimationRenderer>,
}

impl<'a, Message> canvas::Program<Message> for ParticipantOverlay<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let translate = |pos: Position| -> Position {
            Position {
                x: pos.x * bounds.width as f64,
                y: pos.y * bounds.height as f64,
            }
        };
        let mut geometries = self.participants.draw(renderer, bounds, &translate);
        if let Some(click_renderer) = self.click_animation_renderer {
            geometries.push(click_renderer.draw(renderer, bounds, &translate));
        }
        geometries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room_service::{DrawSettings, DrawingMode};

    #[test]
    fn keys_mean_what_they_should_to_a_stamp() {
        let none = ModifiersState::empty();
        let letter = Key::Character("a".into());
        assert_eq!(
            stamp_key(&letter, Some("a"), none),
            Some(StampInput::Typed("a".into()))
        );
        let space = Key::Named(NamedKey::Space);
        assert_eq!(
            stamp_key(&space, Some(" "), none),
            Some(StampInput::Typed(" ".into()))
        );
        for (key, input) in [
            (NamedKey::Enter, StampInput::Finish),
            (NamedKey::Escape, StampInput::Cancel),
            (NamedKey::Backspace, StampInput::Backspace),
        ] {
            assert_eq!(stamp_key(&Key::Named(key), None, none), Some(input));
        }

        // Shortcuts aren't words, but Option is how a UK keyboard types a #.
        assert_eq!(stamp_key(&letter, Some("a"), ModifiersState::SUPER), None);
        assert_eq!(stamp_key(&letter, Some("a"), ModifiersState::CONTROL), None);
        assert_eq!(
            stamp_key(&Key::Character("#".into()), Some("#"), ModifiersState::ALT),
            Some(StampInput::Typed("#".into()))
        );

        // Nothing to type: Tab, arrows and function keys, and keys with no text at all.
        assert_eq!(
            stamp_key(&Key::Named(NamedKey::Tab), Some("\t"), none),
            None
        );
        let arrow = Key::Named(NamedKey::ArrowUp);
        assert_eq!(stamp_key(&arrow, Some("\u{F700}"), none), None);
        assert_eq!(stamp_key(&Key::Named(NamedKey::Shift), None, none), None);
    }

    fn position(x: f64, y: f64) -> Position {
        Position { x, y }
    }

    fn participants() -> ParticipantsManager {
        let mut participants = ParticipantsManager::new();
        participants
            .add_participant(
                LOCAL_PARTICIPANT_IDENTITY.to_string(),
                LOCAL_PARTICIPANT_IDENTITY,
                true,
                DrawingMode::Draw(DrawSettings { permanent: true }),
            )
            .unwrap();
        participants
    }

    #[test]
    fn freehand_sends_every_point() {
        let mut participants = participants();
        let mut stroke = LocalStroke::default();

        assert_eq!(
            stroke.begin(&mut participants, position(0.1, 0.2)),
            StrokeEvent::Start {
                x: 0.1,
                y: 0.2,
                path_id: 1
            }
        );
        assert_eq!(
            stroke.extend(&mut participants, position(0.3, 0.4), false),
            vec![StrokeEvent::AddPoint { x: 0.3, y: 0.4 }]
        );
        assert_eq!(
            stroke.end(&mut participants, position(0.5, 0.6), false),
            vec![StrokeEvent::End { x: 0.5, y: 0.6 }]
        );
        assert!(!stroke.is_active());
    }

    #[test]
    fn each_stroke_gets_its_own_path_id() {
        let mut participants = participants();
        let mut stroke = LocalStroke::default();
        stroke.begin(&mut participants, position(0.1, 0.1));
        stroke.end(&mut participants, position(0.1, 0.1), false);

        assert_eq!(
            stroke.begin(&mut participants, position(0.2, 0.2)),
            StrokeEvent::Start {
                x: 0.2,
                y: 0.2,
                path_id: 2
            }
        );
    }

    #[test]
    fn a_straight_line_is_replaced_as_it_moves_without_flooding_the_room() {
        let mut participants = participants();
        let mut stroke = LocalStroke::default();
        stroke.begin(&mut participants, position(0.1, 0.1));

        assert_eq!(
            stroke.extend(&mut participants, position(0.2, 0.2), true),
            vec![
                StrokeEvent::ClearPaths(vec![1]),
                StrokeEvent::Start {
                    x: 0.1,
                    y: 0.1,
                    path_id: 1
                },
                StrokeEvent::AddPoint { x: 0.2, y: 0.2 },
            ]
        );
        assert!(stroke
            .extend(&mut participants, position(0.3, 0.3), true)
            .is_empty());
    }

    #[test]
    fn a_straight_line_is_sent_exactly_when_it_ends() {
        let mut participants = participants();
        let mut stroke = LocalStroke::default();
        stroke.begin(&mut participants, position(0.1, 0.1));
        stroke.extend(&mut participants, position(0.2, 0.2), true);

        assert_eq!(
            stroke.end(&mut participants, position(0.9, 0.8), true),
            vec![
                StrokeEvent::ClearPaths(vec![1]),
                StrokeEvent::Start {
                    x: 0.1,
                    y: 0.1,
                    path_id: 1
                },
                StrokeEvent::End { x: 0.9, y: 0.8 },
            ]
        );
    }

    #[test]
    fn letting_go_of_shift_goes_back_to_freehand() {
        let mut participants = participants();
        let mut stroke = LocalStroke::default();
        stroke.begin(&mut participants, position(0.1, 0.1));
        stroke.extend(&mut participants, position(0.2, 0.2), true);

        assert_eq!(
            stroke.extend(&mut participants, position(0.3, 0.3), false),
            vec![StrokeEvent::AddPoint { x: 0.3, y: 0.3 }]
        );
    }

    #[test]
    fn letting_go_of_shift_sends_the_line_the_throttle_held_back() {
        let held_back_line = vec![
            StrokeEvent::ClearPaths(vec![1]),
            StrokeEvent::Start {
                x: 0.1,
                y: 0.1,
                path_id: 1,
            },
            StrokeEvent::AddPoint { x: 0.3, y: 0.3 },
        ];

        let mut participants = participants();
        let mut stroke = LocalStroke::default();
        stroke.begin(&mut participants, position(0.1, 0.1));
        stroke.extend(&mut participants, position(0.2, 0.2), true);
        stroke.extend(&mut participants, position(0.3, 0.3), true);

        let mut expected = held_back_line.clone();
        expected.push(StrokeEvent::AddPoint { x: 0.4, y: 0.4 });
        assert_eq!(
            stroke.extend(&mut participants, position(0.4, 0.4), false),
            expected
        );
        assert_eq!(
            stroke.extend(&mut participants, position(0.5, 0.5), false),
            vec![StrokeEvent::AddPoint { x: 0.5, y: 0.5 }]
        );

        let mut stroke = LocalStroke::default();
        stroke.begin(&mut participants, position(0.1, 0.1));
        stroke.extend(&mut participants, position(0.2, 0.2), true);
        stroke.extend(&mut participants, position(0.3, 0.3), true);

        let mut expected = held_back_line;
        expected.push(StrokeEvent::End { x: 0.4, y: 0.4 });
        assert_eq!(
            stroke.end(&mut participants, position(0.4, 0.4), false),
            expected
        );
    }

    #[test]
    fn a_cancelled_stroke_stops_following_the_pointer() {
        let mut participants = participants();
        let mut stroke = LocalStroke::default();
        stroke.begin(&mut participants, position(0.1, 0.1));
        stroke.cancel();

        assert!(stroke
            .extend(&mut participants, position(0.2, 0.2), false)
            .is_empty());
        assert!(stroke
            .end(&mut participants, position(0.2, 0.2), false)
            .is_empty());
    }

    #[test]
    fn undo_clears_the_last_stroke_for_everyone() {
        let mut participants = participants();
        let mut stroke = LocalStroke::default();
        assert_eq!(stroke.undo(&mut participants), None);

        stroke.begin(&mut participants, position(0.1, 0.1));
        assert_eq!(stroke.undo(&mut participants), None);

        stroke.end(&mut participants, position(0.2, 0.2), false);
        assert_eq!(
            stroke.undo(&mut participants),
            Some(StrokeEvent::ClearPaths(vec![1]))
        );
        assert_eq!(stroke.undo(&mut participants), None);
    }

    #[test]
    fn the_hint_only_shows_once_and_goes_away_by_itself() {
        let mut hint = DrawingHint::default();
        assert!(!hint.is_visible());

        hint.show_once();
        assert!(hint.is_visible());
        assert_eq!(hint.opacity(), 1.0);

        let long_ago = Instant::now() - DRAWING_HINT_DURATION - DRAWING_HINT_FADE_DURATION;
        hint.shown_at = Some(long_ago);
        assert!(!hint.is_visible());

        hint.show_once();
        assert!(!hint.is_visible());
    }

    #[test]
    fn undo_shortcut_is_command_z_but_not_redo() {
        let command = if cfg!(target_os = "macos") {
            ModifiersState::SUPER
        } else {
            ModifiersState::CONTROL
        };
        let z = Key::Character("z".into());
        assert!(is_undo_shortcut(&z, command));
        assert!(is_undo_shortcut(&Key::Character("Z".into()), command));
        assert!(!is_undo_shortcut(&z, ModifiersState::empty()));
        assert!(!is_undo_shortcut(&z, command | ModifiersState::SHIFT));
        assert!(!is_undo_shortcut(&Key::Character("x".into()), command));
    }
}
