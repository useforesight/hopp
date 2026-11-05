//! Hopp for Omarchy: small windows that fit a tiling desktop instead of one big one.
//!
//! - `home`: teammates, rooms, call and settings (the bar plugin's fallback).
//! - `faces`: floating, pinned, fixed-size call window.
//! - `viewer`: the shared screen as a tiled viewport that follows the sharer.
//! - `viewer` in peek mode: the whole screen in `special:hopp`.
//! - `toast`: incoming and outgoing calls as a layer-shell card.
//!
//! The `Shell` opens and closes those windows from session state, so the call itself
//! never depends on any one window staying open.
mod ambient;
mod captures;
mod cards;
mod demo;
mod drawpad;
mod faces;
mod geometry;
mod home;
mod hypr;
mod keys;
mod marks;
mod overlay;
mod painted;
mod palette;
mod session;
mod settings;
mod sharing;
mod style;
mod toast;
mod tones;
mod viewer;

pub use demo::run as marks_demo;

use std::time::Duration;

use gpui_kit::{App, AppContext, Entity, Global, Keystroke, WindowHandle};

use crate::{engine::Engine, settings::Settings};
pub use session::{Session, SessionEvent};

/// Receives JSON state snapshots for the bar plugin.
pub type Publisher = Box<dyn Fn(&str)>;

pub struct Options {
    /// Exit after three seconds (packaging check).
    pub smoke_test: bool,
    /// Start without opening the Hopp window (the bar plugin provides the UI).
    pub background: bool,
    /// Screenshot fixture only: open just this kind of window
    /// (`home`, `faces`, `viewer`, `peek` or `call`).
    #[doc(hidden)]
    pub only: Option<String>,
    /// Receives a JSON state snapshot whenever it changes (for the bar plugin).
    pub publish: Option<Publisher>,
}

pub struct Shell {
    session: Entity<Session>,
    home: Option<WindowHandle<home::HomeView>>,
    faces: Option<WindowHandle<faces::FacesView>>,
    viewer: Option<WindowHandle<viewer::ViewerView>>,
    peek: Option<WindowHandle<viewer::ViewerView>>,
    toast: Option<WindowHandle<toast::ToastView>>,
    /// Cards from agents, with how many it was sized for and whether it sits below
    /// the call card.
    cards: Option<(WindowHandle<cards::CardsView>, usize, bool)>,
    sharing: Option<WindowHandle<sharing::SharingView>>,
    /// Others' marks over the monitor you share.
    marks: Option<WindowHandle<overlay::MarksOverlay>>,
    /// The marks overlay couldn't open over this share, and you've been told.
    marks_failed: bool,
    /// Whether this share can have Hopp's overlays over it without viewers seeing them
    /// (see `hypr::clean_share`). Asked once per share.
    clean_share: Option<bool>,
    /// The pad you draw on your own shared screen with (see `drawpad`).
    drawpad: Option<WindowHandle<drawpad::DrawPad>>,
    captures: Option<WindowHandle<captures::CapturesView>>,
    settings: Option<WindowHandle<settings::SettingsView>>,
    /// The user closed the faces window during this call; don't reopen it.
    faces_dismissed: bool,
    /// The user closed the viewer during this share; don't reopen it.
    viewer_dismissed: bool,
    /// The viewer (or peek) is showing faces in its bands, so the call window steps aside.
    docked_viewer: bool,
    docked_peek: bool,
    /// Whether those windows are on a workspace that's on screen right now. Assumed so
    /// until Hyprland says otherwise (and always, off Hyprland).
    visible: hypr::Visible,
    background: bool,
    only: Option<String>,
}

impl Global for Shell {}

impl Shell {
    fn allowed(cx: &App, kind: &str) -> bool {
        cx.global::<Shell>()
            .only
            .as_deref()
            .is_none_or(|only| only == kind || (kind == "settings" && only == "settings-capture"))
    }

    pub fn session(cx: &App) -> Entity<Session> {
        cx.global::<Shell>().session.clone()
    }

    pub fn show_home(cx: &mut App) {
        if !Self::allowed(cx, "home") {
            return;
        }
        let session = Self::session(cx);
        let existing = cx.global::<Shell>().home;
        if let Some(handle) = existing
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let handle = home::open(session, cx);
        cx.global_mut::<Shell>().home = handle;
    }

    pub fn show_settings(cx: &mut App) {
        if !Self::allowed(cx, "settings") {
            return;
        }
        if let Some(handle) = cx.global::<Shell>().settings
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let session = Self::session(cx);
        let handle = settings::open(session, cx);
        cx.global_mut::<Shell>().settings = handle;
    }

    /// Settings, open at one section.
    pub fn show_settings_at(section: settings::Section, cx: &mut App) {
        Self::show_settings(cx);
        if let Some(handle) = cx.global::<Shell>().settings {
            let _ = handle.update(cx, |view, _, cx| view.show(section, cx));
        }
    }

    /// The captures window on one call: an id, `last` or `live`.
    pub fn show_call(call: Option<&str>, cx: &mut App) {
        Self::show_captures(cx);
        let (Some(call), Some(handle)) = (call, cx.global::<Shell>().captures) else {
            return;
        };
        let _ = handle.update(cx, |view, _, cx| view.open_call(call, cx));
    }

    /// The live transcript and past captures.
    pub fn show_captures(cx: &mut App) {
        if !Self::allowed(cx, "captures") {
            return;
        }
        if let Some(handle) = cx.global::<Shell>().captures
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let session = Self::session(cx);
        let handle = captures::open(session, cx);
        cx.global_mut::<Shell>().captures = handle;
    }

    pub fn show_faces(cx: &mut App) {
        cx.global_mut::<Shell>().faces_dismissed = false;
        if let Some(handle) = cx.global::<Shell>().faces {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        Self::sync(cx);
    }

    pub fn show_viewer(cx: &mut App) {
        cx.global_mut::<Shell>().viewer_dismissed = false;
        if let Some(handle) = cx.global::<Shell>().viewer {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        Self::sync(cx);
    }

    /// Hyprland reports which viewer windows are on screen.
    fn set_visible(visible: hypr::Visible, cx: &mut App) {
        if cx.global::<Shell>().visible != visible {
            cx.global_mut::<Shell>().visible = visible;
            Self::sync(cx);
        }
    }

    /// A viewer started or stopped showing faces in its bands.
    pub fn set_docked(peek: bool, docked: bool, cx: &mut App) {
        let shell = cx.global_mut::<Shell>();
        let flag = if peek {
            &mut shell.docked_peek
        } else {
            &mut shell.docked_viewer
        };
        if *flag != docked {
            *flag = docked;
            Self::sync(cx);
        }
    }

    /// Super+H: the whole shared screen over whatever you're doing, or back again.
    pub fn toggle_peek(cx: &mut App) {
        if let Some(handle) = cx.global_mut::<Shell>().peek.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
            Self::set_docked(true, false, cx);
            return;
        }
        if !Self::allowed(cx, "peek") {
            return;
        }
        let session = Self::session(cx);
        if session.read(cx).remote_identity.is_none() {
            return;
        }
        let handle = viewer::open(session, true, cx);
        cx.global_mut::<Shell>().peek = handle;
    }

    /// Others' cursors, ink and stamps over the monitor you share (not a window share:
    /// there is no stable place to draw them). Open only while there's something to show:
    /// an open overlay costs GPU time on every frame (most of all while the share is
    /// taken through the Hyprland plugin), and cursors fade out after a few idle seconds.
    /// Checked on every screen change, so `retry` is false there: a monitor that couldn't
    /// be found is tried again only on a real change.
    fn sync_marks(retry: bool, cx: &mut App) {
        let session = Self::session(cx);
        let target = session
            .read(cx)
            .share_target
            .clone()
            .filter(|target| target.output_name.is_some());
        let sharing_monitor = session.read(cx).sharing && target.is_some();
        if !sharing_monitor {
            let shell = cx.global_mut::<Shell>();
            shell.clean_share = None;
            shell.marks_failed = false;
        }
        // Over your own share only when viewers won't see it: without the Hyprland plugin
        // the overlay would black out the whole share.
        let clean = sharing_monitor
            && *cx
                .global_mut::<Shell>()
                .clean_share
                .get_or_insert_with(hypr::clean_share);
        let marks_open = cx.global::<Shell>().marks.is_some();
        let marks_wanted = sharing_monitor && Self::allowed(cx, "marks");
        let showing = session.read(cx).has_marks();
        let failed = cx.global::<Shell>().marks_failed;
        match (marks_wanted && showing, target, marks_open) {
            (true, Some(_), false) if !clean => {
                if !failed {
                    cx.global_mut::<Shell>().marks_failed = true;
                    crate::logger::line(format!(
                        "Marks overlay: off, the {} Hyprland plugin isn't loaded",
                        hypr::SHARE_PLUGIN
                    ));
                }
            }
            (true, Some(target), false) if retry || !failed => {
                let handle = overlay::open(session.clone(), &target, cx);
                // Tried again on the next change (a monitor plugged back in); said once.
                if handle.is_none() && !failed {
                    cx.global_mut::<Shell>().marks_failed = true;
                    let monitor = target.output_name.clone().unwrap_or_default();
                    session.update(cx, |session, cx| {
                        session.push_error(format!(
                            "Couldn't show others' cursors and ink on {monitor}"
                        ));
                        cx.notify();
                    });
                }
                cx.global_mut::<Shell>().marks = handle;
            }
            (false, _, true) | (true, None, true) => {
                Self::close(cx, |shell| shell.marks.take().map(|handle| handle.into()));
            }
            _ => {}
        }
    }

    /// Bring the window set in line with the session.
    fn sync(cx: &mut App) {
        let session = Self::session(cx);
        let (in_call, remote, ringing, sharing) = {
            let state = session.read(cx);
            (
                state.in_call(),
                state.remote_identity.is_some(),
                state.ringing.is_some()
                    || (state.dialing.is_some() && !state.in_call())
                    || state.missed_shown().is_some(),
                state.sharing,
            )
        };
        let sharing = sharing && Self::allowed(cx, "sharing");
        let sharing_open = cx.global::<Shell>().sharing.is_some();
        if sharing && !sharing_open {
            // On the monitor you share, so the draw pad's hole lines up with it.
            let target = session.read(cx).share_target.clone();
            let handle = sharing::open(session.clone(), target.as_ref(), cx);
            cx.global_mut::<Shell>().sharing = handle;
        } else if !sharing && sharing_open {
            Self::close(cx, |shell| shell.sharing.take().map(|handle| handle.into()));
        }
        Self::sync_marks(true, cx);
        let clean = cx.global::<Shell>().clean_share == Some(true);
        // Drawing on your own screen while you share it: the pad over the same monitor.
        let pad_target = session
            .read(cx)
            .share_target
            .clone()
            .filter(|target| target.output_name.is_some());
        let pad_wanted = {
            let state = session.read(cx);
            state.sharing && (state.drawing || state.stamping) && Self::allowed(cx, "drawpad")
        };
        let pad_open = cx.global::<Shell>().drawpad.is_some();
        match (pad_wanted, pad_target, pad_open) {
            (true, Some(_), false) if !clean => {
                // Hyprland would black out the whole share under the pad.
                session.update(cx, |session, cx| {
                    session.stop_marking(cx);
                    session.push_error(format!(
                        "Drawing on your own screen needs the {} Hyprland plugin (make install, then log out and back in)",
                        hypr::SHARE_PLUGIN
                    ));
                    cx.notify();
                });
            }
            (true, Some(target), false) => {
                let handle = drawpad::open(session.clone(), &target, cx);
                if handle.is_none() {
                    // Nowhere to draw: don't leave drawing on with no way to use it.
                    session.update(cx, |session, cx| session.stop_marking(cx));
                }
                cx.global_mut::<Shell>().drawpad = handle;
            }
            (false, _, true) | (true, None, true) => {
                Self::close(cx, |shell| shell.drawpad.take().map(|handle| handle.into()));
            }
            _ => {}
        }
        let shell = cx.global::<Shell>();
        let (want_faces, want_viewer) = (
            in_call
                && !shell.faces_dismissed
                // Faces are already in an on-screen viewer's bands; on another workspace
                // the floating call window comes back.
                && !(shell.docked_viewer && shell.visible.viewer)
                && !(shell.docked_peek && shell.visible.peek)
                && Self::allowed(cx, "faces"),
            remote && !shell.viewer_dismissed && Self::allowed(cx, "viewer"),
        );
        let ringing = ringing && Self::allowed(cx, "call");
        let (faces_open, viewer_open, peek_open, toast_open) = (
            shell.faces.is_some(),
            shell.viewer.is_some(),
            shell.peek.is_some(),
            shell.toast.is_some(),
        );

        if want_faces && !faces_open {
            let handle = faces::open(session.clone(), cx);
            cx.global_mut::<Shell>().faces = handle;
        } else if !want_faces && faces_open {
            Self::close(cx, |shell| shell.faces.take().map(|handle| handle.into()));
        }
        if want_viewer && !viewer_open {
            let handle = viewer::open(session.clone(), false, cx);
            cx.global_mut::<Shell>().viewer = handle;
        } else if !remote && viewer_open {
            Self::close(cx, |shell| {
                shell.docked_viewer = false;
                shell.viewer.take().map(|handle| handle.into())
            });
        }
        if !remote && peek_open {
            Self::close(cx, |shell| {
                shell.docked_peek = false;
                shell.peek.take().map(|handle| handle.into())
            });
        }
        if ringing && !toast_open {
            let handle = toast::open(session.clone(), cx);
            cx.global_mut::<Shell>().toast = handle;
        } else if !ringing && toast_open {
            Self::close(cx, |shell| shell.toast.take().map(|handle| handle.into()));
        }
        Self::sync_cards(cx, ringing);
    }

    /// The card stack: open while cards wait, sized to them, below the call card when
    /// that shows too (a layer surface can't move, so that reopens it).
    fn sync_cards(cx: &mut App, below_call: bool) {
        let session = Self::session(cx);
        let count = if Self::allowed(cx, "cards") {
            session.read(cx).showing_cards().len()
        } else {
            0
        };
        let current = cx
            .global::<Shell>()
            .cards
            .as_ref()
            .map(|(handle, sized, below)| (*handle, *sized, *below));
        match current {
            None if count > 0 => {
                let handle = cards::open(session, count, below_call, cx);
                cx.global_mut::<Shell>().cards = handle.map(|handle| (handle, count, below_call));
            }
            Some(_) if count == 0 => {
                Self::close(cx, |shell| {
                    shell.cards.take().map(|(handle, _, _)| handle.into())
                });
            }
            Some((_, _, below)) if below != below_call => {
                Self::close(cx, |shell| {
                    shell.cards.take().map(|(handle, _, _)| handle.into())
                });
                let handle = cards::open(session, count, below_call, cx);
                cx.global_mut::<Shell>().cards = handle.map(|handle| (handle, count, below_call));
            }
            Some((handle, sized, _))
                if sized.min(cards::SHOWN + 1) != count.min(cards::SHOWN + 1) =>
            {
                let resized = handle.update(cx, |_, window, _| {
                    window.resize(gpui_kit::size(
                        gpui_kit::px(cards::WIDTH),
                        gpui_kit::px(cards::height(count)),
                    ))
                });
                if resized.is_ok()
                    && let Some((_, sized, _)) = cx.global_mut::<Shell>().cards.as_mut()
                {
                    *sized = count;
                }
            }
            _ => {}
        }
    }

    fn close(cx: &mut App, take: impl FnOnce(&mut Shell) -> Option<gpui_kit::AnyWindowHandle>) {
        if let Some(handle) = take(cx.global_mut::<Shell>()) {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    fn handle(event: &SessionEvent, cx: &mut App) {
        match event {
            SessionEvent::Activate => Self::show_home(cx),
            SessionEvent::Request(request) => {
                use crate::protocol::UiRequest;
                match request {
                    UiRequest::Open => Self::show_home(cx),
                    UiRequest::Settings => Self::show_settings(cx),
                    UiRequest::Peek => Self::toggle_peek(cx),
                    UiRequest::ShowFaces => Self::show_faces(cx),
                    UiRequest::ShowViewer => Self::show_viewer(cx),
                    UiRequest::Captures(call) => Self::show_call(call.as_deref(), cx),
                    UiRequest::Quit => {
                        Self::session(cx).update(cx, |session, _| session.shutdown());
                        cx.quit();
                        return;
                    }
                    _ => {}
                }
            }
            SessionEvent::CallStarted => {
                cx.global_mut::<Shell>().faces_dismissed = false;
            }
            SessionEvent::RemoteShareStarted => {
                cx.global_mut::<Shell>().viewer_dismissed = false;
            }
            SessionEvent::SignedOut => {
                if !cx.global::<Shell>().background {
                    Self::show_home(cx);
                }
            }
            SessionEvent::CallEnded
            | SessionEvent::RemoteShareEnded
            | SessionEvent::RingingChanged
            | SessionEvent::SharingChanged
            | SessionEvent::CardsChanged
            | SessionEvent::DrawingChanged
            | SessionEvent::SignedIn => {}
            // Frames change no windows; the views showing them redraw on their own.
            SessionEvent::MarksChanged => return Self::sync_marks(false, cx),
            SessionEvent::Screen | SessionEvent::Cameras => return,
        }
        Self::sync(cx);
    }

    /// A window went away; if the user closed it, remember that. Hopp keeps running with
    /// no windows so calls still reach it, and exits only on Quit. Closing the toast after
    /// accepting used to look like "nothing left" because the call hadn't joined yet.
    fn window_closed(id: gpui_kit::WindowId, cx: &mut App) {
        let shell = cx.global_mut::<Shell>();
        let is = |handle: Option<gpui_kit::AnyWindowHandle>| {
            handle.is_some_and(|handle| handle.window_id() == id)
        };
        if is(shell.home.map(Into::into)) {
            shell.home = None;
        }
        if is(shell.faces.map(Into::into)) {
            shell.faces = None;
            shell.faces_dismissed = true;
        }
        let mut undocked = false;
        if is(shell.viewer.map(Into::into)) {
            shell.viewer = None;
            shell.viewer_dismissed = true;
            undocked |= std::mem::take(&mut shell.docked_viewer);
        }
        if is(shell.peek.map(Into::into)) {
            shell.peek = None;
            undocked |= std::mem::take(&mut shell.docked_peek);
        }
        if is(shell.toast.map(Into::into)) {
            shell.toast = None;
        }
        if is(shell.cards.as_ref().map(|(handle, _, _)| (*handle).into())) {
            shell.cards = None;
        }
        if is(shell.drawpad.map(Into::into)) {
            shell.drawpad = None;
        }
        if is(shell.sharing.map(Into::into)) {
            shell.sharing = None;
        }
        if is(shell.captures.map(Into::into)) {
            shell.captures = None;
        }
        if is(shell.settings.map(Into::into)) {
            shell.settings = None;
        }
        if undocked {
            // Faces were in the closed viewer's bands; bring the call window back.
            Self::sync(cx);
        }
    }
}

/// Call shortcuts that work in every Hopp window.
pub fn global_shortcut(
    keystroke: &Keystroke,
    held: bool,
    session: &Entity<Session>,
    cx: &mut App,
) -> bool {
    let modifiers = keystroke.modifiers;
    if !(modifiers.control && modifiers.shift && !modifiers.alt && !modifiers.platform) || held {
        return false;
    }
    match keystroke.key.as_str() {
        "m" => session.update(cx, |session, cx| session.toggle_microphone(cx)),
        "s" => session.update(cx, |session, cx| session.toggle_share(false, cx)),
        "l" => session.update(cx, |session, cx| session.leave(cx)),
        "c" => session.update(cx, |session, cx| session.toggle_capture(cx)),
        _ => return false,
    }
    true
}

pub fn open(engine: Engine, settings: Settings, options: Options) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            // GPUI quits on Linux when the last window closes, which ended calls whenever
            // the viewer or toast was the only window. Hopp runs windowless in the
            // background and exits only on Quit. (Screenshot fixtures still end with their
            // window.)
            if options.only.is_none() {
                cx.set_quit_mode(gpui_kit::QuitMode::Explicit);
            }
            gpui_omarchy::init(cx);
            let session = cx.new(|_| Session::new(engine, settings));
            cx.set_global(Shell {
                session: session.clone(),
                home: None,
                faces: None,
                viewer: None,
                peek: None,
                toast: None,
                cards: None,
                drawpad: None,
                sharing: None,
                marks: None,
                marks_failed: false,
                clean_share: None,
                captures: None,
                settings: None,
                faces_dismissed: false,
                viewer_dismissed: false,
                docked_viewer: false,
                docked_peek: false,
                visible: hypr::Visible {
                    viewer: true,
                    peek: true,
                },
                background: options.background,
                only: options.only,
            });
            if cx.global::<Shell>().only.as_deref() == Some("peek") {
                // The fixture shows peek as soon as a share arrives.
                cx.subscribe(&session, |_, event, cx| {
                    if *event == SessionEvent::RemoteShareStarted {
                        Shell::toggle_peek(cx);
                    }
                })
                .detach();
            }
            cx.subscribe(&session, |_, event, cx| Shell::handle(event, cx))
                .detach();
            cx.on_window_closed(|cx, id| Shell::window_closed(id, cx))
                .detach();
            if let Some(publish) = options.publish {
                let mut last = String::new();
                cx.observe(&session, move |session, cx| {
                    let snapshot = session.read(cx).snapshot().to_string();
                    if snapshot != last {
                        publish(&snapshot);
                        last = snapshot;
                    }
                })
                .detach();
            }

            // The engine pump runs at the app level, not inside any window. It sleeps until
            // the engine (or Hyprland) has something new, with a slow tick for expiring
            // pointers and theme changes.
            let pump = session.clone();
            let wake = session.read(cx).wake();
            let visibility = hypr::watch_visibility(&wake);
            cx.spawn(async move |cx| {
                loop {
                    let housekeeping = cx.background_executor().timer(Duration::from_secs(1));
                    futures_util::future::select(
                        std::pin::pin!(wake.notified()),
                        std::pin::pin!(housekeeping),
                    )
                    .await;
                    cx.update(|cx| {
                        pump.update(cx, |session, cx| session.tick(cx));
                        let latest = visibility
                            .as_ref()
                            .and_then(|receiver| receiver.try_iter().last());
                        if let Some(visible) = latest {
                            Shell::set_visible(visible, cx);
                        }
                    });
                }
            })
            .detach();

            match cx.global::<Shell>().only.as_deref() {
                Some("settings") => Shell::show_settings(cx),
                Some("settings-capture") => Shell::show_settings_at(settings::Section::Capture, cx),
                Some("captures") => Shell::show_captures(cx),
                _ if !options.background => Shell::show_home(cx),
                _ => {}
            }
            if options.smoke_test {
                cx.spawn(async |cx| {
                    cx.background_executor().timer(Duration::from_secs(3)).await;
                    cx.update(|cx| cx.quit());
                })
                .detach();
            }
            cx.activate(true);
        });
}
