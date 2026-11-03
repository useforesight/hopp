//! Captured calls: one call at a time as a timeline of what was said, drawn and shown,
//! and every past call down the side, searchable.
//!
//! Reads the capture database and frame records directly (the recorder writes them),
//! polling while a capture is live. The timeline itself comes from
//! [`crate::capture::timeline`]: speech as messages (one voice on two mics shown once),
//! drawings zoomed to where they were drawn, screen changes as filmstrips. A scrubber
//! across the top shows who spoke when and jumps there; any picture opens full size.
//! Narrow, the call list hides behind a button, which suits a tile beside the share.
use std::{
    cell::Cell,
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_kit::{
    AnyElement, App, AppContext, Bounds, ClipboardItem, Context, Entity, FontWeight,
    HighlightStyle, Hsla, ImageCache, ImageCacheError, ImageCacheItem, InteractiveElement,
    IntoElement, ListAlignment, ListOffset, ListState, ObjectFit, ParentElement, Pixels, Render,
    RenderImage, Resource, SharedString, StatefulInteractiveElement, Styled, StyledImage,
    StyledText, Window, WindowHandle,
    base::input::{InputEvent, InputState},
    canvas, div, image_cache, img, list,
    prelude::FluentBuilder,
    px, relative, rgb, size,
};
use gpui_omarchy::{ActiveTheme, IconName, icon, input};

use super::{
    palette::Speakers,
    session::Session,
    style::{self, Tone},
};
use crate::capture::{
    frame_files::{self as moments, FrameInfo, MarkInfo},
    store::{self, Call, Kind, Line, Store},
    timeline::{self, Burst, Drawing, Filter, Overview, Row},
};

/// Below this width the call list hides and the timeline fills the window.
const NARROW: f32 = 760.;
/// The timeline's reading width.
const COLUMN: f32 = 860.;
const POLL: Duration = Duration::from_millis(600);
/// Decoded frames kept at once (each up to ~20 MB full size, ~3.5 MB for cards).
const IMAGES: usize = 36;

/// One picture the viewer can step through.
#[derive(Clone)]
struct Picture {
    frame: u32,
    title: String,
    detail: String,
    marks: Vec<MarkInfo>,
}

pub struct CapturesView {
    session: Entity<Session>,
    store: Result<Store, String>,
    search: Entity<InputState>,
    find: Entity<InputState>,
    calls: Vec<Call>,
    /// Matching text per call while searching every call.
    snippets: Vec<(String, String)>,
    selected: Option<Call>,
    lines: Vec<Line>,
    cursor: i64,
    frames: Vec<FrameInfo>,
    /// Frame lines seen, to know when the frame records need reading again.
    frame_lines: usize,
    rows: Vec<Row>,
    /// Indexes into `rows` that pass the filter and search, in order.
    shown: Vec<usize>,
    overview: Overview,
    gallery: Vec<Picture>,
    colours: HashMap<String, Hsla>,
    feed: ListState,
    filter: Filter,
    /// Drawings shown whole rather than zoomed, by frame number.
    whole: HashSet<u32>,
    /// Marks outlined while their chip is hovered: (frame, mark indexes).
    lit: Option<(u32, Vec<usize>)>,
    /// The picture open full size, as an index into `gallery`.
    viewing: Option<usize>,
    images: Entity<FrameImages>,
    scrubber: Rc<Cell<Bounds<Pixels>>>,
    /// Narrow window: show the list instead of the timeline.
    show_list: bool,
    confirm_delete: bool,
    copied: Option<Instant>,
    last_list: Instant,
    /// The capture id the session last reported, to notice starts and stops.
    live_id: Option<String>,
}

impl CapturesView {
    fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search every call"));
        cx.subscribe(&search, |view, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                view.reload_list(cx);
                cx.notify();
            }
        })
        .detach();
        let find = cx.new(|cx| InputState::new(window, cx).placeholder("Search this call"));
        cx.subscribe(&find, |view, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                view.refilter(cx);
                cx.notify();
            }
        })
        .detach();
        cx.observe(&session, |view, session, cx| {
            let live = session
                .read(cx)
                .capture
                .as_ref()
                .map(|capture| capture.id.clone());
            if live != view.live_id {
                view.live_id = live.clone();
                view.reload_list(cx);
                // Follow a capture that just started, unless you're reading another call.
                if let Some(id) = live
                    && view.selected.as_ref().is_none_or(|call| !call.live())
                {
                    view.select(&id, cx);
                }
            }
            cx.notify();
        })
        .detach();
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if view.update(cx, |view, cx| view.poll(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let store = Store::open_default().map_err(|error| format!("{error:#}"));
        let live_id = session
            .read(cx)
            .capture
            .as_ref()
            .map(|capture| capture.id.clone());
        let images = FrameImages::new(cx);
        let mut view = Self {
            session,
            store,
            search,
            find,
            calls: Vec::new(),
            snippets: Vec::new(),
            selected: None,
            lines: Vec::new(),
            cursor: 0,
            frames: Vec::new(),
            frame_lines: 0,
            rows: Vec::new(),
            shown: Vec::new(),
            overview: Overview::default(),
            gallery: Vec::new(),
            colours: HashMap::new(),
            feed: ListState::new(0, ListAlignment::Top, px(800.)),
            filter: Filter::Everything,
            whole: HashSet::new(),
            lit: None,
            viewing: None,
            images,
            scrubber: Rc::new(Cell::new(Bounds::default())),
            show_list: false,
            confirm_delete: false,
            copied: None,
            last_list: Instant::now(),
            live_id,
        };
        view.reload_list(cx);
        let first = view
            .live_id
            .clone()
            .or_else(|| view.calls.first().map(|call| call.id.clone()));
        if let Some(id) = first {
            view.select(&id, cx);
        }
        view
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_owned()
    }

    fn reload_list(&mut self, cx: &mut Context<Self>) {
        self.last_list = Instant::now();
        let query = self.query(cx);
        let Ok(store) = &self.store else {
            return;
        };
        if query.is_empty() {
            self.calls = store.calls(200).unwrap_or_default();
            self.snippets.clear();
            return;
        }
        self.calls.clear();
        self.snippets.clear();
        for hit in store.search(&query, 200).unwrap_or_default() {
            if self.snippets.iter().any(|(id, _)| *id == hit.call_id) {
                continue;
            }
            if let Ok(Some(call)) = store.call(&hit.call_id) {
                let who = hit.line.speaker.clone().unwrap_or_default();
                let text = if who.is_empty() {
                    hit.snippet
                } else {
                    format!("{who}: {}", hit.snippet)
                };
                self.snippets.push((call.id.clone(), text));
                self.calls.push(call);
            }
        }
    }

    /// Show the call `which` names: an id, `last` or `live` (see [`Store::call`]).
    pub fn open_call(&mut self, which: &str, cx: &mut Context<Self>) {
        let found = self
            .store
            .as_ref()
            .ok()
            .and_then(|store| store.call(which).ok().flatten());
        if let Some(call) = found {
            self.select(&call.id, cx);
        }
    }

    fn select(&mut self, id: &str, cx: &mut Context<Self>) {
        let Ok(store) = &self.store else {
            return;
        };
        self.confirm_delete = false;
        self.selected = store.call(id).ok().flatten();
        self.lines.clear();
        self.cursor = 0;
        merge(
            &mut self.lines,
            &mut self.cursor,
            store.changes(id, 0).unwrap_or_default(),
        );
        self.frames = moments::list(id).unwrap_or_default();
        self.frame_lines = frame_lines(&self.lines);
        self.whole.clear();
        self.lit = None;
        self.viewing = None;
        self.show_list = false;
        self.rebuild(cx);
        let live = self.selected.as_ref().is_some_and(Call::live);
        self.feed.set_follow_mode(if live {
            gpui_kit::FollowMode::Tail
        } else {
            gpui_kit::FollowMode::Normal
        });
        if live {
            self.feed.scroll_to_end();
        } else {
            self.feed.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
        }
        cx.notify();
    }

    /// Rows, the scrubber, the viewer's pictures and everyone's colours, from the lines
    /// and frames.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let Some(call) = self.selected.clone() else {
            self.rows.clear();
            self.refilter(cx);
            return;
        };
        self.rows = timeline::build(&self.lines, &self.frames);
        let duration = call
            .ended_ms
            .map(|ended| ended - call.started_ms)
            .unwrap_or_else(|| store::now_ms() - call.started_ms);
        self.overview = timeline::overview(&self.rows, &self.lines, duration);
        self.colours = colours(&call, &self.lines, &self.frames, cx);
        self.gallery = gallery(&self.rows, &self.frames);
        self.refilter(cx);
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let find = self.find.read(cx).value().trim().to_lowercase();
        self.shown = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| self.filter == Filter::Everything || row.filter() == self.filter)
            .filter(|(_, row)| {
                find.is_empty()
                    || matches!(row, Row::Said { text, .. } if text.to_lowercase().contains(&find))
            })
            .map(|(index, _)| index)
            .collect();
        self.feed.reset(self.shown.len());
    }

    /// New lines for a live call, and a fresh list now and then.
    fn poll(&mut self, cx: &mut Context<Self>) {
        let Ok(store) = &self.store else {
            return;
        };
        let mut changed = false;
        if let Some(call) = self.selected.clone()
            && call.live()
        {
            let new = store.changes(&call.id, self.cursor).unwrap_or_default();
            if !new.is_empty() {
                merge(&mut self.lines, &mut self.cursor, new);
                let seen = frame_lines(&self.lines);
                if seen != self.frame_lines {
                    self.frame_lines = seen;
                    self.frames = moments::list(&call.id).unwrap_or_default();
                }
                changed = true;
            }
            if let Ok(Some(fresh)) = store.call(&call.id) {
                changed |= fresh != call;
                self.selected = Some(fresh);
            }
            if changed {
                self.rebuild(cx);
            }
            // The clock in the header moves while live.
            changed = true;
        }
        if self.last_list.elapsed() > Duration::from_secs(5) {
            self.reload_list(cx);
            changed = true;
        }
        if self
            .copied
            .is_some_and(|at| at.elapsed() > Duration::from_secs(2))
        {
            self.copied = None;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    fn agent_prompt(call: &Call) -> String {
        if call.live() {
            format!(
                "I'm in a Hopp call that is being transcribed on this computer (capture {id}). \
                 Read what's been said with `hopp capture show {id}`. To keep listening, run \
                 `hopp capture follow {id} --on-wake \"hey claude\"`: it returns when someone \
                 says \"hey claude\", with the lines leading up to it. Answer, then run it again.",
                id = call.id
            )
        } else {
            format!(
                "Read the transcript of a Hopp call with `hopp capture show {id}`. Write a short \
                 title and a two- or three-line summary with any decisions and follow-ups, then \
                 save them with `hopp capture annotate {id} --title \"…\" --summary \"…\"`.",
                id = call.id
            )
        }
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let (Ok(store), Some(call)) = (&self.store, self.selected.clone()) else {
            return;
        };
        if call.live() {
            return;
        }
        if !self.confirm_delete {
            self.confirm_delete = true;
            cx.notify();
            return;
        }
        if store.delete(&call.id).is_ok() {
            if let Ok(path) = crate::capture::markdown_path(&call.id) {
                let _ = std::fs::remove_file(path);
            }
            moments::remove(&call.id);
        }
        self.selected = None;
        self.lines.clear();
        self.frames.clear();
        self.rebuild(cx);
        self.reload_list(cx);
        if let Some(first) = self.calls.first().map(|call| call.id.clone()) {
            self.select(&first, cx);
        }
        cx.notify();
    }

    /// Scroll the timeline to the first row at or after `t_ms`.
    fn jump(&mut self, t_ms: i64, cx: &mut Context<Self>) {
        let target = self
            .shown
            .iter()
            .position(|&index| self.rows[index].t_ms() >= t_ms - 1_000)
            .unwrap_or(self.shown.len().saturating_sub(1));
        self.feed.scroll_to(ListOffset {
            item_ix: target,
            offset_in_item: px(0.),
        });
        cx.notify();
    }

    fn colour(&self, name: &str, cx: &App) -> Hsla {
        self.colours
            .get(name)
            .copied()
            .unwrap_or_else(|| style::person_color(name, cx))
    }

    fn own_name(&self, cx: &App) -> Option<String> {
        self.session.read(cx).user.as_ref().map(|user| {
            format!("{} {}", user.first_name, user.last_name)
                .trim()
                .to_owned()
        })
    }

    /// Faces overlapping a little, left to right, each ringed in `ring` (the colour
    /// behind them). Placed at fixed offsets: negative margins don't reserve their width.
    fn faces<'a>(
        &self,
        names: impl Iterator<Item = &'a String>,
        diameter: f32,
        ring: Hsla,
        cx: &App,
    ) -> gpui_kit::Div {
        let names: Vec<&String> = names.collect();
        let step = diameter * 0.72;
        let width = diameter + step * names.len().saturating_sub(1) as f32;
        let mut row = div()
            .relative()
            .flex_shrink_0()
            .w(px(width))
            .h(px(diameter));
        for (index, name) in names.into_iter().enumerate() {
            row = row.child(
                face(name, diameter, self.colour(name, cx), cx)
                    .absolute()
                    .left(px(index as f32 * step))
                    .top(px(0.))
                    .border_2()
                    .border_color(ring),
            );
        }
        row
    }

    // ---- the call list ------------------------------------------------------------------

    fn list(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.omarchy().clone();
        let selected = self.selected.as_ref().map(|call| call.id.clone());
        let mut list = div()
            .id("captures-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(8.))
            .pb(px(12.));
        if self.calls.is_empty() {
            list = list.child(div().px(px(8.)).child(style::muted_text(
                if self.snippets.is_empty() && self.query(cx).is_empty() {
                    "Nothing captured yet. Calls are captured as soon as they connect."
                } else {
                    "No call mentions that."
                },
                cx,
            )));
        }
        for call in &self.calls {
            let id = call.id.clone();
            let chosen = selected.as_deref() == Some(id.as_str());
            let when = when(call.started_ms);
            let length = match call.ended_ms {
                Some(ended) => store::duration(ended - call.started_ms),
                None => "capturing".into(),
            };
            let note = self
                .snippets
                .iter()
                .find(|(call_id, _)| *call_id == call.id)
                .map(|(_, text)| text.clone())
                .or_else(|| call.summary.clone());
            let faces = self
                .faces(call.participants.iter().take(5), 18., theme.inset, cx)
                .mt(px(6.));
            list = list.child(
                div()
                    .id(SharedString::from(format!("call-{id}")))
                    .px(px(10.))
                    .py(px(9.))
                    .rounded(px(9.))
                    .cursor_pointer()
                    .when(chosen, |row| {
                        row.bg(theme.surface)
                            .border_l_2()
                            .border_color(theme.accent)
                    })
                    .hover(|row| row.bg(theme.surface))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .when(call.live(), |row| {
                                row.child(
                                    icon(IconName::AudioLines)
                                        .size(px(12.))
                                        .text_color(theme.accent),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(13.))
                                    .text_color(theme.bright)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_ellipsis()
                                    .child(call.heading()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.secondary)
                            .child(format!("{when} · {length}")),
                    )
                    .children(note.map(|text| {
                        div()
                            .mt(px(3.))
                            .text_size(px(11.5))
                            .line_height(px(15.5))
                            .text_color(theme.secondary)
                            .line_clamp(2)
                            .child(text)
                    }))
                    .child(faces)
                    .on_click(cx.listener(move |view, _, _, cx| view.select(&id, cx))),
            );
        }
        list
    }

    // ---- the call -----------------------------------------------------------------------

    fn header(&self, call: &Call, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.omarchy().clone();
        let own = self
            .session
            .read(cx)
            .capture
            .as_ref()
            .filter(|capture| capture.id == call.id)
            .cloned();
        let elapsed = call
            .ended_ms
            .unwrap_or_else(store::now_ms)
            .saturating_sub(call.started_ms);
        let state = if call.live() {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .px(px(8.))
                .py(px(2.))
                .rounded_full()
                .bg(theme.danger.opacity(0.16))
                .text_color(theme.danger)
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(div().size(px(6.)).rounded_full().bg(theme.danger))
                .child(format!("Capturing · {}", store::clock(elapsed)))
        } else {
            div()
                .px(px(8.))
                .py(px(2.))
                .rounded_full()
                .bg(theme.surface)
                .text_color(theme.secondary)
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("Ended · {}", store::clock(elapsed)))
        };
        let drawings = self
            .rows
            .iter()
            .filter(|row| matches!(row, Row::Drawing(_)))
            .count();
        let screens = self
            .rows
            .iter()
            .filter(|row| matches!(row, Row::Screen(_)))
            .count();
        let mut sub = vec![store::local_time(call.started_ms, "%a %-d %b, %H:%M")];
        if drawings > 0 {
            sub.push(plural(drawings, "drawing"));
        }
        if screens > 0 {
            sub.push(plural(screens, "screen change"));
        }
        if let Some(capture) = &own {
            sub.push(super::faces::capture_state(capture));
        }
        let faces = self.faces(call.participants.iter(), 22., theme.background, cx);

        let mut actions = div().flex().items_center().gap(px(4.));
        if own.is_some() {
            actions = actions
                .child(
                    style::control(
                        "bookmark",
                        IconName::Bookmark,
                        Tone::Plain,
                        "Bookmark this moment",
                        cx,
                    )
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.session.update(cx, |session, _| {
                            session.send(crate::engine::Command::Bookmark(None))
                        })
                    })),
                )
                .child(
                    style::control(
                        "stop",
                        IconName::CircleStop,
                        Tone::Danger,
                        "Stop capturing",
                        cx,
                    )
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.session
                            .update(cx, |session, cx| session.toggle_capture(cx))
                    })),
                );
        }
        let prompt = Self::agent_prompt(call);
        actions = actions.child(
            style::control(
                "copy-prompt",
                if self.copied.is_some() {
                    IconName::Check
                } else {
                    IconName::Copy
                },
                Tone::Plain,
                "Copy a prompt for an agent to read this call",
                cx,
            )
            .on_click(cx.listener(move |view, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(prompt.clone()));
                view.copied = Some(Instant::now());
                cx.notify();
            })),
        );
        if !call.live() {
            let id = call.id.clone();
            actions = actions
                .child(
                    style::control(
                        "markdown",
                        IconName::FileText,
                        Tone::Plain,
                        "Open as Markdown",
                        cx,
                    )
                    .on_click(move |_, _, _| {
                        if let Ok(path) = crate::capture::markdown_path(&id) {
                            let _ = std::process::Command::new("xdg-open").arg(path).spawn();
                        }
                    }),
                )
                .child(
                    style::control(
                        "delete",
                        IconName::Trash,
                        if self.confirm_delete {
                            Tone::Danger
                        } else {
                            Tone::Plain
                        },
                        if self.confirm_delete {
                            "Click again to delete this call for good"
                        } else {
                            "Delete this call"
                        },
                        cx,
                    )
                    .on_click(cx.listener(|view, _, _, cx| view.delete(cx))),
                );
        }
        let narrow = f32::from(window.viewport_size().width) < NARROW;
        if narrow {
            actions = actions.child(
                style::control("toggle-list", IconName::List, Tone::Plain, "All calls", cx)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.show_list = true;
                        cx.notify();
                    })),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(18.))
            .pt(px(16.))
            .pb(px(12.))
            .border_b_1()
            .border_color(theme.border.opacity(0.6))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .text_ellipsis()
                                            .text_size(px(16.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.bright)
                                            .child(call.heading()),
                                    )
                                    .child(state),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .child(faces)
                                    .child(style::muted_text(sub.join(" · "), cx)),
                            ),
                    )
                    .child(actions),
            )
            .children(call.summary.clone().map(|summary| {
                div()
                    .mt(px(4.))
                    .px(px(12.))
                    .py(px(9.))
                    .rounded(px(9.))
                    .bg(theme.surface)
                    .text_color(theme.foreground)
                    .line_height(px(18.))
                    .child(summary)
            }))
            .into_any_element()
    }

    /// Who spoke when, drawings and screen changes across the whole call; click to jump.
    fn scrubber(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.omarchy().clone();
        let total = self.overview.duration_ms.max(1) as f32;
        let at = |t: i64| relative((t as f32 / total).clamp(0., 1.));
        let width = |from: i64, to: i64, least: f32| {
            relative((((to - from) as f32) / total).max(least).min(1.))
        };
        let people: Vec<String> = {
            let mut out: Vec<String> = Vec::new();
            for (who, _, _) in &self.overview.speech {
                if !out.contains(who) {
                    out.push(who.clone());
                }
            }
            out
        };
        let lanes = people.len().max(1) as f32;
        let lane_h = (24. / lanes).clamp(4., 8.);
        let mut track = div()
            .relative()
            .h(px(46.))
            .rounded(px(7.))
            .bg(theme.surface.opacity(0.7))
            .overflow_hidden()
            .cursor_pointer();
        for (who, from, to) in &self.overview.speech {
            let lane = people.iter().position(|name| name == who).unwrap_or(0) as f32;
            track = track.child(
                div()
                    .absolute()
                    .left(at(*from))
                    .top(px(6. + lane * (lane_h + 2.)))
                    .w(width(*from, *to, 0.003))
                    .h(px(lane_h))
                    .rounded(px(3.))
                    .bg(self.colour(who, cx)),
            );
        }
        for (from, to) in &self.overview.screens {
            track = track.child(
                div()
                    .absolute()
                    .left(at(*from))
                    .bottom(px(10.))
                    .w(width(*from, *to, 0.004))
                    .h(px(3.))
                    .rounded(px(2.))
                    .bg(theme.secondary.opacity(0.45)),
            );
        }
        for (t, who) in &self.overview.drawings {
            track = track.child(
                div()
                    .absolute()
                    .left(at(*t))
                    .bottom(px(3.))
                    .size(px(6.))
                    .rounded_full()
                    .bg(self.colour(who, cx)),
            );
        }
        let bounds = self.scrubber.clone();
        track = track.child(
            canvas(move |area, _, _| bounds.set(area), |_, _, _, _| {})
                .absolute()
                .size_full(),
        );
        let bounds = self.scrubber.clone();
        let duration = self.overview.duration_ms;
        let track = track.id("scrubber").on_mouse_down(
            gpui_kit::MouseButton::Left,
            cx.listener(move |view, event: &gpui_kit::MouseDownEvent, _, cx| {
                let area = bounds.get();
                let width = f32::from(area.size.width).max(1.);
                let fraction = (f32::from(event.position.x - area.origin.x) / width).clamp(0., 1.);
                view.jump((fraction * duration as f32) as i64, cx);
            }),
        );
        let mut ticks = div()
            .flex()
            .justify_between()
            .mt(px(3.))
            .font_family(theme.mono_font.clone())
            .text_size(px(10.5))
            .text_color(theme.secondary.opacity(0.7));
        let step = tick_step(duration);
        let mut t = 0;
        while t < duration - step / 3 {
            ticks = ticks.child(store::clock(t));
            t += step;
        }
        ticks = ticks.child(store::clock(duration));
        div()
            .px(px(18.))
            .pt(px(10.))
            .pb(px(8.))
            .border_b_1()
            .border_color(theme.border.opacity(0.6))
            .child(track)
            .child(ticks)
            .into_any_element()
    }

    fn tools(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.omarchy().clone();
        let count = |filter: Filter| {
            self.rows
                .iter()
                .filter(|row| filter == Filter::Everything || row.filter() == filter)
                .count()
        };
        let mut chips = div().flex().items_center().gap(px(6.));
        for (filter, label) in [
            (Filter::Everything, "Everything"),
            (Filter::Said, "Said"),
            (Filter::Drawings, "Drawings"),
            (Filter::Screen, "Screen"),
        ] {
            let on = self.filter == filter;
            let n = count(filter);
            if filter != Filter::Everything && n == 0 {
                continue;
            }
            chips = chips.child(
                div()
                    .id(SharedString::from(format!("filter-{label}")))
                    .flex()
                    .gap(px(5.))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded_full()
                    .border_1()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .map(|chip| {
                        if on {
                            chip.bg(theme.surface)
                                .border_color(theme.surface)
                                .text_color(theme.bright)
                        } else {
                            chip.border_color(theme.border.opacity(0.7))
                                .text_color(theme.secondary)
                                .hover(|chip| chip.bg(theme.surface.opacity(0.6)))
                        }
                    })
                    .child(label)
                    .when(filter != Filter::Everything, |chip| {
                        chip.child(
                            div()
                                .text_color(theme.secondary.opacity(0.8))
                                .child(n.to_string()),
                        )
                    })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.filter = filter;
                        view.refilter(cx);
                        cx.notify();
                    })),
            );
        }
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(18.))
            .py(px(9.))
            .border_b_1()
            .border_color(theme.border.opacity(0.6))
            .child(chips)
            .child(div().flex_1())
            .child(
                div().w(px(210.)).child(
                    input("find-in-call", &self.find, window, cx)
                        .prefix(icon(IconName::Search).size(px(14.))),
                ),
            )
            .into_any_element()
    }

    /// One row of the timeline, centred at reading width with the time in the gutter.
    fn row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self
            .shown
            .get(ix)
            .and_then(|&index| self.rows.get(index))
            .cloned()
        else {
            return div().into_any_element();
        };
        let theme = cx.omarchy().clone();
        let t = row.t_ms();
        let time = div()
            .id(SharedString::from(format!("time-{ix}")))
            .w(px(52.))
            .flex_shrink_0()
            .pt(px(3.))
            .text_right()
            .font_family(theme.mono_font.clone())
            .text_size(px(11.))
            .text_color(theme.secondary.opacity(0.6))
            .cursor_pointer()
            .hover(|time| time.text_color(theme.bright))
            .child(store::clock(t));
        let body: AnyElement = match &row {
            Row::Event { text, .. } => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pt(px(3.))
                .text_size(px(12.))
                .text_color(theme.secondary)
                .child(
                    div()
                        .size(px(6.))
                        .rounded_full()
                        .bg(theme.secondary.opacity(0.6)),
                )
                .child(text.clone())
                .into_any_element(),
            Row::Bookmark { who, text, .. } => div()
                .flex()
                .items_center()
                .gap(px(6.))
                .pt(px(2.))
                .text_color(theme.warning)
                .child(icon(IconName::Bookmark).size(px(13.)))
                .child(match who {
                    Some(who) => format!("{who}: {text}"),
                    None => text.clone(),
                })
                .into_any_element(),
            Row::Gap { text, .. } => div()
                .pt(px(3.))
                .text_size(px(12.))
                .text_color(theme.danger.opacity(0.85))
                .child(format!("Not transcribed: {text}"))
                .into_any_element(),
            Row::Said {
                speaker,
                text,
                draft,
                also,
                ..
            } => self.said(speaker, text, *draft, also.as_deref(), cx),
            Row::Drawing(drawing) => self.drawing(drawing, cx),
            Row::Screen(burst) => self.burst(burst, cx),
        };
        let padding = if matches!(row, Row::Drawing(_) | Row::Screen(_)) {
            px(8.)
        } else {
            px(5.)
        };
        let _ = window;
        div()
            .w_full()
            .flex()
            .justify_center()
            .px(px(18.))
            .child(
                div()
                    .w_full()
                    .max_w(px(COLUMN))
                    .flex()
                    .gap(px(12.))
                    .py(padding)
                    .child(time.on_click(cx.listener(move |view, _, _, cx| view.jump(t, cx))))
                    .child(div().flex_1().min_w_0().child(body)),
            )
            .into_any_element()
    }

    fn said(
        &self,
        speaker: &str,
        text: &str,
        draft: bool,
        also: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.omarchy().clone();
        let colour = self.colour(speaker, cx);
        let you = self.own_name(cx).is_some_and(|own| own == speaker);
        let find = self.find.read(cx).value().trim().to_lowercase();
        let words: AnyElement = if draft {
            div()
                .italic()
                .text_color(theme.secondary.opacity(0.8))
                .child(if text.is_empty() {
                    "…".to_owned()
                } else {
                    format!("{text} …")
                })
                .into_any_element()
        } else if !find.is_empty() {
            let lower = text.to_lowercase();
            let highlights: Vec<_> = lower
                .match_indices(&find)
                .filter(|(start, _)| {
                    text.is_char_boundary(*start) && text.is_char_boundary(start + find.len())
                })
                .map(|(start, found)| {
                    (
                        start..start + found.len(),
                        HighlightStyle {
                            background_color: Some(theme.warning.opacity(0.35)),
                            ..Default::default()
                        },
                    )
                })
                .collect();
            StyledText::new(text.to_owned())
                .with_highlights(highlights)
                .into_any_element()
        } else {
            div().child(text.to_owned()).into_any_element()
        };
        div()
            .flex()
            .gap(px(10.))
            .child(face(speaker, 26., colour, cx).mt(px(1.)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(8.))
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colour)
                            .child(speaker.to_owned())
                            .when(you, |who| {
                                who.child(
                                    div()
                                        .font_weight(FontWeight::NORMAL)
                                        .text_size(px(11.5))
                                        .text_color(theme.secondary)
                                        .child("you"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .mt(px(1.))
                            .text_size(px(14.))
                            .line_height(px(21.))
                            .text_color(theme.bright)
                            .child(words),
                    )
                    .children(also.map(|other| {
                        div()
                            .mt(px(2.))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .text_size(px(11.))
                            .text_color(theme.secondary.opacity(0.8))
                            .child(icon(IconName::AudioLines).size(px(11.)))
                            .child(format!("also on {}'s mic, shown once", first_name(other)))
                    })),
            )
            .into_any_element()
    }

    fn drawing(&self, drawing: &Drawing, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.omarchy().clone();
        let own = self.own_name(cx);
        let frame = drawing.frame;
        let whole = self.whole.contains(&frame);
        let people = drawing.people();
        let who = people
            .iter()
            .map(|name| {
                if own.as_deref() == Some(name.as_str()) {
                    "You".to_owned()
                } else {
                    first_name(name)
                }
            })
            .collect::<Vec<_>>();
        let who = match who.as_slice() {
            [] => "Someone".to_owned(),
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        let faces = self.faces(people.iter(), 18., theme.inset, cx);
        let span = if drawing.end_ms - drawing.t_ms >= 1_000 {
            format!(
                "{}–{}",
                store::clock(drawing.t_ms),
                store::clock(drawing.end_ms)
            )
        } else {
            String::new()
        };
        let segment = |id: &'static str, label: &'static str, on: bool, make_whole: bool| {
            div()
                .id(SharedString::from(format!("{id}-{frame}")))
                .px(px(8.))
                .py(px(2.))
                .cursor_pointer()
                .text_size(px(11.5))
                .map(|button| {
                    if on {
                        button.bg(theme.surface).text_color(theme.bright)
                    } else {
                        button
                            .text_color(theme.secondary)
                            .hover(|button| button.text_color(theme.bright))
                    }
                })
                .child(label)
                .on_click(cx.listener(move |view, _, _, cx| {
                    if make_whole {
                        view.whole.insert(frame);
                    } else {
                        view.whole.remove(&frame);
                    }
                    view.feed.remeasure();
                    cx.notify();
                }))
        };
        let head = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .text_size(px(12.5))
            .child(faces)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.bright)
                    .child(format!("{who} drew")),
            )
            .child(div().flex_1())
            .child(div().text_color(theme.secondary).child(span))
            .child(
                div()
                    .flex()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(theme.border.opacity(0.7))
                    .overflow_hidden()
                    .child(segment("zoomed", "Drawing", !whole, false))
                    .child(segment("whole", "Whole screen", whole, true)),
            );
        let crop = if whole {
            [0., 0., 1., 1.]
        } else {
            drawing.crop
        };
        let lit = self
            .lit
            .as_ref()
            .filter(|(lit_frame, _)| *lit_frame == frame)
            .map(|(_, marks)| marks.clone())
            .unwrap_or_default();
        let picture = self.cropped(
            frame,
            card_path(self.call_id(), frame),
            (drawing.width, drawing.height),
            crop,
            &drawing.marks,
            &lit,
            cx,
        );
        let index = self
            .gallery
            .iter()
            .position(|picture| picture.frame == frame);
        let picture = div()
            .id(SharedString::from(format!("picture-{frame}")))
            .cursor_pointer()
            .child(picture)
            .on_click(cx.listener(move |view, _, _, cx| {
                view.viewing = index;
                cx.notify();
            }));
        div()
            .rounded(px(12.))
            .border_1()
            .border_color(theme.border.opacity(0.7))
            .bg(theme.inset)
            .overflow_hidden()
            .child(head)
            .child(picture)
            .child(self.mark_chips(frame, &drawing.marks, cx))
            .into_any_element()
    }

    /// `frame`, showing only `crop` of it, with `lit` marks outlined.
    #[allow(clippy::too_many_arguments)]
    fn cropped(
        &self,
        frame: u32,
        path: PathBuf,
        (width, height): (u32, u32),
        crop: [f64; 4],
        marks: &[MarkInfo],
        lit: &[usize],
        cx: &App,
    ) -> AnyElement {
        let [left, top, w, h] = crop.map(|v| v as f32);
        let aspect = (w * width.max(1) as f32) / (h * height.max(1) as f32);
        let mut layer = div()
            .absolute()
            .left(relative(-left / w))
            .top(relative(-top / h))
            .w(relative(1. / w))
            .h(relative(1. / h))
            .child(img(path).size_full().object_fit(ObjectFit::Fill));
        for &index in lit {
            let Some(mark) = marks.get(index) else {
                continue;
            };
            let colour = hex(&mark.colour).unwrap_or_else(|| cx.omarchy().accent);
            let [l, t, r, b] = mark.bounds.map(|v| v as f32);
            let (pad_x, pad_y) = (0.006, 0.014);
            let (x, y) = (l.min(r) - pad_x, t.min(b) - pad_y);
            let (bw, bh) = ((r - l).abs() + pad_x * 2., (b - t).abs() + pad_y * 2.);
            layer = layer.child(
                div()
                    .absolute()
                    .left(relative(x))
                    .top(relative(y))
                    .w(relative(bw.max(0.02)))
                    .h(relative(bh.max(0.04)))
                    .rounded(px(6.))
                    .border_2()
                    .border_color(colour)
                    .bg(colour.opacity(0.14)),
            );
        }
        div()
            .id(SharedString::from(format!("crop-{frame}")))
            .relative()
            .w_full()
            .aspect_ratio(aspect)
            .overflow_hidden()
            .bg(gpui_kit::black())
            .child(layer)
            .into_any_element()
    }

    /// One chip per person and kind of mark; hovering one outlines those marks.
    fn mark_chips(&self, frame: u32, marks: &[MarkInfo], cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.omarchy().clone();
        let mut groups: Vec<((String, String), Vec<usize>)> = Vec::new();
        for (index, mark) in marks.iter().enumerate() {
            let key = (mark.by.clone(), mark.shape.clone());
            match groups.iter_mut().find(|(seen, _)| *seen == key) {
                Some((_, indexes)) => indexes.push(index),
                None => groups.push((key, vec![index])),
            }
        }
        let mut chips = div().flex().flex_wrap().gap(px(6.)).px(px(12.)).py(px(10.));
        for (number, ((who, shape), indexes)) in groups.into_iter().enumerate() {
            let label = if shape == "pin" {
                let pins: Vec<String> = indexes
                    .iter()
                    .filter_map(|&i| marks[i].pin.map(|pin| pin.to_string()))
                    .collect();
                format!(
                    "{} · {} {}",
                    first_name(&who),
                    if pins.len() == 1 { "pin" } else { "pins" },
                    pins.join(", ")
                )
            } else if let Some(text) = indexes
                .first()
                .and_then(|&i| marks[i].text.clone())
                .filter(|_| indexes.len() == 1)
            {
                format!("{} · “{text}”", first_name(&who))
            } else {
                let noun = match shape.as_str() {
                    "stroke" => "drawing",
                    other => other,
                };
                format!(
                    "{} · {} {noun}{}",
                    first_name(&who),
                    indexes.len(),
                    if indexes.len() == 1 { "" } else { "s" }
                )
            };
            let colour = self.colour(&who, cx);
            chips = chips.child(
                div()
                    .id(SharedString::from(format!("mark-{frame}-{number}")))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pl(px(7.))
                    .pr(px(10.))
                    .py(px(3.))
                    .rounded_full()
                    .bg(theme.surface)
                    .text_size(px(12.))
                    .text_color(theme.bright)
                    .hover(|chip| chip.bg(theme.selection))
                    .child(div().size(px(9.)).rounded_full().bg(colour))
                    .child(label)
                    .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                        view.lit = hovered.then(|| (frame, indexes.clone()));
                        cx.notify();
                    })),
            );
        }
        chips.into_any_element()
    }

    fn burst(&self, burst: &Burst, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.omarchy().clone();
        let call = self.call_id();
        let span = if burst.end_ms - burst.t_ms >= 1_000 {
            format!(
                "{}–{}",
                store::clock(burst.t_ms),
                store::clock(burst.end_ms)
            )
        } else {
            String::new()
        };
        let mut strip = div()
            .id(SharedString::from(format!("strip-{}", burst.t_ms)))
            .flex()
            .gap(px(8.))
            .px(px(12.))
            .pb(px(12.))
            .overflow_x_scroll();
        for &(frame, t_ms) in &burst.thumbs {
            let index = self
                .gallery
                .iter()
                .position(|picture| picture.frame == frame);
            strip = strip.child(
                div()
                    .id(SharedString::from(format!("thumb-{frame}")))
                    .flex_shrink_0()
                    .cursor_pointer()
                    .child(
                        div()
                            .w(px(140.))
                            .h(px(60.))
                            .rounded(px(6.))
                            .overflow_hidden()
                            .border_1()
                            .border_color(theme.border.opacity(0.7))
                            .hover(|thumb| thumb.border_color(theme.accent))
                            .child(
                                img(card_path(call.clone(), frame))
                                    .size_full()
                                    .object_fit(ObjectFit::Cover),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(3.))
                            .font_family(theme.mono_font.clone())
                            .text_size(px(10.5))
                            .text_color(theme.secondary.opacity(0.75))
                            .child(store::clock(t_ms)),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.viewing = index;
                        cx.notify();
                    })),
            );
        }
        div()
            .rounded(px(12.))
            .border_1()
            .border_color(theme.border.opacity(0.7))
            .bg(theme.inset)
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(8.))
                    .text_size(px(12.5))
                    .child(
                        icon(IconName::Monitor)
                            .size(px(13.))
                            .text_color(theme.secondary),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.bright)
                            .child(if burst.count == 1 {
                                "The shared screen".to_owned()
                            } else {
                                format!("The shared screen changed {} times", burst.count)
                            }),
                    )
                    .child(div().flex_1())
                    .child(div().text_color(theme.secondary).child(span)),
            )
            .child(strip)
            .into_any_element()
    }

    fn call_id(&self) -> String {
        self.selected
            .as_ref()
            .map(|call| call.id.clone())
            .unwrap_or_default()
    }

    /// A picture full size over the window: arrows step through the call's pictures.
    fn viewer(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.omarchy().clone();
        let Some(picture) = self.gallery.get(index).cloned() else {
            return div().into_any_element();
        };
        let call = self.call_id();
        let count = self.gallery.len();
        let step = move |by: isize| {
            move |view: &mut Self, cx: &mut Context<Self>| {
                view.viewing = Some((index as isize + by).rem_euclid(count as isize) as usize);
                cx.notify();
            }
        };
        let (prev, next) = (step(-1), step(1));
        let info = self
            .frames
            .iter()
            .find(|frame| frame.number == picture.frame)
            .map(|frame| (frame.width, frame.height))
            .unwrap_or((16, 9));
        let lit: Vec<usize> = (0..picture.marks.len()).collect();
        let shown = self.cropped(
            picture.frame,
            full_path(&call, picture.frame),
            info,
            [0., 0., 1., 1.],
            &picture.marks,
            if self.lit.is_some() { &lit } else { &[] },
            cx,
        );
        let mut strip = div().flex().justify_center().gap(px(6.)).mt(px(12.));
        let from = index.saturating_sub(6);
        for (offset, other) in self.gallery.iter().enumerate().skip(from).take(13) {
            let on = offset == index;
            strip = strip.child(
                div()
                    .id(SharedString::from(format!("viewer-thumb-{offset}")))
                    .w(px(72.))
                    .h(px(31.))
                    .rounded(px(4.))
                    .overflow_hidden()
                    .cursor_pointer()
                    .border_1()
                    .map(|thumb| {
                        if on {
                            thumb.border_color(theme.accent)
                        } else {
                            thumb.border_color(theme.border).opacity(0.55)
                        }
                    })
                    .child(
                        img(card_path(call.clone(), other.frame))
                            .size_full()
                            .object_fit(ObjectFit::Cover),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.viewing = Some(offset);
                        cx.notify();
                    })),
            );
        }
        div()
            .id("viewer")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .flex_col()
            .p(px(18.))
            .bg(theme.background.opacity(0.97))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .mb(px(12.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.bright)
                            .child(picture.title.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .text_color(theme.secondary)
                            .child(picture.detail.clone()),
                    )
                    .child(
                        style::control(
                            "viewer-prev",
                            IconName::ChevronLeft,
                            Tone::Plain,
                            "Previous (←)",
                            cx,
                        )
                        .on_click(cx.listener(move |view, _, _, cx| prev(view, cx))),
                    )
                    .child(
                        style::control(
                            "viewer-next",
                            IconName::ChevronRight,
                            Tone::Plain,
                            "Next (→)",
                            cx,
                        )
                        .on_click(cx.listener(move |view, _, _, cx| next(view, cx))),
                    )
                    .child(
                        style::control("viewer-close", IconName::X, Tone::Plain, "Close (Esc)", cx)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.viewing = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(1600.))
                            .rounded(px(10.))
                            .overflow_hidden()
                            .shadow_lg()
                            .child(shown),
                    ),
            )
            .child(strip)
            .into_any_element()
    }

    fn step_viewer(&mut self, by: isize, cx: &mut Context<Self>) {
        if let Some(index) = self.viewing
            && !self.gallery.is_empty()
        {
            self.viewing =
                Some((index as isize + by).rem_euclid(self.gallery.len() as isize) as usize);
            cx.notify();
        }
    }

    fn detail(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.omarchy().clone();
        let Some(call) = self.selected.clone() else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(style::muted_text("Choose a call", cx))
                .into_any_element();
        };
        let header = self.header(&call, window, cx);
        let scrubber = self.scrubber(cx);
        let tools = self.tools(window, cx);
        let empty =
            if self.shown.is_empty() {
                Some(div().flex_1().flex().items_center().justify_center().child(
                    style::muted_text(
                        if !self.rows.is_empty() {
                            "Nothing here matches."
                        } else if call.live() {
                            "Listening. What is said appears within a few seconds."
                        } else {
                            "Nothing was said."
                        },
                        cx,
                    ),
                ))
            } else {
                None
            };
        let feed = list(
            self.feed.clone(),
            cx.processor(|view, ix, window, cx| view.row(ix, window, cx)),
        )
        .flex_1()
        .min_h_0()
        .pt(px(8.))
        .pb(px(24.));
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme.background)
            .child(header)
            .when(!self.rows.is_empty(), |detail| {
                detail.child(scrubber).child(tools)
            })
            .map(|detail| match empty {
                Some(empty) => detail.child(empty),
                None => detail.child(
                    image_cache(self.images.clone())
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .child(feed),
                ),
            })
            .into_any_element()
    }
}

impl Render for CapturesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let narrow = f32::from(window.viewport_size().width) < NARROW;
        let session = self.session.clone();
        let mut root = div()
            .relative()
            .size_full()
            .flex()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font.clone())
            .text_size(px(12.5))
            .on_key_down(
                cx.listener(move |view, event: &gpui_kit::KeyDownEvent, window, cx| {
                    let key = event.keystroke.key.as_str();
                    if view.viewing.is_some() {
                        match key {
                            "escape" => {
                                view.viewing = None;
                                cx.notify();
                            }
                            "left" => view.step_viewer(-1, cx),
                            "right" => view.step_viewer(1, cx),
                            _ => return,
                        }
                        cx.stop_propagation();
                    } else if key == "escape" {
                        window.remove_window();
                        cx.stop_propagation();
                    } else if super::global_shortcut(&event.keystroke, event.is_held, &session, cx)
                    {
                        cx.stop_propagation();
                    }
                }),
            );
        if let Err(error) = &self.store {
            return root.child(
                div()
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(style::section_label("CAPTURES", cx))
                    .child(style::muted_text(
                        format!("The capture database couldn't be opened: {error}"),
                        cx,
                    )),
            );
        }
        let side = div()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(theme.inset)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .pt(px(16.))
                    .pb(px(10.))
                    .child(
                        icon(IconName::Captions)
                            .size(px(15.))
                            .text_color(theme.accent),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.bright)
                            .child("Calls"),
                    )
                    .child(
                        style::control(
                            "capture-settings",
                            IconName::Settings,
                            Tone::Plain,
                            "Capture settings",
                            cx,
                        )
                        .on_click(|_, _, cx| {
                            super::Shell::show_settings_at(super::settings::Section::Capture, cx)
                        }),
                    )
                    .when(narrow, |row| {
                        row.child(
                            style::control(
                                "back",
                                IconName::X,
                                Tone::Plain,
                                "Back to the call",
                                cx,
                            )
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.show_list = false;
                                cx.notify();
                            })),
                        )
                    }),
            )
            .child(
                div().px(px(12.)).pb(px(10.)).child(
                    input("capture-search", &self.search, window, cx)
                        .prefix(icon(IconName::Search).size(px(14.))),
                ),
            )
            .child(self.list(cx));
        if narrow {
            root = if self.show_list {
                root.child(side.flex_1())
            } else {
                root.child(self.detail(window, cx))
            };
        } else {
            root = root
                .child(side.w(px(256.)).flex_shrink_0())
                .child(div().w(px(1.)).bg(theme.border.opacity(0.6)))
                .child(self.detail(window, cx));
        }
        if let Some(index) = self.viewing {
            root = root.child(self.viewer(index, cx));
        }
        root
    }
}

pub fn open(session: Entity<Session>, cx: &mut App) -> Option<WindowHandle<CapturesView>> {
    let options = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1180.), px(820.)),
            cx,
        ))),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("Hopp captures".into()),
            ..Default::default()
        }),
        app_id: Some("hopp.captures".into()),
        window_min_size: Some(size(px(360.), px(320.))),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        cx.new(|cx| CapturesView::new(session, window, cx))
    })
    .ok()
}

/// A round face with initials in the person's colour.
fn face(name: &str, diameter: f32, colour: Hsla, _cx: &App) -> gpui_kit::Div {
    div()
        .size(px(diameter))
        .flex_shrink_0()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(colour)
        .text_color(gpui_kit::hsla(0., 0., 0.08, 1.))
        .text_size(px((diameter * 0.4).max(7.)))
        .font_weight(FontWeight::BOLD)
        .child(initials(name))
}

fn initials(name: &str) -> String {
    let letters: String = name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect();
    if letters.is_empty() {
        "?".into()
    } else {
        letters.to_uppercase()
    }
}

fn first_name(name: &str) -> String {
    name.split_whitespace().next().unwrap_or(name).to_owned()
}

fn plural(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// "Today 18:27", "Mon 16:15" or "3 Nov 16:15".
fn when(started_ms: i64) -> String {
    let today = store::local_time(store::now_ms(), "%Y-%m-%d");
    let day = store::local_time(started_ms, "%Y-%m-%d");
    if day == today {
        store::local_time(started_ms, "Today %H:%M")
    } else if store::now_ms() - started_ms < 6 * 24 * 3_600_000 {
        store::local_time(started_ms, "%a %H:%M")
    } else {
        store::local_time(started_ms, "%-d %b %H:%M")
    }
}

/// Ticks every minute for short calls, then wider.
fn tick_step(duration_ms: i64) -> i64 {
    let minutes = duration_ms / 60_000;
    let step = match minutes {
        0..=6 => 1,
        7..=15 => 3,
        16..=40 => 5,
        41..=90 => 10,
        _ => 20,
    };
    if duration_ms < 60_000 {
        15_000
    } else {
        step * 60_000
    }
}

fn hex(colour: &str) -> Option<Hsla> {
    let digits = colour.strip_prefix('#')?;
    let value = u32::from_str_radix(digits, 16).ok()?;
    (digits.len() == 6).then(|| rgb(value).into())
}

/// Everyone's colour: the colour their marks were drawn in when the frames say, so a
/// person's words, cursor and ink match; otherwise the transcript palette.
fn colours(call: &Call, lines: &[Line], frames: &[FrameInfo], cx: &App) -> HashMap<String, Hsla> {
    let speakers = Speakers::new(
        &call.participants,
        lines.iter().filter_map(|line| line.speaker.as_deref()),
    );
    let mut out = HashMap::new();
    for name in call
        .participants
        .iter()
        .map(String::as_str)
        .chain(lines.iter().filter_map(|line| line.speaker.as_deref()))
    {
        out.entry(name.to_owned())
            .or_insert_with(|| speakers.color(name, cx));
    }
    for frame in frames {
        for mark in frame.marks.iter().chain(&frame.ink) {
            if let Some(colour) = hex(&mark.colour) {
                out.insert(mark.by.clone(), colour);
            }
        }
    }
    out
}

/// Every picture in the call, in timeline order, for the viewer.
fn gallery(rows: &[Row], frames: &[FrameInfo]) -> Vec<Picture> {
    let mut out = Vec::new();
    for row in rows {
        match row {
            Row::Drawing(drawing) => out.push(Picture {
                frame: drawing.frame,
                title: format!("Drawing · {}", store::clock(drawing.t_ms)),
                detail: if drawing.frames > 1 {
                    format!(
                        "{} · drawn over {} frames",
                        summary(&drawing.marks),
                        drawing.frames
                    )
                } else {
                    summary(&drawing.marks)
                },
                marks: drawing.marks.clone(),
            }),
            Row::Screen(burst) => {
                let numbers: Vec<u32> = frames
                    .iter()
                    .filter(|frame| {
                        frame.trigger == "screen"
                            && frame.t_ms >= burst.t_ms
                            && frame.t_ms <= burst.end_ms
                    })
                    .map(|frame| frame.number)
                    .collect();
                for frame in frames
                    .iter()
                    .filter(|frame| numbers.contains(&frame.number))
                {
                    out.push(Picture {
                        frame: frame.number,
                        title: format!("Screen · {}", store::clock(frame.t_ms)),
                        detail: format!("Frame {}", frame.number),
                        marks: Vec::new(),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// "Alex: 2 drawings · Sam: 3 pins, 1 line".
fn summary(marks: &[MarkInfo]) -> String {
    let mut people: Vec<(String, Vec<(String, usize)>)> = Vec::new();
    for mark in marks {
        let shape = match mark.shape.as_str() {
            "stroke" => "drawing",
            other => other,
        }
        .to_owned();
        let person = match people.iter_mut().find(|(who, _)| *who == mark.by) {
            Some(person) => person,
            None => {
                people.push((mark.by.clone(), Vec::new()));
                people.last_mut().expect("just pushed")
            }
        };
        match person.1.iter_mut().find(|(seen, _)| *seen == shape) {
            Some((_, n)) => *n += 1,
            None => person.1.push((shape, 1)),
        }
    }
    people
        .into_iter()
        .map(|(who, shapes)| {
            let shapes: Vec<String> = shapes
                .into_iter()
                .map(|(shape, n)| plural(n, &shape))
                .collect();
            format!("{}: {}", first_name(&who), shapes.join(", "))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The picture for cards and thumbnails: the small copy (1440 px on its long edge).
fn card_path(call_id: String, frame: u32) -> PathBuf {
    moments::small(&call_id, frame).unwrap_or_default()
}

/// The sharpest copy for the viewer: full size if kept, else the card picture.
fn full_path(call_id: &str, frame: u32) -> PathBuf {
    moments::full(call_id, frame).unwrap_or_default()
}

fn frame_lines(lines: &[Line]) -> usize {
    lines.iter().filter(|line| line.kind == Kind::Frame).count()
}

/// Take in lines written since `cursor` (a revision): new ones, and drafts that grew,
/// became final or were retracted, each in place under its id. A line keeps the place
/// it had when its speech began, so nothing already shown moves.
fn merge(lines: &mut Vec<Line>, cursor: &mut i64, changed: Vec<Line>) {
    for line in changed {
        *cursor = (*cursor).max(line.rev);
        let existing = lines.iter().position(|shown| shown.id == line.id);
        match (existing, line.kind) {
            (Some(index), Kind::Retracted) => {
                lines.remove(index);
            }
            (None, Kind::Retracted) => {}
            (Some(index), _) => lines[index] = line,
            (None, _) => lines.push(line),
        }
    }
    store::sort(lines);
}

/// Decoded frames, the least recently shown let go past [`IMAGES`], so a long call
/// doesn't hold every frame in memory (each card ~3.5 MB decoded, full size ~20 MB).
pub struct FrameImages {
    items: HashMap<u64, (ImageCacheItem, u64)>,
    tick: u64,
}

impl FrameImages {
    fn new(cx: &mut App) -> Entity<Self> {
        let cache = cx.new(|_| Self {
            items: HashMap::new(),
            tick: 0,
        });
        cx.observe_release(&cache, |cache, cx| {
            for (_, (item, _)) in cache.items.drain() {
                if let Some(Ok(image)) = item.get() {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();
        cache
    }
}

impl ImageCache for FrameImages {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let mut hasher = DefaultHasher::new();
        resource.hash(&mut hasher);
        let key = hasher.finish();
        self.tick += 1;
        let tick = self.tick;
        let entry = self
            .items
            .entry(key)
            .or_insert_with(|| (ImageCacheItem::new(resource, cx), tick));
        entry.1 = tick;
        let loaded = entry.0.use_image(window);
        if self.items.len() > IMAGES
            && let Some(oldest) = self
                .items
                .iter()
                .filter(|(other, _)| **other != key)
                .min_by_key(|(_, (_, used))| *used)
                .map(|(other, _)| *other)
            && let Some((item, _)) = self.items.remove(&oldest)
            && let Some(Ok(image)) = item.get()
        {
            cx.drop_image(image, Some(window));
        }
        loaded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: i64, rev: i64, t_ms: i64, kind: Kind, text: &str) -> Line {
        Line {
            id,
            t_ms,
            end_ms: None,
            kind,
            speaker_id: Some("mira".into()),
            speaker: Some("Mira".into()),
            text: text.into(),
            joins: Vec::new(),
            rev,
        }
    }

    /// A draft stays where its speech began while it grows and becomes final, and a
    /// retracted one disappears; lines that finish later still go where they were said.
    #[test]
    fn drafts_change_in_place_and_nothing_shown_moves() {
        let (mut lines, mut cursor) = (Vec::new(), 0);
        merge(
            &mut lines,
            &mut cursor,
            vec![
                line(1, 1, 0, Kind::Speech, "First."),
                line(2, 2, 5_000, Kind::Draft, ""),
                line(3, 3, 7_000, Kind::Draft, ""),
            ],
        );
        merge(
            &mut lines,
            &mut cursor,
            vec![
                line(2, 4, 5_000, Kind::Draft, "So the"),
                line(4, 5, 2_000, Kind::Speech, "Said earlier, written late."),
                line(3, 6, 7_000, Kind::Retracted, ""),
                line(2, 7, 5_000, Kind::Speech, "So the cache is warm."),
            ],
        );
        assert_eq!(cursor, 7);
        let shown: Vec<_> = lines
            .iter()
            .map(|line| (line.id, line.kind, line.text.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                (1, Kind::Speech, "First."),
                (4, Kind::Speech, "Said earlier, written late."),
                (2, Kind::Speech, "So the cache is warm."),
            ]
        );
    }

    #[test]
    fn marks_are_summed_up_per_person() {
        let mark = |by: &str, shape: &str| MarkInfo {
            by: by.into(),
            shape: shape.into(),
            pin: None,
            text: None,
            bounds: [0.; 4],
            colour: "#0a84ff".into(),
        };
        let marks = [
            mark("Alex Morgan", "stroke"),
            mark("Sam Taylor", "pin"),
            mark("Alex Morgan", "stroke"),
            mark("Sam Taylor", "line"),
            mark("Sam Taylor", "pin"),
        ];
        assert_eq!(summary(&marks), "Alex: 2 drawings · Sam: 2 pins, 1 line");
    }

    #[test]
    fn colours_come_from_hex() {
        assert!(hex("#0a84ff").is_some());
        assert!(hex("0a84ff").is_none());
        assert!(hex("#fff").is_none());
    }
}
