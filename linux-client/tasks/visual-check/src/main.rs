//! Synthetic scenes for the native UI. Every user, camera and screen here is generated;
//! nothing authenticates, captures, transmits or injects input.
use std::collections::HashMap;

use hopp_visual_fixture::{
    api::{RoomInfo, Teammate, UserInfo},
    engine::{Engine, Event, Participant, SharedFrames},
    frames::{self, VideoFrame},
    protocol::{InputEvent, PathPoint, Point},
    settings::{FaceLayout, FaceSize, Settings},
    ui,
};

fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args();
    while let Some(value) = args.next() {
        if value == name {
            return args.next();
        }
    }
    None
}

/// A dark editor-and-terminal screen with code-like rows, so cropping and readability
/// are visible in screenshots.
fn remote_screen(width: usize, height: usize) -> VideoFrame {
    let mut bgra = vec![0; width * height * 4];
    let put = |bgra: &mut Vec<u8>, x: usize, y: usize, [r, g, b]: [u8; 3]| {
        let i = (y * width + x) * 4;
        bgra[i..i + 4].copy_from_slice(&[b, g, r, 255]);
    };
    let split = width * 3 / 5;
    let bar = height / 40;
    let palette = [
        [187, 154, 247],
        [122, 162, 247],
        [158, 206, 106],
        [224, 175, 104],
        [125, 207, 255],
        [169, 177, 214],
    ];
    for y in 0..height {
        for x in 0..width {
            let mut color = if y < bar {
                [22, 22, 30]
            } else if x < split {
                [26, 27, 38]
            } else {
                [19, 20, 28]
            };
            // Window borders.
            if y >= bar && (x == split || x == split + 1 || x < 2 || x > width - 3) {
                color = [65, 72, 104];
            }
            // Code rows: indented runs of coloured tokens on a 28px line pitch.
            let line = (y.saturating_sub(bar + 30)) / 28;
            let within = (y.saturating_sub(bar + 30)) % 28;
            if y > bar + 30 && (6..18).contains(&within) && line < 48 {
                let pane_left = if x < split { 70 } else { split + 30 };
                let indent = (line * 7 % 5) * 40;
                let start = pane_left + indent;
                let tokens = 3 + line * 13 % 6;
                let mut cursor = start;
                for token in 0..tokens {
                    let len = 40 + (line * 31 + token * 17) % 160;
                    if (cursor..cursor + len).contains(&x)
                        && x < if x < split { split - 30 } else { width - 30 }
                    {
                        color = palette[(line + token) % palette.len()];
                    }
                    cursor += len + 18;
                }
                if x < split && (20..52).contains(&x) {
                    color = [86, 95, 137];
                }
            }
            put(&mut bgra, x, y, color);
        }
    }
    VideoFrame {
        width: width as u32,
        height: height as u32,
        bgra,
    }
}

/// A 4:3 webcam-like portrait; tiles must crop it to a square without stretching.
fn camera(width: usize, height: usize, tint: [u8; 3]) -> VideoFrame {
    let mut bgra = vec![0; width * height * 4];
    let (cx, cy) = (width as f32 / 2., height as f32 * 0.42);
    for y in 0..height {
        for x in 0..width {
            let fx = x as f32;
            let fy = y as f32;
            let wall = 0.55 + 0.45 * (fy / height as f32);
            let mut color = [
                (tint[0] as f32 * 0.35 * wall) as u8,
                (tint[1] as f32 * 0.35 * wall) as u8,
                (tint[2] as f32 * 0.35 * wall) as u8,
            ];
            let head = ((fx - cx) / (width as f32 * 0.14)).powi(2)
                + ((fy - cy) / (height as f32 * 0.21)).powi(2);
            let shoulders = ((fx - cx) / (width as f32 * 0.34)).powi(2)
                + ((fy - height as f32 * 1.05) / (height as f32 * 0.42)).powi(2);
            if head < 1. || shoulders < 1. {
                color = [
                    (tint[0] as f32 * 0.9) as u8,
                    (tint[1] as f32 * 0.9) as u8,
                    (tint[2] as f32 * 0.9) as u8,
                ];
            }
            let i = (y * width + x) * 4;
            bgra[i..i + 4].copy_from_slice(&[color[2], color[1], color[0], 255]);
        }
    }
    VideoFrame {
        width: width as u32,
        height: height as u32,
        bgra,
    }
}

fn person(id: &str, first: &str, last: &str, active: bool) -> Teammate {
    Teammate {
        id: id.into(),
        first_name: first.into(),
        last_name: last.into(),
        email: format!("{}@example.test", first.to_lowercase()),
        is_active: active,
        status: None,
    }
}

fn main() {
    let scene = arg("--scene").unwrap_or_else(|| "home".into());
    let people: usize = arg("--people")
        .and_then(|value| value.parse().ok())
        .unwrap_or(2);
    let (commands, mut rx) = tokio::sync::mpsc::channel(256);
    let wake = std::sync::Arc::new(tokio::sync::Notify::new());
    let (events, receiver) = hopp_visual_fixture::wake::channel(&wake);
    let shared = SharedFrames {
        remote: frames::slot(),
        local_camera: frames::slot(),
        cameras: Default::default(),
    };
    let send = |event: Event| events.send(event).unwrap();

    let signed_in = scene != "signin";
    if signed_in {
        send(Event::LoggedIn(UserInfo {
            id: "local".into(),
            first_name: "Alex".into(),
            last_name: "Morgan".into(),
            email: "alex@example.test".into(),
            status: None,
            status_auto: false,
        }));
        send(Event::Rooms(vec![
            RoomInfo {
                id: "1".into(),
                name: "Standup".into(),
            },
            RoomInfo {
                id: "2".into(),
                name: "Friday pairing".into(),
            },
        ]));
        send(Event::Teammates(vec![
            person("local", "Alex", "Morgan", true),
            person("mira", "Mira", "Chen", true),
            person("jonas", "Jonas", "Weber", true),
            person("ana", "Ana", "Souza", true),
            person("lee", "Lee", "Park", true),
            person("sam", "Sam", "Ortiz", false),
        ]));
        send(Event::RoomPresence(HashMap::from([(
            "1".into(),
            vec!["lee".into()],
        )])));
    }

    let in_call = !matches!(
        scene.as_str(),
        "signin"
            | "home"
            | "settings"
            | "settings-capture"
            | "call"
            | "call-missed"
            | "home-missed"
    );
    if in_call {
        let roster = [
            ("mira:audio", "Mira Chen"),
            ("jonas:audio", "Jonas Weber"),
            ("ana:audio", "Ana Souza"),
            ("lee:audio", "Lee Park"),
        ];
        send(Event::Joined(RoomInfo {
            id: "2".into(),
            name: "Friday pairing".into(),
        }));
        send(Event::Connected);
        send(Event::Participants(
            roster
                .iter()
                .take(people.saturating_sub(1).max(1))
                .enumerate()
                .map(|(index, (identity, name))| Participant {
                    identity: (*identity).into(),
                    name: (*name).into(),
                    // One person is capturing, to show the badge, and one is muted.
                    capturing: index == 0,
                    muted: index == 1,
                })
                .collect(),
        ));
        send(Event::Microphone(true));
        send(Event::Camera(true));
        *shared.local_camera.lock().unwrap() = Some(camera(640, 480, [122, 162, 247]));
        let tints = [
            [187, 154, 247],
            [235, 146, 123],
            [125, 207, 255],
            [158, 206, 106],
        ];
        for (index, (identity, _)) in roster.iter().take(people.saturating_sub(1)).enumerate() {
            // Jonas keeps his camera off to show the initials tile.
            if identity.starts_with("jonas") {
                continue;
            }
            let slot = frames::slot();
            *slot.lock().unwrap() = Some(camera(640, 480, tints[index % tints.len()]));
            shared
                .cameras
                .lock()
                .unwrap()
                .insert(identity.replace(":audio", ":camera"), slot);
        }
    }
    if scene == "faces-speaking" {
        // Mira is talking and you're speaking up: her name in green, your ring thick.
        send(Event::Speaking {
            identities: vec!["mira".into()],
            you: true,
        });
        send(Event::MicrophoneLevel(0.8));
    }
    if matches!(scene.as_str(), "faces-heard" | "heard" | "heard-beside") {
        // You said the wake word: a light round your tile, your words in the bubble.
        let words = "So, Jeepers, why's the build failing on main since this morning?"
            .split_whitespace()
            .map(|word| (word.to_owned(), word.starts_with("Jeepers")))
            .collect();
        send(Event::WakeHeard {
            identity: None,
            words: Some(words),
        });
    }
    if matches!(scene.as_str(), "offrecord" | "offrecord-ending") {
        // Someone said "tinfoil hat": the pill, at the start and in its last seconds.
        send(Event::OffRecord(Some(std::time::Duration::from_millis(
            if scene == "offrecord" { 107_900 } else { 4_900 },
        ))));
    }
    if scene == "faces-silent" {
        send(Event::MicrophoneSilent(Some("USB Microphone".into())));
    }
    if matches!(scene.as_str(), "viewer" | "peek" | "faces-sharing") {
        send(Event::RemoteShare {
            identity: "mira:video".into(),
            active: true,
        });
        *shared.remote.lock().unwrap() = Some(remote_screen(2560, 1600));
        // Mira's cursor on the right-hand pane, Jonas pointing at the code.
        send(Event::Pointer {
            sender: "mira:audio".into(),
            point: Point { x: 0.72, y: 0.34 },
        });
        send(Event::Pointer {
            sender: "jonas:audio".into(),
            point: Point { x: 0.46, y: 0.42 },
        });
        send(Event::Drawing {
            sender: "jonas:audio".into(),
            event: InputEvent::DrawStart(PathPoint {
                point: Point { x: 0.3, y: 0.38 },
                path_id: 1,
            }),
        });
        for step in 1..=40 {
            let angle = step as f64 / 40. * std::f64::consts::TAU;
            send(Event::Drawing {
                sender: "jonas:audio".into(),
                event: InputEvent::DrawAddPoint(Point {
                    x: 0.38 - 0.08 * angle.cos(),
                    y: 0.38 + 0.03 * angle.sin(),
                }),
            });
        }
        // A stamp Mira finished, and one Jonas is still speaking over.
        use hopp_visual_fixture::protocol::StampMessage;
        for (sender, message) in [
            (
                "mira:audio",
                StampMessage::StampPlace {
                    id: 1,
                    x: 0.80,
                    y: 0.30,
                },
            ),
            (
                "mira:audio",
                StampMessage::StampText {
                    id: 1,
                    text: "This should subtract the discount before it renders the total".into(),
                    draft: String::new(),
                    done: true,
                },
            ),
            (
                "jonas:audio",
                StampMessage::StampPlace {
                    id: 7,
                    x: 0.22,
                    y: 0.70,
                },
            ),
            (
                "jonas:audio",
                StampMessage::StampText {
                    id: 7,
                    text: "The cache key here ignores the".into(),
                    draft: "user's locale".into(),
                    done: false,
                },
            ),
        ] {
            send(Event::Stamp {
                sender: sender.into(),
                message,
            });
        }
        // Mira underlines something on the right: each person's ink in their colour.
        send(Event::Drawing {
            sender: "mira:audio".into(),
            event: InputEvent::DrawStart(PathPoint {
                point: Point { x: 0.62, y: 0.52 },
                path_id: 1,
            }),
        });
        for step in 1..=20 {
            send(Event::Drawing {
                sender: "mira:audio".into(),
                event: InputEvent::DrawAddPoint(Point {
                    x: 0.62 + 0.012 * step as f64,
                    y: 0.52 + 0.002 * (step as f64 * 0.9).sin(),
                }),
            });
        }
    }
    if scene == "cards" {
        use hopp_visual_fixture::cards::{CardRequest, Question};
        let card = |id: &str, title: &str, body: Option<&str>, question: Option<(&str, &str)>| {
            CardRequest {
                id: id.into(),
                title: title.into(),
                body: body.map(str::to_owned),
                sender: Some("Claude".into()),
                question: question.map(|(accept, reject)| Question {
                    accept: accept.into(),
                    reject: reject.into(),
                }),
                ephemeral: false,
            }
        };
        // Oldest first; the newest shows on top, and the oldest waits behind "+1 more".
        for request in [
            card("card_1", "Waiting behind the others", None, None),
            card(
                "card_2",
                "The flaky test is a race in the cache warmer",
                Some(
                    "warm() starts before the store is open on a cold start. Lee's fix \
                     in #1271 orders them; it passed 50 runs here.",
                ),
                None,
            ),
            card(
                "card_3",
                "Lunch order closes at 12:30",
                Some(
                    "A notice with a body long enough to need clamping to three lines, so \
                      this sentence runs on well past the end of the third line of the card \
                      to show the clamp.",
                ),
                None,
            ),
            card(
                "card_4",
                "Deploy the fix to staging?",
                Some("From the call: Lee asked for the cache fix on staging before review."),
                Some(("Deploy", "Not yet")),
            ),
        ] {
            send(Event::Ui(hopp_visual_fixture::engine::UiRequest::Card(
                request,
            )));
        }
    }
    if scene == "call-missed" || scene == "home-missed" {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        for (id, name, invite, ago) in [
            ("jonas", "Jonas Weber", true, 26 * 3600),
            ("mira", "Mira Chen", false, 900),
            ("mira", "Mira Chen", false, 300),
        ] {
            send(Event::Missed {
                caller_id: id.into(),
                name: name.into(),
                invite,
                at_ms: now - ago * 1000,
            });
        }
    }
    if scene == "call" {
        send(Event::IncomingCall {
            caller_id: "mira".into(),
            name: "Mira Chen".into(),
        });
    }
    if scene == "marks" {
        // You're sharing; two viewers point, draw and stamp on your screen.
        send(Event::Sharing {
            active: true,
            target: Some(hopp_visual_fixture::screencast::CaptureTarget {
                output_name: Some("HEADLESS-1".into()),
                logical_width: 1728,
                logical_height: 1080,
                logical_origin: Some((0., 0.)),
            }),
        });
        use hopp_visual_fixture::protocol::StampMessage;
        for (sender, message) in [
            (
                "mira:audio",
                StampMessage::StampPlace {
                    id: 1,
                    x: 0.70,
                    y: 0.36,
                },
            ),
            (
                "mira:audio",
                StampMessage::StampText {
                    id: 1,
                    text: "This should subtract the discount".into(),
                    draft: "before it renders".into(),
                    done: false,
                },
            ),
        ] {
            send(Event::Stamp {
                sender: sender.into(),
                message,
            });
        }
        send(Event::Drawing {
            sender: "jonas:audio".into(),
            event: InputEvent::DrawStart(PathPoint {
                point: Point { x: 0.30, y: 0.55 },
                path_id: 3,
            }),
        });
        for step in 1..=44 {
            let angle = step as f64 / 40. * std::f64::consts::TAU;
            send(Event::Drawing {
                sender: "jonas:audio".into(),
                event: InputEvent::DrawAddPoint(Point {
                    x: 0.38 - 0.08 * angle.cos(),
                    y: 0.55 + 0.05 * angle.sin(),
                }),
            });
        }
        send(Event::Drawing {
            sender: "jonas:audio".into(),
            event: InputEvent::ClickAnimation(Point { x: 0.55, y: 0.75 }),
        });
    }
    if scene == "drawpad" {
        send(Event::Sharing {
            active: true,
            target: Some(hopp_visual_fixture::screencast::CaptureTarget {
                output_name: Some("eDP-1".into()),
                logical_width: 1280,
                logical_height: 720,
                logical_origin: Some((0., 0.)),
            }),
        });
        send(Event::Ui(hopp_visual_fixture::engine::UiRequest::Draw));
    }
    if scene == "sharing" || scene == "sharing-draw" || scene == "sharing-stamp" {
        send(Event::Sharing {
            active: true,
            target: Some(hopp_visual_fixture::screencast::CaptureTarget {
                output_name: Some("eDP-1".into()),
                logical_width: 1728,
                logical_height: 1080,
                logical_origin: Some((0., 0.)),
            }),
        });
        if scene == "sharing" {
            send(Event::ControlGranted(Some("mira:audio".into())));
        } else if scene == "sharing-stamp" {
            // The stamp tool on your own screen: the pill says how to leave one.
            send(Event::Ui(hopp_visual_fixture::engine::UiRequest::Stamp));
        } else {
            // Drawing on your own screen: the pill becomes the drawing bar.
            send(Event::Ui(hopp_visual_fixture::engine::UiRequest::Draw));
        }
    }
    if scene == "home-call" {
        send(Event::Sharing {
            active: true,
            target: None,
        });
    }
    send(Event::Devices {
        inputs: vec!["Built-in microphone".into()],
        outputs: vec!["Built-in speakers".into()],
        cameras: vec![("/dev/video0".into(), "USB Camera".into())],
    });

    if matches!(scene.as_str(), "viewer" | "peek" | "marks") {
        // Keep the cursors moving a little, so their names show when the shot is taken.
        let events = events.clone();
        std::thread::spawn(move || {
            for step in 0..200 {
                let wobble = (step as f64 * 0.4).sin() * 0.004;
                for (sender, x, y) in [("mira:audio", 0.62, 0.46), ("jonas:audio", 0.46, 0.42)] {
                    let _ = events.send(Event::Pointer {
                        sender: sender.into(),
                        point: Point {
                            x: x + wobble,
                            y: y - wobble,
                        },
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
        });
    }
    std::thread::spawn(move || {
        while let Some(command) = rx.blocking_recv() {
            if matches!(command, hopp_visual_fixture::engine::Command::Shutdown) {
                break;
            }
        }
    });
    let engine = Engine {
        commands,
        events: receiver,
        shared,
        wake,
    };
    let settings = Settings {
        face_layout: match arg("--layout").as_deref() {
            Some("row") => FaceLayout::Row,
            Some("bubbles") => FaceLayout::Bubbles,
            _ => FaceLayout::Column,
        },
        face_size: match arg("--size").as_deref() {
            Some("small") => FaceSize::Small,
            Some("large") => FaceSize::Large,
            _ => FaceSize::Medium,
        },
        ..Settings::default()
    };
    let only = match scene.as_str() {
        "signin" | "home" | "home-call" | "home-missed" => "home",
        "call-missed" => "call",
        "faces" | "faces-sharing" | "faces-speaking" | "faces-silent" | "faces-heard" => "faces",
        // The pill alone: the pad (transparent) would cover it in the harness.
        "sharing-draw" | "sharing-stamp" => "sharing",
        "offrecord-ending" => "offrecord",
        other => other,
    };
    ui::open(
        engine,
        settings,
        ui::Options {
            smoke_test: false,
            background: !matches!(only, "home" | "settings" | "settings-capture"),
            only: Some(only.into()),
            publish: None,
        },
    );
}
