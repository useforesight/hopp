//! Settings: a window of its own, sections down the side and one setting per row,
//! title and explanation on the left, the control on the right.
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyElement, AppContext, Bounds, ClipboardItem, Context, Entity, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    WindowHandle,
    base::input::{InputEvent, InputState},
    div,
    prelude::FluentBuilder,
    px, size,
};
use gpui_omarchy::{ActiveTheme, ChoiceItem, ChoiceState, IconName, combobox, icon, input};

use super::{
    Shell,
    session::{Devices, Session},
    style::{self, Tone},
};
use crate::{
    capture::{config, models},
    settings::{FaceLayout, FaceSize},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Account,
    Devices,
    CallWindow,
    Capture,
    Shortcuts,
}

impl Section {
    const ALL: [Section; 5] = [
        Section::Account,
        Section::Devices,
        Section::CallWindow,
        Section::Capture,
        Section::Shortcuts,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::Account => "Account",
            Section::Devices => "Audio and video",
            Section::CallWindow => "Call window",
            Section::Capture => "Capture",
            Section::Shortcuts => "Shortcuts",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Section::Account => IconName::CircleUser,
            Section::Devices => IconName::Headphones,
            Section::CallWindow => IconName::LayoutGrid,
            Section::Capture => IconName::AudioLines,
            Section::Shortcuts => IconName::Keyboard,
        }
    }
}

struct DeviceChoices {
    from: Devices,
    input: Entity<ChoiceState>,
    output: Entity<ChoiceState>,
    camera: Entity<ChoiceState>,
}

pub struct SettingsView {
    session: Entity<Session>,
    section: Section,
    server: Entity<InputState>,
    devices: Option<DeviceChoices>,
    /// Which copy button was pressed, to say "Copied" for a moment.
    copied: Option<(&'static str, Instant)>,
    /// The model whose Delete was pressed once; a second press deletes it.
    confirm_delete: Option<&'static str>,
}

const MCP_CLAUDE: &str = "claude mcp add hopp -- hopp mcp";
const MCP_CODEX: &str = "codex mcp add hopp -- hopp mcp";

impl SettingsView {
    fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let current = session.read(cx).settings.server.clone();
        let server = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(current)
                .placeholder(crate::api::DEFAULT_API_BASE)
        });
        cx.subscribe(&server, |view, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let value = state.read(cx).value().trim().to_owned();
                view.session.update(cx, |session, cx| {
                    if !value.is_empty() && session.settings.server != value {
                        session.settings.server = value;
                        let _ = crate::settings::save(&session.settings);
                        cx.notify();
                    }
                });
            }
        })
        .detach();
        // Downloads and "Copied" labels change without the session noticing.
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let alive = view.update(cx, |view, cx| {
                    let copied = view
                        .copied
                        .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(2));
                    if copied {
                        view.copied = None;
                    }
                    if copied || models::any_downloading() {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            session,
            section: Section::Account,
            server,
            devices: None,
            copied: None,
            confirm_delete: None,
        }
    }

    pub fn show(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        cx.notify();
    }

    // ---- devices ----------------------------------------------------------------------

    fn sync_devices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(devices) = self.session.read(cx).devices.clone() else {
            return;
        };
        if self.devices.as_ref().is_some_and(|built| {
            built.from.inputs == devices.inputs
                && built.from.outputs == devices.outputs
                && built.from.cameras == devices.cameras
        }) {
            return;
        }
        let settings = self.session.read(cx).settings.clone();
        let named = |list: &[String]| {
            list.iter()
                .map(|name| (name.clone(), name.clone()))
                .collect::<Vec<_>>()
        };
        let input = self.choice(
            named(&devices.inputs),
            settings.input_device.as_deref(),
            window,
            cx,
        );
        let output = self.choice(
            named(&devices.outputs),
            settings.output_device.as_deref(),
            window,
            cx,
        );
        let camera = self.choice(
            devices.cameras.clone(),
            settings.camera_device.as_deref(),
            window,
            cx,
        );
        self.devices = Some(DeviceChoices {
            from: devices,
            input,
            output,
            camera,
        });
    }

    fn choice(
        &mut self,
        devices: Vec<(String, String)>,
        selected: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ChoiceState> {
        let mut items = vec![ChoiceItem::new("", "System default")];
        items.extend(
            devices
                .into_iter()
                .map(|(id, label)| ChoiceItem::new(id, label)),
        );
        let index = selected
            .and_then(|selected| {
                items
                    .iter()
                    .position(|item| item.value.as_ref() == selected)
            })
            .unwrap_or(0);
        let state = cx.new(|cx| ChoiceState::new(items, window, cx).default_selected(index));
        cx.observe(&state, |view, _, cx| view.apply_devices(cx))
            .detach();
        state
    }

    fn apply_devices(&mut self, cx: &mut Context<Self>) {
        let Some(choices) = &self.devices else {
            return;
        };
        let pick = |state: &Entity<ChoiceState>| {
            state
                .read(cx)
                .selected()
                .map(|item| item.value.to_string())
                .filter(|value| !value.is_empty())
        };
        let (input, output, camera) = (
            pick(&choices.input),
            pick(&choices.output),
            pick(&choices.camera),
        );
        self.session.update(cx, |session, _| {
            session.select_audio(input, output);
            session.select_camera(camera);
            let _ = crate::settings::save(&session.settings);
        });
    }

    // ---- building blocks --------------------------------------------------------------

    /// One setting: what it is and why on the left, the control on the right.
    fn row(
        title: impl Into<SharedString>,
        description: Option<impl Into<SharedString>>,
        control: impl IntoElement,
        cx: &gpui_kit::App,
    ) -> gpui_kit::Div {
        let theme = cx.omarchy();
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .py(px(11.))
            .border_b_1()
            .border_color(theme.border.opacity(0.45))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_color(theme.bright).child(title.into()))
                    .children(description.map(|text| style::muted_text(text, cx))),
            )
            .child(div().flex_shrink_0().child(control))
    }

    /// Choices side by side in a tray, the chosen one raised.
    fn segmented<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: &[(T, &'static str)],
        chosen: T,
        apply: fn(&mut Self, T, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let theme = cx.omarchy().clone();
        let mut tray = div()
            .flex()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(8.))
            .bg(theme.surface);
        for (value, label) in options.iter().copied() {
            let on = value == chosen;
            tray = tray.child(
                div()
                    .id(SharedString::from(format!("{id}-{label}")))
                    .px(px(10.))
                    .py(px(3.))
                    .rounded(px(6.))
                    .text_size(px(11.5))
                    .cursor_pointer()
                    .when(on, |option| {
                        option
                            .bg(theme.background)
                            .text_color(theme.bright)
                            .shadow_sm()
                    })
                    .when(!on, |option| {
                        option
                            .text_color(theme.foreground)
                            .hover(|option| option.text_color(theme.bright))
                    })
                    .child(label)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        apply(view, value, cx);
                        cx.notify();
                    })),
            );
        }
        tray
    }

    fn copy_chip(
        &self,
        id: &'static str,
        label: &'static str,
        text: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let copied = self.copied.is_some_and(|(which, _)| which == id);
        style::chip(
            id,
            if copied { "Copied" } else { label },
            Some(if copied {
                IconName::Check
            } else {
                IconName::Copy
            }),
            Tone::Plain,
            cx,
        )
        .on_click(cx.listener(move |view, _, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
            view.copied = Some((id, Instant::now()));
            cx.notify();
        }))
    }

    fn open_folder(directory: Option<std::path::PathBuf>) {
        if let Some(directory) = directory
            && crate::paths::private_dir(&directory).is_ok()
        {
            let _ = std::process::Command::new("xdg-open")
                .arg(directory)
                .spawn();
        }
    }

    // ---- sections ---------------------------------------------------------------------

    fn account(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.omarchy().clone();
        let session = self.session.read(cx);
        let user = session.user.clone();
        let (own_status, auto_away) = (session.own_status(), session.settings.auto_away);
        let mut page = div().flex().flex_col();
        page = page.child(Self::row(
            match &user {
                Some(user) => crate::ui::session::full_name(&user.first_name, &user.last_name),
                None => "Not signed in".into(),
            },
            user.as_ref().map(|user| user.email.clone()),
            div()
                .flex()
                .gap(px(6.))
                .when(user.is_some(), |row| {
                    row.child(
                        style::chip(
                            "sign-out",
                            "Sign out",
                            Some(IconName::LogOut),
                            Tone::Plain,
                            cx,
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.session.update(cx, |session, cx| session.sign_out(cx))
                        })),
                    )
                })
                .when(user.is_none(), |row| {
                    row.child(
                        style::chip(
                            "sign-in",
                            "Sign in",
                            Some(IconName::ArrowRight),
                            Tone::On,
                            cx,
                        )
                        .on_click(|_, _, cx| Shell::show_home(cx)),
                    )
                }),
            cx,
        ));
        // Servers without statuses leave them out, and then so does this.
        if let Some(current) = own_status {
            use crate::api::UserStatus;
            let picker = self.segmented(
                "status",
                &UserStatus::ALL.map(|status| (status, status.label())),
                current,
                |view, status, cx| {
                    view.session
                        .update(cx, |session, cx| session.set_status(status, cx))
                },
                cx,
            );
            page = page.child(Self::row(
                "Status",
                Some("What teammates see before they ring. Do not disturb turns calls away and lists them as missed."),
                picker,
                cx,
            ));
            let away = self.segmented(
                "auto-away",
                &[(false, "Off"), (true, "On")],
                auto_away,
                |view, on, cx| {
                    view.session
                        .update(cx, |session, cx| session.set_auto_away(on, cx))
                },
                cx,
            );
            page = page.child(Self::row(
                "Away while locked",
                Some("Shows you as Away while the screen is locked, except during a call. A status you chose stays."),
                away,
                cx,
            ));
        }
        page = page.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .py(px(11.))
                .border_b_1()
                .border_color(theme.border.opacity(0.45))
                .child(div().text_color(theme.bright).child("Hopp server"))
                .child(style::muted_text(
                    "Managed Hopp, or your own. Takes effect the next time you sign in.",
                    cx,
                ))
                .child(input("server", &self.server, window, cx)),
        );
        page = page.child(Self::row(
            "Quit Hopp",
            Some("Leaves any call. The bar starts Hopp again when you next open it."),
            style::chip("quit", "Quit", Some(IconName::Power), Tone::Danger, cx).on_click(
                cx.listener(|view, _, _, cx| {
                    view.session.update(cx, |session, cx| session.quit(cx))
                }),
            ),
            cx,
        ));
        page.child(
            style::muted_text(
                format!(
                    "Hopp {} · Omarchy theme {}",
                    env!("CARGO_PKG_VERSION"),
                    theme.name
                ),
                cx,
            )
            .pt(px(12.)),
        )
        .into_any_element()
    }

    fn devices(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.sync_devices(window, cx);
        let mut page = div().flex().flex_col();
        for (title, description, choice) in [
            (
                "Microphone",
                "Echo cancellation and noise suppression are always on.",
                self.devices.as_ref().map(|choices| choices.input.clone()),
            ),
            (
                "Speakers",
                "Where everyone else's voices play.",
                self.devices.as_ref().map(|choices| choices.output.clone()),
            ),
            (
                "Camera",
                "Shown square in the call window.",
                self.devices.as_ref().map(|choices| choices.camera.clone()),
            ),
        ] {
            let control = match choice {
                Some(choice) => div()
                    .w(px(260.))
                    .child(combobox(title, &choice, window, cx))
                    .into_any_element(),
                None => style::muted_text("Looking for devices…", cx).into_any_element(),
            };
            page = page.child(Self::row(title, Some(description), control, cx));
        }
        let settings = self.session.read(cx).settings.clone();
        let microphone = self.segmented(
            "start-microphone",
            &[(false, "Muted"), (true, "On")],
            settings.start_microphone,
            |view, on, cx| {
                view.session.update(cx, |session, cx| {
                    session.settings.start_microphone = on;
                    let _ = crate::settings::save(&session.settings);
                    cx.notify();
                })
            },
            cx,
        );
        let camera = self.segmented(
            "start-camera",
            &[(false, "Off"), (true, "On")],
            settings.start_camera,
            |view, on, cx| {
                view.session.update(cx, |session, cx| {
                    session.settings.start_camera = on;
                    let _ = crate::settings::save(&session.settings);
                    cx.notify();
                })
            },
            cx,
        );
        page.child(Self::row(
            "Microphone when a call starts",
            Some("Muted, or on as soon as you're connected."),
            microphone,
            cx,
        ))
        .child(Self::row(
            "Camera when a call starts",
            None::<&str>,
            camera,
            cx,
        ))
        .into_any_element()
    }

    fn call_window(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.session.read(cx).settings.clone();
        let size = self.segmented(
            "face-size",
            &[
                (FaceSize::Small, "Small"),
                (FaceSize::Medium, "Medium"),
                (FaceSize::Large, "Large"),
            ],
            settings.face_size,
            |view, size, cx| {
                view.session.update(cx, |session, cx| {
                    session.settings.face_size = size;
                    let _ = crate::settings::save(&session.settings);
                    cx.notify();
                })
            },
            cx,
        );
        let layout = self.segmented(
            "face-layout",
            &[
                (FaceLayout::Column, "Column"),
                (FaceLayout::Row, "Row"),
                (FaceLayout::Bubbles, "Bubbles"),
            ],
            settings.face_layout,
            |view, layout, cx| {
                view.session.update(cx, |session, cx| {
                    session.settings.face_layout = layout;
                    let _ = crate::settings::save(&session.settings);
                    cx.notify();
                })
            },
            cx,
        );
        div()
            .flex()
            .flex_col()
            .child(Self::row(
                "Size",
                Some("Tiles are always square and never stretch."),
                size,
                cx,
            ))
            .child(Self::row(
                "Layout",
                Some("Hover the call window to change these there too."),
                layout,
                cx,
            ))
            .into_any_element()
    }

    fn size(bytes: u64) -> String {
        if bytes >= 1_000_000_000 {
            format!("{:.1} GB", bytes as f64 / 1e9)
        } else {
            format!("{} MB", bytes / 1_000_000)
        }
    }

    fn model_list(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let theme = cx.omarchy().clone();
        let chosen = config::load().model;
        let mut list = div()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border.opacity(0.7))
            .overflow_hidden();
        for (index, model) in models::MODELS.iter().enumerate() {
            let in_use = model.name == chosen;
            let installed = models::installed(model);
            let state = models::download_state(model.name);
            let status: AnyElement = match &state {
                Some(Ok(percent)) => {
                    style::muted_text(format!("Downloading {percent}%"), cx).into_any_element()
                }
                Some(Err(_)) => div()
                    .text_size(px(11.))
                    .text_color(theme.danger)
                    .child("Download failed")
                    .into_any_element(),
                None if installed => {
                    let shared = models::shared(model);
                    let downloaded = div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_size(px(11.))
                        .text_color(theme.success)
                        .child(icon(IconName::Check).size(px(12.)))
                        .child(if shared { "Hex's copy" } else { "Downloaded" });
                    // The model in use can't be deleted: choose another first. Hex's
                    // copy is Hex's to delete.
                    if in_use || shared {
                        downloaded.into_any_element()
                    } else {
                        let confirming = self.confirm_delete == Some(model.name);
                        let size = Self::size(model.bytes);
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(downloaded)
                            .child(
                                style::chip(
                                    SharedString::from(format!("delete-{}", model.name)),
                                    if confirming {
                                        format!("Delete {size}?")
                                    } else {
                                        "Delete".into()
                                    },
                                    Some(IconName::Trash),
                                    if confirming {
                                        Tone::Danger
                                    } else {
                                        Tone::Plain
                                    },
                                    cx,
                                )
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        cx.stop_propagation();
                                        if view.confirm_delete == Some(model.name) {
                                            view.confirm_delete = None;
                                            if let Err(error) = models::delete(model) {
                                                crate::logger::line(format!("{error:#}"));
                                            }
                                        } else {
                                            view.confirm_delete = Some(model.name);
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                            .into_any_element()
                    }
                }
                None => style::chip(
                    SharedString::from(format!("download-{}", model.name)),
                    "Download",
                    Some(IconName::Download),
                    Tone::Plain,
                    cx,
                )
                .on_click(move |_, _, cx| {
                    // Downloading doesn't choose the model; clicking the row does.
                    cx.stop_propagation();
                    models::download_in_background(model);
                })
                .into_any_element(),
            };
            list = list.child(
                div()
                    .id(SharedString::from(format!("model-{}", model.name)))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(12.))
                    .py(px(8.))
                    .cursor_pointer()
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(theme.border.opacity(0.5))
                    })
                    .when(in_use, |row| row.bg(theme.selection.opacity(0.6)))
                    .hover(|row| row.bg(theme.surface))
                    .child(
                        // A radio: ringed when chosen.
                        div()
                            .size(px(14.))
                            .flex_shrink_0()
                            .rounded_full()
                            .border_color(if in_use { theme.accent } else { theme.border })
                            .map(|radio| {
                                if in_use {
                                    radio.border_4()
                                } else {
                                    radio.border_1()
                                }
                            }),
                    )
                    .child(
                        div()
                            .w(px(120.))
                            .flex_shrink_0()
                            .text_color(theme.bright)
                            .font_weight(if in_use {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .child(model.name),
                    )
                    .child(
                        div()
                            .w(px(64.))
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(theme.secondary.opacity(0.7))
                            .child(Self::size(model.bytes)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(style::muted_text(model.note, cx)),
                    )
                    .child(div().flex_shrink_0().child(status))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.confirm_delete = None;
                        let mut settings = config::load();
                        settings.model = model.name.into();
                        if let Err(error) = config::save(&settings) {
                            crate::logger::line(format!("Saving capture settings: {error:#}"));
                        }
                        if !models::installed(model) {
                            models::download_in_background(model);
                        }
                        cx.notify();
                    })),
            );
        }
        list
    }

    fn capture(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.omarchy().clone();
        let keep = config::load().keep_days;
        let (captures, downloaded) = crate::capture::disk_usage();
        let megabytes = |bytes: u64| {
            if bytes >= 1_000_000_000 {
                format!("{:.1} GB", bytes as f64 / 1e9)
            } else {
                format!("{:.0} MB", bytes as f64 / 1e6)
            }
        };
        let failure =
            models::MODELS
                .iter()
                .find_map(|model| match models::download_state(model.name) {
                    Some(Err(error)) => Some(format!("{}: {error}", model.name)),
                    _ => None,
                });
        let device = self.session.read(cx).capture_device.clone();
        let processor = self.segmented(
            "processor",
            &[(false, "CPU"), (true, "GPU")],
            config::load().gpu,
            |_, gpu, _| {
                let mut settings = config::load();
                settings.gpu = gpu;
                if let Err(error) = config::save(&settings) {
                    crate::logger::line(format!("Saving capture settings: {error:#}"));
                }
            },
            cx,
        );
        let every = self.segmented(
            "auto",
            &[(false, "Off"), (true, "On")],
            config::load().auto,
            |_, auto, _| {
                let mut settings = config::load();
                settings.auto = auto;
                if let Err(error) = config::save(&settings) {
                    crate::logger::line(format!("Saving capture settings: {error:#}"));
                }
            },
            cx,
        );
        let live = self.segmented(
            "drafts",
            &[(false, "Off"), (true, "On")],
            config::load().drafts,
            |_, drafts, _| {
                let mut settings = config::load();
                settings.drafts = drafts;
                if let Err(error) = config::save(&settings) {
                    crate::logger::line(format!("Saving capture settings: {error:#}"));
                }
            },
            cx,
        );
        let retention = self.segmented(
            "keep",
            &[
                (None, "Never"),
                (Some(90), "90 days"),
                (Some(30), "30 days"),
            ],
            keep,
            |_, keep_days, _| {
                let mut settings = config::load();
                settings.keep_days = keep_days;
                if config::save(&settings).is_ok() {
                    // Apply it now rather than at the next capture.
                    let _ = crate::capture::prune();
                }
            },
            cx,
        );
        div()
            .flex()
            .flex_col()
            .child(
                style::muted_text(
                    "Capture transcribes a call on this computer and keeps it here; nothing is \
                 uploaded. Start one with the capture button in a call or Super+Alt+C.",
                    cx,
                )
                .pb(px(8.)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .py(px(11.))
                    .border_b_1()
                    .border_color(theme.border.opacity(0.45))
                    .child(div().text_color(theme.bright).child("Transcription model"))
                    .child(style::muted_text(
                        "Parakeet English is the most accurate for English; the others cover \
                         more languages. A model only uses memory while a capture is \
                         transcribing, and is unloaded after 90 quiet seconds.",
                        cx,
                    ))
                    .child(self.model_list(cx))
                    .children(failure.map(|failure| {
                        div()
                            .text_size(px(11.))
                            .text_color(theme.danger)
                            .child(format!("Download failed: {failure}"))
                    })),
            )
            .child(Self::row(
                "Runs on",
                Some(format!(
                    "The GPU is faster and uses less energy. On Apple silicon it needs a \
                     kernel that can preempt GPU compute work (Aurora 2026.10.11.14 or later); \
                     with an older one the cursor and typing lag while it transcribes, so choose \
                     the CPU, which only uses time nothing else needs. Applies from the next \
                     capture.{}",
                    match device.as_deref() {
                        Some("CPU") => " Last capture: the CPU.".to_owned(),
                        Some(gpu) => format!(" Last capture: {gpu}."),
                        None => String::new(),
                    }
                )),
                processor,
                cx,
            ))
            .child(Self::row(
                "Capture every call",
                Some(
                    "Start capturing as soon as a call connects. Off: only when you start it."
                        .to_owned(),
                ),
                every,
                cx,
            ))
            .child(Self::row(
                "Live text",
                Some(
                    "Show what people say while they are still saying it, in the Captures \
                     window and to agents that ask for drafts. Words appear about two seconds \
                     after they are said; the final text is the same either way. Needs a \
                     Parakeet model on the GPU, and uses about a quarter of it while someone \
                     talks. Applies from the next capture."
                        .to_owned(),
                ),
                live,
                cx,
            ))
            .child(Self::row(
                "Storage",
                Some(format!(
                    "Captures {} · models {}",
                    megabytes(captures),
                    megabytes(downloaded)
                )),
                div()
                    .flex()
                    .gap(px(6.))
                    .child(
                        style::chip(
                            "show-captures",
                            "Captures",
                            Some(IconName::Captions),
                            Tone::Plain,
                            cx,
                        )
                        .on_click(|_, _, cx| Shell::show_captures(cx)),
                    )
                    .child(
                        style::chip(
                            "capture-folder",
                            "Folder",
                            Some(IconName::FolderOpen),
                            Tone::Plain,
                            cx,
                        )
                        .on_click(|_, _, _| {
                            Self::open_folder(
                                crate::paths::data_dir()
                                    .ok()
                                    .map(|data| data.join("capture")),
                            )
                        }),
                    ),
                cx,
            ))
            .child(Self::row(
                "Delete captures after",
                Some("Transcripts and their Markdown copies."),
                retention,
                cx,
            ))
            .child(Self::row(
                "Coding agents",
                Some("Lets agents read, search and follow captures (MCP). Copy the setup command."),
                div()
                    .flex()
                    .gap(px(6.))
                    .child(self.copy_chip("mcp-claude", "Claude Code", MCP_CLAUDE, cx))
                    .child(self.copy_chip("mcp-codex", "Codex", MCP_CODEX, cx)),
                cx,
            ))
            .child(Self::row(
                "Hooks",
                Some("Your programs, run when a capture starts or finishes."),
                style::chip(
                    "hooks-folder",
                    "Folder",
                    Some(IconName::Terminal),
                    Tone::Plain,
                    cx,
                )
                .on_click(|_, _, _| {
                    Self::open_folder(
                        crate::paths::config_dir()
                            .ok()
                            .map(|config| config.join("hooks")),
                    )
                }),
                cx,
            ))
            .into_any_element()
    }

    fn shortcuts(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.omarchy().clone();
        let mut page = div().flex().flex_col();
        for (keys, action) in [
            ("Super Alt H", "Open Hopp from anywhere"),
            ("Super H", "Peek at the whole shared screen"),
            ("Super Alt M", "Mute or unmute, from anywhere"),
            ("Super Alt V", "Camera on or off, from anywhere"),
            ("Super Alt X", "Share your screen, or stop, from anywhere"),
            ("Super Alt L", "Leave the call, from anywhere"),
            ("Super Alt C", "Capture the call, or stop"),
            ("Super Alt D", "Draw on the shared screen, or stop"),
            ("Super Alt P", "Stamp a note on the shared screen, or stop"),
            ("Super Ctrl Esc", "Take back remote control, from anywhere"),
            ("Ctrl Shift M", "Mute or unmute, in Hopp windows"),
            ("Ctrl Shift S", "Share or stop sharing"),
            ("Ctrl Shift C", "Capture or stop"),
            ("Ctrl Shift L", "Leave the call"),
            ("Esc", "Release control of a teammate's screen"),
        ] {
            page = page.child(Self::row(
                action,
                None::<&str>,
                div()
                    .px(px(7.))
                    .py(px(2.))
                    .rounded(px(5.))
                    .bg(theme.surface)
                    .font_family(theme.mono_font.clone())
                    .text_size(px(11.))
                    .text_color(theme.bright)
                    .child(keys),
                cx,
            ));
        }
        page.child(
            style::muted_text(
                "The Super shortcuts are Hyprland bindings in ~/.config/hypr/hopp.lua; rebind \
                 them in your own Hyprland config.",
                cx,
            )
            .pt(px(10.)),
        )
        .into_any_element()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let mut nav = div()
            .w(px(184.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(10.))
            .bg(theme.inset)
            .border_r_1()
            .border_color(theme.border.opacity(0.6))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .pt(px(6.))
                    .pb(px(12.))
                    .child(style::mark(18., cx))
                    .child(
                        div()
                            .text_color(theme.bright)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Settings"),
                    ),
            );
        for section in Section::ALL {
            let on = section == self.section;
            nav = nav.child(
                div()
                    .id(section.label())
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .px(px(10.))
                    .py(px(7.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .when(on, |item| item.bg(theme.selection).text_color(theme.bright))
                    .when(!on, |item| {
                        item.text_color(theme.foreground)
                            .hover(|item| item.bg(theme.surface))
                    })
                    .child(icon(section.icon()).size(px(15.)).text_color(if on {
                        theme.accent
                    } else {
                        theme.secondary
                    }))
                    .child(section.label())
                    .on_click(cx.listener(move |view, _, _, cx| view.show(section, cx))),
            );
        }
        let content = match self.section {
            Section::Account => self.account(window, cx),
            Section::Devices => self.devices(window, cx),
            Section::CallWindow => self.call_window(cx),
            Section::Capture => self.capture(cx),
            Section::Shortcuts => self.shortcuts(cx),
        };
        let session = self.session.clone();
        div()
            .size_full()
            .flex()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font.clone())
            .text_size(px(12.5))
            .on_key_down(
                cx.listener(move |_, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        window.remove_window();
                        cx.stop_propagation();
                    } else if super::global_shortcut(&event.keystroke, event.is_held, &session, cx)
                    {
                        cx.stop_propagation();
                    }
                }),
            )
            .child(nav)
            .child(
                div()
                    .id("settings-content")
                    .flex_1()
                    .min_w_0()
                    .overflow_y_scroll()
                    .px(px(24.))
                    .py(px(18.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.bright)
                            .pb(px(6.))
                            .child(self.section.label()),
                    )
                    .child(content),
            )
    }
}

pub fn open(
    session: Entity<Session>,
    cx: &mut gpui_kit::App,
) -> Option<WindowHandle<SettingsView>> {
    let options = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(860.), px(620.)),
            cx,
        ))),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("Hopp settings".into()),
            ..Default::default()
        }),
        app_id: Some("hopp.settings".into()),
        window_min_size: Some(size(px(620.), px(420.))),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        cx.new(|cx| SettingsView::new(session, window, cx))
    })
    .ok()
}
