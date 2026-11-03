use std::time::{Duration, Instant};

use iced::widget::canvas::{path, stroke, Cache, Frame, Geometry, Stroke};
use iced::{Color, Point, Rectangle, Renderer};

use crate::{room_service::DrawingMode, utils::geometry::Position};

/// How long ink stays after the last stroke before it all fades together.
const PATH_EXPIRY_DURATION: Duration = Duration::from_secs(3);
const FADE_OUT_DURATION: Duration = Duration::from_millis(450);

const INK_WIDTH: f32 = 3.5;
const HALO_WIDTH: f32 = INK_WIDTH + 1.5;
const HALO_OPACITY: f32 = 0.28;
/// Ink brighter than this gets a dark halo, anything darker gets a light one.
const HALO_LUMINANCE_THRESHOLD: f32 = 0.45;

/// Points closer together than this are hand tremor, not part of the shape.
const MIN_POINT_SPACING: f32 = 3.0;
/// Turns tighter than about 100 degrees stay corners instead of being rounded off.
const SHARP_TURN_COSINE: f32 = -0.17;

fn color_from_hex(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');

    // Check if the hex string has at least 6 characters to avoid panic
    if hex.len() < 6 {
        log::warn!(
            "color_from_hex: invalid hex color '{}', using default black color",
            hex
        );
        return Color::from_rgb8(0, 0, 0);
    }

    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    Color::from_rgb8(r, g, b)
}

/// Keeps ink readable on any background: whichever one it blends into, the halo stands out.
fn halo_color(ink: Color) -> Color {
    let luminance = 0.2126 * ink.r + 0.7152 * ink.g + 0.0722 * ink.b;
    if luminance > HALO_LUMINANCE_THRESHOLD {
        Color::from_rgba(0.0, 0.0, 0.0, HALO_OPACITY)
    } else {
        Color::from_rgba(1.0, 1.0, 1.0, HALO_OPACITY)
    }
}

fn fade_opacity(elapsed: Duration) -> f32 {
    let progress = (elapsed.as_secs_f32() / FADE_OUT_DURATION.as_secs_f32()).clamp(0.0, 1.0);
    1.0 - progress * progress * (3.0 - 2.0 * progress)
}

fn midpoint(a: Point, b: Point) -> Point {
    Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
}

fn is_sharp_turn(previous: Point, corner: Point, next: Point) -> bool {
    let incoming = corner - previous;
    let outgoing = next - corner;
    let lengths = incoming.x.hypot(incoming.y) * outgoing.x.hypot(outgoing.y);
    lengths > 0.0
        && (incoming.x * outgoing.x + incoming.y * outgoing.y) / lengths < SHARP_TURN_COSINE
}

/// Translates a stroke to window coordinates and drops points too close together to matter.
/// The last point is always kept so the ink ends exactly where the pen is.
fn spaced_points(points: &[Position], translate: &dyn Fn(Position) -> Position) -> Vec<Point> {
    let mut spaced: Vec<Point> = Vec::with_capacity(points.len());
    let mut tip = None;
    for position in points {
        let translated = translate(*position);
        let point = Point::new(translated.x as f32, translated.y as f32);
        tip = Some(point);
        if spaced
            .last()
            .is_none_or(|last| last.distance(point) >= MIN_POINT_SPACING)
        {
            spaced.push(point);
        }
    }
    if let Some(tip) = tip {
        if spaced.len() > 1 {
            *spaced.last_mut().unwrap() = tip;
        } else if spaced.first() != Some(&tip) {
            spaced.push(tip);
        }
    }
    spaced
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum InkSegment {
    Line(Point),
    Curve { control: Point, to: Point },
}

/// Joins the midpoints of a stroke with curves that use its points as control points. Each curve
/// only depends on its neighbours, so the ink already drawn doesn't shift as new points arrive.
fn ink_segments(points: &[Point]) -> Vec<InkSegment> {
    if points.len() < 3 {
        return points
            .iter()
            .skip(1)
            .copied()
            .map(InkSegment::Line)
            .collect();
    }

    let mut segments = Vec::with_capacity(points.len() + 1);
    segments.push(InkSegment::Line(midpoint(points[0], points[1])));
    for neighbours in points.windows(3) {
        let (previous, corner, next) = (neighbours[0], neighbours[1], neighbours[2]);
        let to = midpoint(corner, next);
        if is_sharp_turn(previous, corner, next) {
            segments.push(InkSegment::Line(corner));
            segments.push(InkSegment::Line(to));
        } else {
            segments.push(InkSegment::Curve {
                control: corner,
                to,
            });
        }
    }
    segments.push(InkSegment::Line(points[points.len() - 1]));
    segments
}

#[derive(Debug, Clone)]
struct DrawPath {
    path_id: u64,
    points: Vec<Position>,
}

impl DrawPath {
    pub fn new(path_id: u64, point: Position) -> Self {
        Self {
            path_id,
            points: vec![point],
        }
    }
}

#[derive(Debug, Clone)]
struct FadingPath {
    path: DrawPath,
    started_at: Instant,
}

pub struct Draw {
    in_progress_path: Option<DrawPath>,
    completed_paths: Vec<DrawPath>,
    fading_paths: Vec<FadingPath>,
    last_drawing_change: Option<Instant>,
    completed_cache: Cache,
    mode: DrawingMode,
    color: Color,
    auto_clear: bool,
}

impl std::fmt::Debug for Draw {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Draw")
            .field("in_progress_path", &self.in_progress_path)
            .field("completed_paths", &self.completed_paths)
            .field("fading_paths", &self.fading_paths)
            .field("mode", &self.mode)
            .field("color", &self.color)
            .finish()
    }
}

impl Draw {
    pub fn new(color: &str, auto_clear: bool, initial_drawing_mode: DrawingMode) -> Self {
        Self {
            in_progress_path: None,
            completed_paths: Vec::new(),
            fading_paths: Vec::new(),
            last_drawing_change: None,
            completed_cache: Cache::new(),
            mode: initial_drawing_mode,
            color: color_from_hex(color),
            auto_clear,
        }
    }

    pub fn mode(&self) -> DrawingMode {
        self.mode.clone()
    }

    pub fn set_color(&mut self, color: &str) {
        self.color = color_from_hex(color);
        self.completed_cache.clear();
    }

    /// Where the stroke being drawn has got to.
    pub fn pen_tip(&self) -> Option<Position> {
        self.in_progress_path
            .as_ref()
            .and_then(|path| path.points.last().copied())
    }

    pub fn set_mode(&mut self, mode: DrawingMode) {
        self.mode = mode.clone();
        if mode == DrawingMode::Disabled {
            self.clear();
        }
    }

    /// Starts a new path, returning whether it replaces one of the same ID that was just cleared.
    /// That's how a straight line being dragged out gets redrawn, so its ghost mustn't linger.
    pub fn start_path(&mut self, path_id: u64, point: Position) -> bool {
        if self.mode == DrawingMode::Disabled {
            log::warn!("start_path: drawing mode is disabled, skipping path");
            return false;
        }

        log::debug!("start_path: starting new path with id {}", path_id);
        let fading_count = self.fading_paths.len();
        self.fading_paths
            .retain(|fading| fading.path.path_id != path_id);
        self.in_progress_path = Some(DrawPath::new(path_id, point));
        self.fading_paths.len() != fading_count
    }

    pub fn add_point(&mut self, point: Position) {
        if self.mode == DrawingMode::Disabled {
            log::warn!("add_point: drawing mode is disabled, skipping point");
            return;
        }

        if let Some(in_progress_path) = self.in_progress_path.as_mut() {
            in_progress_path.points.push(point);
        } else {
            log::warn!("add_point: no current path in progress, skipping point");
        }
    }

    /// Reduces the path in progress to a straight line from where it started to `end`.
    pub fn straighten_in_progress(&mut self, end: Position) {
        if let Some(in_progress_path) = self.in_progress_path.as_mut() {
            in_progress_path.points.truncate(1);
            in_progress_path.points.push(end);
        }
    }

    pub fn finish_path(&mut self) {
        if self.mode == DrawingMode::Disabled {
            log::warn!("finish_path: drawing mode is disabled, skipping path");
            return;
        }

        if let Some(in_progress_path) = self.in_progress_path.take() {
            log::debug!("finish_path: finishing path {}", in_progress_path.path_id);
            self.last_drawing_change = Some(Instant::now());
            self.completed_paths.push(in_progress_path);
            self.completed_cache.clear();
        } else {
            log::warn!("finish_path: no path in progress");
        }
    }

    fn fade_out(&mut self, path: DrawPath) {
        self.fading_paths.push(FadingPath {
            path,
            started_at: Instant::now(),
        });
    }

    pub fn clear_path(&mut self, path_id: u64) {
        log::debug!("clear_path: clearing path {}", path_id);

        // Clear current path if it matches
        if self
            .in_progress_path
            .as_ref()
            .is_some_and(|in_progress| in_progress.path_id == path_id)
        {
            if let Some(in_progress) = self.in_progress_path.take() {
                self.fade_out(in_progress);
            }
        }

        // Remove from completed paths
        if let Some(index) = self
            .completed_paths
            .iter()
            .position(|path| path.path_id == path_id)
        {
            let path = self.completed_paths.remove(index);
            self.fade_out(path);
            self.completed_cache.clear();
        }
    }

    pub fn clear(&mut self) {
        if let Some(in_progress) = self.in_progress_path.take() {
            self.fade_out(in_progress);
        }
        for path in std::mem::take(&mut self.completed_paths) {
            self.fade_out(path);
        }
        self.completed_cache.clear();
    }

    /// Takes back the most recently finished path, returning its ID.
    pub fn undo_last_path(&mut self) -> Option<u64> {
        let path = self.completed_paths.pop()?;
        let path_id = path.path_id;
        self.fade_out(path);
        self.last_drawing_change = Some(Instant::now());
        self.completed_cache.clear();
        Some(path_id)
    }

    pub fn clear_cache(&mut self) {
        self.completed_cache.clear();
    }

    /// Whether something is moving on its own and needs frames even with no input.
    pub fn is_animating(&self) -> bool {
        !self.fading_paths.is_empty()
    }

    /// Fades out paths that have been left alone long enough and returns their IDs, so others
    /// can do the same. Paths clear as a group, a drawing shouldn't vanish one stroke at a time.
    pub fn clear_expired_paths(&mut self) -> Vec<u64> {
        let now = Instant::now();
        self.fading_paths
            .retain(|fading| now.duration_since(fading.started_at) < FADE_OUT_DURATION);

        if !self.auto_clear || self.in_progress_path.is_some() {
            return Vec::new();
        }

        // Only clear in non-permanent mode
        if let DrawingMode::Draw(settings) = &self.mode {
            if settings.permanent {
                return Vec::new();
            }
        } else {
            return Vec::new();
        }

        let Some(last_drawing_change) = self.last_drawing_change else {
            return Vec::new();
        };
        if self.completed_paths.is_empty()
            || now.duration_since(last_drawing_change) < PATH_EXPIRY_DURATION
        {
            return Vec::new();
        }

        let removed_ids = self
            .completed_paths
            .iter()
            .map(|path| path.path_id)
            .collect();
        self.clear();
        removed_ids
    }

    /// Returns cached geometry for completed paths.
    pub fn draw_completed(
        &self,
        renderer: &Renderer,
        bounds: Rectangle,
        translate: &dyn Fn(Position) -> Position,
    ) -> Geometry {
        self.completed_cache.draw(renderer, bounds.size(), |frame| {
            let paths: Vec<path::Path> = self
                .completed_paths
                .iter()
                .filter_map(|draw_path| Self::build_path(&draw_path.points, translate))
                .collect();
            // All the halos go under all the ink, so a stroke doesn't smudge the ones it crosses.
            for path in &paths {
                frame.stroke(path, Self::make_stroke(halo_color(self.color), HALO_WIDTH));
            }
            for path in &paths {
                frame.stroke(path, Self::make_stroke(self.color, INK_WIDTH));
            }
        })
    }

    /// Draws everything that changes from frame to frame: paths fading out and the path in
    /// progress.
    pub fn draw_in_progress_to_frame(
        &self,
        frame: &mut Frame,
        translate: &dyn Fn(Position) -> Position,
    ) {
        let now = Instant::now();
        for fading in &self.fading_paths {
            let opacity = fade_opacity(now.duration_since(fading.started_at));
            self.draw_ink(frame, &fading.path.points, opacity, translate);
        }
        if let Some(in_progress) = &self.in_progress_path {
            self.draw_ink(frame, &in_progress.points, 1.0, translate);
        }
    }

    fn draw_ink(
        &self,
        frame: &mut Frame,
        points: &[Position],
        opacity: f32,
        translate: &dyn Fn(Position) -> Position,
    ) {
        if let Some(path) = Self::build_path(points, translate) {
            let mut halo = halo_color(self.color);
            halo.a *= opacity;
            let mut ink = self.color;
            ink.a *= opacity;
            frame.stroke(&path, Self::make_stroke(halo, HALO_WIDTH));
            frame.stroke(&path, Self::make_stroke(ink, INK_WIDTH));
        }
    }

    fn make_stroke(color: Color, width: f32) -> Stroke<'static> {
        Stroke {
            style: stroke::Style::Solid(color),
            width,
            line_cap: stroke::LineCap::Round,
            line_join: stroke::LineJoin::Round,
            line_dash: stroke::LineDash::default(),
        }
    }

    fn build_path(
        points: &[Position],
        translate: &dyn Fn(Position) -> Position,
    ) -> Option<path::Path> {
        let points = spaced_points(points, translate);
        let first = *points.first()?;

        let mut builder = path::Builder::new();
        builder.move_to(first);
        if points.len() == 1 {
            // A click: the round caps of a tiny line make a dot.
            builder.line_to(Point::new(first.x + 0.01, first.y));
        }
        for segment in ink_segments(&points) {
            match segment {
                InkSegment::Line(to) => builder.line_to(to),
                InkSegment::Curve { control, to } => builder.quadratic_curve_to(control, to),
            }
        }
        Some(builder.build())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room_service::DrawSettings;

    fn position(x: f64, y: f64) -> Position {
        Position { x, y }
    }

    fn fading_mode(permanent: bool) -> DrawingMode {
        DrawingMode::Draw(DrawSettings { permanent })
    }

    fn draw_with_finished_paths(auto_clear: bool, permanent: bool, path_ids: &[u64]) -> Draw {
        let mut draw = Draw::new("#FF0000", auto_clear, fading_mode(permanent));
        for path_id in path_ids {
            draw.start_path(*path_id, position(0.1, 0.1));
            draw.add_point(position(0.2, 0.2));
            draw.finish_path();
        }
        draw
    }

    fn age_paths(draw: &mut Draw, age: Duration) {
        draw.last_drawing_change = Some(Instant::now() - age);
    }

    fn age_fades(draw: &mut Draw, age: Duration) {
        let started_at = Instant::now() - age;
        for fading in &mut draw.fading_paths {
            fading.started_at = started_at;
        }
    }

    fn identity(position: Position) -> Position {
        position
    }

    #[test]
    fn a_click_keeps_a_single_point_to_draw_as_a_dot() {
        let click = [position(10.0, 10.0), position(10.0, 10.0)];
        assert_eq!(spaced_points(&click, &identity).len(), 1);
        assert!(Draw::build_path(&click, &identity).is_some());
    }

    #[test]
    fn tremor_is_dropped_but_the_ink_still_ends_at_the_pen() {
        let points = [
            position(0.0, 0.0),
            position(0.4, 0.0),
            position(0.8, 0.0),
            position(5.0, 0.0),
            position(5.3, 0.0),
        ];
        assert_eq!(
            spaced_points(&points, &identity),
            vec![Point::new(0.0, 0.0), Point::new(5.3, 0.0)]
        );
    }

    #[test]
    fn strokes_are_curved_through_their_midpoints() {
        let points = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(20.0, 10.0),
        ];
        assert_eq!(
            ink_segments(&points),
            vec![
                InkSegment::Line(Point::new(5.0, 0.0)),
                InkSegment::Curve {
                    control: Point::new(10.0, 0.0),
                    to: Point::new(15.0, 5.0),
                },
                InkSegment::Line(Point::new(20.0, 10.0)),
            ]
        );
    }

    #[test]
    fn sharp_turns_stay_corners() {
        let points = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(1.0, 2.0),
        ];
        assert!(ink_segments(&points)
            .iter()
            .all(|segment| matches!(segment, InkSegment::Line(_))));
        assert!(ink_segments(&points).contains(&InkSegment::Line(Point::new(10.0, 0.0))));
    }

    #[test]
    fn a_straight_line_stays_straight() {
        let points = [Point::new(0.0, 0.0), Point::new(100.0, 40.0)];
        assert_eq!(
            ink_segments(&points),
            vec![InkSegment::Line(Point::new(100.0, 40.0))]
        );
    }

    #[test]
    fn ink_drawn_earlier_does_not_move_as_the_stroke_grows() {
        let mut points: Vec<Point> = (0..20)
            .map(|index| Point::new(index as f32 * 5.0, (index as f32 * 0.7).sin() * 20.0))
            .collect();
        let before = ink_segments(&points);
        points.push(Point::new(120.0, 30.0));
        let after = ink_segments(&points);
        assert_eq!(before[..before.len() - 1], after[..before.len() - 1]);
    }

    #[test]
    fn nothing_expires_before_the_pause_is_over() {
        let mut draw = draw_with_finished_paths(true, false, &[1, 2]);
        age_paths(&mut draw, PATH_EXPIRY_DURATION / 2);
        assert!(draw.clear_expired_paths().is_empty());
        assert_eq!(draw.completed_paths.len(), 2);
    }

    #[test]
    fn strokes_expire_together_and_are_reported_together() {
        let mut draw = draw_with_finished_paths(true, false, &[1, 2, 3]);
        age_paths(&mut draw, PATH_EXPIRY_DURATION);
        assert_eq!(draw.clear_expired_paths(), vec![1, 2, 3]);
        assert!(draw.completed_paths.is_empty());
        assert_eq!(draw.fading_paths.len(), 3);
        assert!(draw.is_animating());
    }

    #[test]
    fn undoing_a_stroke_keeps_the_rest_on_screen_for_another_pause() {
        let mut draw = draw_with_finished_paths(true, false, &[1, 2]);
        age_paths(&mut draw, PATH_EXPIRY_DURATION + Duration::from_millis(200));

        assert_eq!(draw.undo_last_path(), Some(2));
        assert!(draw.clear_expired_paths().is_empty());
        assert_eq!(draw.completed_paths.len(), 1);

        age_paths(&mut draw, PATH_EXPIRY_DURATION);
        assert_eq!(draw.clear_expired_paths(), vec![1]);
    }

    #[test]
    fn a_new_stroke_holds_older_ones_on_screen() {
        let mut draw = draw_with_finished_paths(true, false, &[1]);
        age_paths(&mut draw, PATH_EXPIRY_DURATION * 2);

        draw.start_path(2, position(0.5, 0.5));
        assert!(draw.clear_expired_paths().is_empty());

        draw.finish_path();
        assert!(draw.clear_expired_paths().is_empty());
        assert_eq!(draw.completed_paths.len(), 2);
    }

    #[test]
    fn permanent_ink_never_expires() {
        let mut draw = draw_with_finished_paths(true, true, &[1]);
        age_paths(&mut draw, PATH_EXPIRY_DURATION * 10);
        assert!(draw.clear_expired_paths().is_empty());
        assert_eq!(draw.completed_paths.len(), 1);
    }

    #[test]
    fn someone_elses_ink_only_expires_when_they_say_so() {
        let mut draw = draw_with_finished_paths(false, false, &[1]);
        age_paths(&mut draw, PATH_EXPIRY_DURATION * 10);
        assert!(draw.clear_expired_paths().is_empty());
        assert_eq!(draw.completed_paths.len(), 1);

        draw.clear_path(1);
        assert!(draw.completed_paths.is_empty());
        assert!(draw.is_animating());
    }

    #[test]
    fn fades_finish_and_stop_animating_for_everyone() {
        let mut draw = draw_with_finished_paths(false, false, &[1]);
        draw.clear_path(1);

        assert!(draw.clear_expired_paths().is_empty());
        assert!(draw.is_animating());

        age_fades(&mut draw, FADE_OUT_DURATION);
        draw.clear_expired_paths();
        assert!(!draw.is_animating());
    }

    #[test]
    fn fade_runs_from_opaque_to_invisible() {
        assert_eq!(fade_opacity(Duration::ZERO), 1.0);
        assert_eq!(fade_opacity(FADE_OUT_DURATION), 0.0);
        let halfway = fade_opacity(FADE_OUT_DURATION / 2);
        assert!(halfway > 0.0 && halfway < 1.0);
    }

    #[test]
    fn clearing_fades_the_stroke_in_progress_too() {
        let mut draw = draw_with_finished_paths(false, false, &[1]);
        draw.start_path(2, position(0.3, 0.3));
        draw.clear();
        assert!(draw.in_progress_path.is_none());
        assert!(draw.completed_paths.is_empty());
        assert_eq!(draw.fading_paths.len(), 2);
    }

    #[test]
    fn the_pen_tip_is_the_end_of_the_stroke_being_drawn() {
        let mut draw = Draw::new("#FF0000", false, fading_mode(false));
        assert!(draw.pen_tip().is_none());
        draw.start_path(1, position(0.1, 0.1));
        draw.add_point(position(0.2, 0.3));
        assert_eq!(draw.pen_tip().map(|tip| (tip.x, tip.y)), Some((0.2, 0.3)));
        draw.finish_path();
        assert!(draw.pen_tip().is_none());
    }

    #[test]
    fn a_new_colour_applies_to_ink_already_drawn() {
        let mut draw = draw_with_finished_paths(false, false, &[1]);
        draw.set_color("#0a84ff");
        assert_eq!(draw.color, color_from_hex("#0a84ff"));
        assert_eq!(draw.completed_paths.len(), 1);
    }

    #[test]
    fn restarting_a_cleared_path_replaces_its_ghost() {
        let mut draw = Draw::new("#FF0000", false, fading_mode(false));
        assert!(!draw.start_path(1, position(0.1, 0.1)));
        draw.add_point(position(0.2, 0.2));

        draw.clear_path(1);
        assert_eq!(draw.fading_paths.len(), 1);

        assert!(draw.start_path(1, position(0.1, 0.1)));
        assert!(draw.fading_paths.is_empty());
        assert!(draw.in_progress_path.is_some());
    }

    #[test]
    fn straightening_keeps_only_the_start_and_end() {
        let mut draw = Draw::new("#FF0000", false, fading_mode(false));
        draw.start_path(1, position(0.1, 0.1));
        draw.add_point(position(0.2, 0.5));
        draw.add_point(position(0.3, 0.1));

        draw.straighten_in_progress(position(0.9, 0.9));

        let points = &draw.in_progress_path.as_ref().unwrap().points;
        assert_eq!(points.len(), 2);
        assert_eq!((points[0].x, points[0].y), (0.1, 0.1));
        assert_eq!((points[1].x, points[1].y), (0.9, 0.9));
    }

    #[test]
    fn undo_takes_back_the_last_stroke_only() {
        let mut draw = draw_with_finished_paths(false, false, &[1, 2]);
        assert_eq!(draw.undo_last_path(), Some(2));
        assert_eq!(draw.completed_paths.len(), 1);
        assert_eq!(draw.completed_paths[0].path_id, 1);
        assert_eq!(draw.undo_last_path(), Some(1));
        assert_eq!(draw.undo_last_path(), None);
    }

    #[test]
    fn halos_contrast_with_the_ink() {
        assert_eq!(halo_color(color_from_hex("#FFB900")).r, 0.0);
        assert_eq!(halo_color(color_from_hex("#B80088")).r, 1.0);
    }
}
