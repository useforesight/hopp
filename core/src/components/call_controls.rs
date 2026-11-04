use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use iced::widget::{container, mouse_area, row, text, tooltip};
use iced::{Background, Border, Color, Element, Padding, Theme};
use socket_lib::{AppAction, AudioCaptureMessage, CameraStartMessage};
use winit::event_loop::EventLoopProxy;

use crate::audio::capturer::list_audio_inputs;
use crate::camera::capturer::CameraCapturer;
use crate::components::split_button::{
    split_button_dropdown_wrap, split_button_sized, svg_button_sized, SplitButtonItem,
    SplitButtonSize,
};
use crate::livekit::call_capture;
use crate::livekit::participant::ParticipantInfo;
use crate::windows::colors::ColorToken;
use crate::windows::shadows::ShadowToken;
use crate::UserEvent;

const ICON_MICROPHONE_ON: char = '\u{F105}';
const ICON_MICROPHONE_OFF: char = '\u{F106}';
const ICON_SCREEN_SHARE: char = '\u{F102}';
const ICON_VIDEO: char = '\u{F101}';
const ICON_PHONE_OFF: char = '\u{F103}';
pub const ICON_CAPTURE: &[u8] = include_bytes!("../../resources/icons/audio-lines.svg");

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CallControlsDensity {
    Regular,
    Compact,
    /// Without the device menus, for a window too narrow to have them.
    Icons,
    /// Only the mic, the camera and ending the call, for the narrowest windows.
    Essentials,
}

impl CallControlsDensity {
    const fn button_size(self) -> SplitButtonSize {
        match self {
            Self::Regular => SplitButtonSize::regular(),
            Self::Compact | Self::Icons | Self::Essentials => SplitButtonSize::compact(),
        }
    }

    const fn spacing(self) -> f32 {
        match self {
            Self::Regular => 8.0,
            Self::Compact | Self::Icons | Self::Essentials => 4.0,
        }
    }

    /// The capture control and the space before it, where captures work.
    const fn capture_width(self) -> f32 {
        if !call_capture::SUPPORTED {
            return 0.0;
        }
        match self {
            Self::Regular => 44.0,
            Self::Compact | Self::Icons => 34.0,
            Self::Essentials => 0.0,
        }
    }

    const fn has_device_menus(self) -> bool {
        matches!(self, Self::Regular | Self::Compact)
    }

    pub const fn total_width(self) -> f32 {
        self.capture_width()
            + match self {
                Self::Regular => 237.0,
                Self::Compact => 189.0,
                Self::Icons => 132.0,
                Self::Essentials => 98.0,
            }
    }

    const fn camera_dropdown_tail(self) -> f32 {
        self.capture_width()
            + match self {
                Self::Regular => 111.0,
                _ => 87.0,
            }
    }

    const fn mic_dropdown_tail(self) -> f32 {
        self.capture_width()
            + match self {
                Self::Regular => 178.0,
                _ => 140.0,
            }
    }
}

#[derive(Debug, Clone)]
pub enum CallControlsMessage {
    MicToggle,
    MicDropdownToggle,
    MicDropdownDismiss,
    SelectMic(String),
    VideoToggle,
    CameraDropdownToggle,
    CameraDropdownDismiss,
    SelectCamera(String),
    ScreenShare,
    OpenScreenSharePicker,
    ToggleCapture,
    OpenCaptures,
    EndCall,
}

#[derive(Default)]
pub struct CallControlsState {
    camera_active: bool,
    camera_dropdown_open: bool,
    available_cameras: Vec<socket_lib::CameraDevice>,
    selected_camera_name: Option<String>,
    mic_dropdown_open: bool,
    available_mics: Vec<socket_lib::AudioDevice>,
    selected_mic_name: Option<String>,
}

impl CallControlsState {
    pub fn new(
        camera_active: bool,
        selected_camera_name: Option<String>,
        selected_mic_name: Option<String>,
    ) -> Self {
        Self {
            camera_active,
            selected_camera_name,
            selected_mic_name,
            ..Self::default()
        }
    }

    pub fn set_camera_active(&mut self, active: bool, device_name: Option<String>) {
        self.camera_active = active;
        if active {
            self.selected_camera_name = device_name;
        }
    }

    pub fn set_selected_mic_name(&mut self, name: Option<String>) {
        self.selected_mic_name = name;
    }

    pub fn dismiss_dropdowns(&mut self) {
        self.camera_dropdown_open = false;
        self.mic_dropdown_open = false;
    }

    pub fn has_open_dropdown(&self) -> bool {
        self.camera_dropdown_open || self.mic_dropdown_open
    }

    pub fn view<'a>(
        &'a self,
        participants: &'a Arc<RwLock<HashMap<String, ParticipantInfo>>>,
        density: CallControlsDensity,
    ) -> Element<'a, CallControlsMessage, Theme, iced::Renderer> {
        let (is_muted, is_screensharing, is_capturing) = participants
            .read()
            .ok()
            .and_then(|participants| {
                participants
                    .get("local")
                    .map(|local| (local.muted(), local.is_screensharing(), local.capturing()))
            })
            .unwrap_or((false, false, false));
        let size = density.button_size();
        let has_menus = density.has_device_menus();

        let mic = split_button_sized(
            if is_muted {
                ICON_MICROPHONE_OFF
            } else {
                ICON_MICROPHONE_ON
            },
            if is_muted {
                ColorToken::Gray400.to_color()
            } else {
                ColorToken::Orange400.to_color()
            },
            CallControlsMessage::MicToggle,
            has_menus.then_some(CallControlsMessage::MicDropdownToggle),
            self.mic_dropdown_open,
            size,
        );
        let video = split_button_sized(
            ICON_VIDEO,
            if self.camera_active {
                ColorToken::Green400.to_color()
            } else {
                ColorToken::Gray400.to_color()
            },
            CallControlsMessage::VideoToggle,
            has_menus.then_some(CallControlsMessage::CameraDropdownToggle),
            self.camera_dropdown_open,
            size,
        );
        let screen = split_button_sized(
            ICON_SCREEN_SHARE,
            if is_screensharing {
                ColorToken::Green400.to_color()
            } else {
                ColorToken::Gray400.to_color()
            },
            CallControlsMessage::ScreenShare,
            has_menus.then_some(CallControlsMessage::OpenScreenSharePicker),
            false,
            size,
        );
        let end_call = split_button_sized(
            ICON_PHONE_OFF,
            ColorToken::Red500.to_color(),
            CallControlsMessage::EndCall,
            None,
            false,
            size,
        );

        let mut controls = row![mic, video].spacing(density.spacing());
        if density != CallControlsDensity::Essentials {
            controls = controls.push(screen);
            if call_capture::SUPPORTED {
                controls = controls.push(capture_control(is_capturing, density));
            }
        }
        controls.push(end_call).into()
    }

    pub fn wrap_dropdown<'a, Message, Map>(
        &'a self,
        base: Element<'a, Message, Theme, iced::Renderer>,
        map: Map,
        density: CallControlsDensity,
        top_offset: f32,
        trailing_padding: f32,
    ) -> Element<'a, Message, Theme, iced::Renderer>
    where
        Message: Clone + 'a,
        Map: Fn(CallControlsMessage) -> Message + Copy + 'a,
    {
        if self.camera_dropdown_open {
            let items: Vec<SplitButtonItem> = self
                .available_cameras
                .iter()
                .map(|camera| SplitButtonItem {
                    label: camera.name.clone(),
                    selected: self
                        .selected_camera_name
                        .as_ref()
                        .map_or(camera.default, |selected| selected == &camera.name),
                })
                .collect();
            split_button_dropdown_wrap(
                base,
                &items,
                map(CallControlsMessage::CameraDropdownDismiss),
                move |index| {
                    map(CallControlsMessage::SelectCamera(
                        self.available_cameras[index].name.clone(),
                    ))
                },
                top_offset,
                trailing_padding + density.camera_dropdown_tail(),
            )
        } else if self.mic_dropdown_open {
            let items: Vec<SplitButtonItem> = self
                .available_mics
                .iter()
                .map(|mic| SplitButtonItem {
                    label: mic.name.clone(),
                    selected: self
                        .selected_mic_name
                        .as_ref()
                        .map_or(mic.default, |selected| selected == &mic.name),
                })
                .collect();
            split_button_dropdown_wrap(
                base,
                &items,
                map(CallControlsMessage::MicDropdownDismiss),
                move |index| {
                    map(CallControlsMessage::SelectMic(
                        self.available_mics[index].name.clone(),
                    ))
                },
                top_offset,
                trailing_padding + density.mic_dropdown_tail(),
            )
        } else {
            base
        }
    }

    pub fn update(
        &mut self,
        message: CallControlsMessage,
        participants: &Arc<RwLock<HashMap<String, ParticipantInfo>>>,
        event_loop_proxy: &EventLoopProxy<UserEvent>,
    ) {
        let send = |event| {
            if let Err(error) = event_loop_proxy.send_event(event) {
                log::error!("CallControls: failed to send event: {error:?}");
            }
        };

        match message {
            CallControlsMessage::MicToggle => {
                let muted = participants
                    .read()
                    .ok()
                    .and_then(|participants| participants.get("local").map(ParticipantInfo::muted))
                    .unwrap_or(false);
                send(if muted {
                    UserEvent::UnmuteAudio
                } else {
                    UserEvent::MuteAudio
                });
            }
            CallControlsMessage::MicDropdownToggle => {
                self.camera_dropdown_open = false;
                if !self.mic_dropdown_open {
                    self.available_mics = list_audio_inputs();
                }
                self.mic_dropdown_open = !self.mic_dropdown_open;
            }
            CallControlsMessage::MicDropdownDismiss => self.mic_dropdown_open = false,
            CallControlsMessage::SelectMic(name) => {
                self.mic_dropdown_open = false;
                send(UserEvent::StartAudioCapture {
                    msg: AudioCaptureMessage { device_name: name },
                    from_socket: false,
                });
            }
            CallControlsMessage::VideoToggle => send(if self.camera_active {
                UserEvent::StopCamera
            } else {
                UserEvent::StartCamera {
                    msg: CameraStartMessage { device_name: None },
                    from_socket: false,
                }
            }),
            CallControlsMessage::CameraDropdownToggle => {
                self.mic_dropdown_open = false;
                if !self.camera_dropdown_open {
                    self.available_cameras = CameraCapturer::list_devices();
                }
                self.camera_dropdown_open = !self.camera_dropdown_open;
            }
            CallControlsMessage::CameraDropdownDismiss => self.camera_dropdown_open = false,
            CallControlsMessage::SelectCamera(name) => {
                self.camera_dropdown_open = false;
                send(UserEvent::StartCamera {
                    msg: CameraStartMessage {
                        device_name: Some(name),
                    },
                    from_socket: false,
                });
            }
            CallControlsMessage::ScreenShare => {
                let active = participants
                    .read()
                    .ok()
                    .and_then(|participants| {
                        participants
                            .get("local")
                            .map(ParticipantInfo::is_screensharing)
                    })
                    .unwrap_or(false);
                send(if active {
                    UserEvent::StopScreenShare
                } else {
                    UserEvent::GetAvailableContent
                });
            }
            CallControlsMessage::OpenScreenSharePicker => send(UserEvent::GetAvailableContent),
            CallControlsMessage::ToggleCapture => {
                send(UserEvent::AppAction(AppAction::ToggleCapture));
            }
            CallControlsMessage::OpenCaptures => {
                send(UserEvent::AppAction(AppAction::OpenCaptures));
            }
            CallControlsMessage::EndCall => send(UserEvent::CallEnd),
        }
    }
}

fn capture_control<'a>(
    is_capturing: bool,
    density: CallControlsDensity,
) -> Element<'a, CallControlsMessage, Theme, iced::Renderer> {
    let control = mouse_area(svg_button_sized(
        ICON_CAPTURE,
        if is_capturing {
            ColorToken::Rose500.to_color()
        } else {
            ColorToken::Gray400.to_color()
        },
        CallControlsMessage::ToggleCapture,
        density.button_size(),
    ))
    .on_right_press(CallControlsMessage::OpenCaptures);

    let hint = container(
        text(if is_capturing {
            "Stop capturing · right-click for your captures"
        } else {
            "Capture this call · right-click for your captures"
        })
        .size(12)
        .color(Color::WHITE),
    )
    .padding(Padding::from([4.0, 8.0]))
    .style(|_theme: &Theme| container::Style {
        background: Some(Background::Color(ColorToken::Gray600.to_color())),
        border: Border {
            color: Color::from_rgba(1.0, 1.0, 1.0, 0.15),
            width: 1.0,
            radius: 6.0.into(),
        },
        shadow: ShadowToken::Xs.to_shadow(),
        ..Default::default()
    });

    // Without the menus the controls are at the bottom of the window, with no room below them.
    let hint_position = if density.has_device_menus() {
        tooltip::Position::Bottom
    } else {
        tooltip::Position::Top
    };
    tooltip(control, hint, hint_position)
        .gap(4)
        .snap_within_viewport(true)
        .into()
}
