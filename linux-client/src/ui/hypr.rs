//! Hyprland window placement for Hopp's own windows.
//!
//! Hyprland decides the size of a floating window once it is mapped and ignores the
//! client resizing itself afterwards. The call window's size is ours to decide (it grows
//! by one tile per person), so we ask the compositor directly, keeping the window pinned
//! to whichever corner of the screen it is nearest.
use std::{
    io::{BufRead, BufReader},
    os::unix::net::UnixStream,
    process::Command,
    sync::mpsc,
    thread,
    time::Duration,
};

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Keep the corner of `window` nearest a corner of `area` fixed while resizing it.
pub fn anchored(window: Rect, area: Rect, width: f64, height: f64) -> Rect {
    let right = window.x + window.width / 2. > area.x + area.width / 2.;
    let bottom = window.y + window.height / 2. > area.y + area.height / 2.;
    let x = if right {
        window.x + window.width - width
    } else {
        window.x
    };
    let y = if bottom {
        window.y + window.height - height
    } else {
        window.y
    };
    // Never leave the usable area.
    Rect {
        x: x.clamp(area.x, (area.x + area.width - width).max(area.x)),
        y: y.clamp(area.y, (area.y + area.height - height).max(area.y)),
        width,
        height,
    }
}

fn hyprctl_json(what: &str) -> Option<Value> {
    let output = Command::new("hyprctl").args(["-j", what]).output().ok()?;
    output
        .status
        .success()
        .then(|| serde_json::from_slice(&output.stdout).ok())
        .flatten()
}

/// The Hyprland plugin that leaves Hopp's overlays out of the screen share.
pub const SHARE_PLUGIN: &str = "hopp-share-clean";

/// Whether Hopp can draw over the monitor you share without your viewers seeing it.
///
/// Hyprland on its own hides a no_screen_share surface by painting a black box over it in
/// the share, so a full-screen overlay would black out the whole screen. The
/// `hopp-share-clean` plugin leaves the overlays out instead. Without it (not installed,
/// or built for another Hyprland and refused), Hopp draws nothing over your shared screen.
pub fn clean_share() -> bool {
    let Ok(output) = Command::new("hyprctl")
        .args(["-j", "plugin", "list"])
        .output()
    else {
        return false;
    };
    output.status.success() && lists_plugin(&output.stdout, SHARE_PLUGIN)
}

fn lists_plugin(json: &[u8], name: &str) -> bool {
    serde_json::from_slice::<Value>(json)
        .ok()
        .and_then(|plugins| {
            plugins
                .as_array()
                .map(|list| list.iter().any(|plugin| plugin["name"] == name))
        })
        .unwrap_or(false)
}

fn number(value: &Value, index: usize) -> f64 {
    value[index].as_f64().unwrap_or_default()
}

/// The space bars and other panels take at each edge of monitor `output` (left, top,
/// right, bottom, in logical pixels): where a layer surface that respects them, such as
/// the sharing pill, ends up. Zero when Hyprland can't say.
pub fn reserved(output: &str) -> [f32; 4] {
    hyprctl_json("monitors")
        .and_then(|monitors| reserved_in(&monitors, output))
        .unwrap_or_default()
}

fn reserved_in(monitors: &Value, output: &str) -> Option<[f32; 4]> {
    let monitor = monitors
        .as_array()?
        .iter()
        .find(|monitor| monitor["name"].as_str() == Some(output))?;
    let reserved = &monitor["reserved"];
    Some([0, 1, 2, 3].map(|edge| number(reserved, edge) as f32))
}

/// Where one of this process's windows is on its monitor.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// From the monitor's top-left corner, in logical pixels.
    pub window: Rect,
    /// The monitor's logical size.
    pub monitor: (f64, f64),
    /// The monitor's output name ("DP-2").
    pub output: String,
}

/// Where this process's window with `class` is. `None` without Hyprland or the window.
pub fn placed(class: &str) -> Option<Placed> {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    placed_in(
        &hyprctl_json("clients")?,
        &hyprctl_json("monitors")?,
        class,
        std::process::id(),
    )
}

fn placed_in(clients: &Value, monitors: &Value, class: &str, pid: u32) -> Option<Placed> {
    let client = clients.as_array()?.iter().find(|client| {
        client["class"] == class && client["pid"].as_u64() == Some(u64::from(pid))
    })?;
    let monitor = monitors
        .as_array()?
        .iter()
        .find(|monitor| monitor["id"] == client["monitor"])?;
    let scale = monitor["scale"].as_f64().filter(|scale| *scale > 0.)?;
    Some(Placed {
        window: Rect {
            x: number(&client["at"], 0) - monitor["x"].as_f64()?,
            y: number(&client["at"], 1) - monitor["y"].as_f64()?,
            width: number(&client["size"], 0),
            height: number(&client["size"], 1),
        },
        monitor: (
            monitor["width"].as_f64()? / scale,
            monitor["height"].as_f64()? / scale,
        ),
        output: monitor["name"].as_str()?.to_owned(),
    })
}

/// The monitor with the keyboard focus: its output name and logical size.
pub fn focused_monitor() -> Option<(String, (f64, f64))> {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    focused_in(&hyprctl_json("monitors")?)
}

fn focused_in(monitors: &Value) -> Option<(String, (f64, f64))> {
    let monitor = monitors
        .as_array()?
        .iter()
        .find(|monitor| monitor["focused"] == true)?;
    let scale = monitor["scale"].as_f64().filter(|scale| *scale > 0.)?;
    Some((
        monitor["name"].as_str()?.to_owned(),
        (
            monitor["width"].as_f64()? / scale,
            monitor["height"].as_f64()? / scale,
        ),
    ))
}

/// Resize this process's window with `class` to `width`×`height` logical pixels.
/// Runs in the background and retries briefly while the window is being mapped.
pub fn fit(class: &'static str, width: f32, height: f32) {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return;
    }
    let pid = std::process::id();
    thread::spawn(move || {
        for _ in 0..20 {
            if place(class, pid, f64::from(width), f64::from(height)).is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    });
}

fn place(class: &str, pid: u32, width: f64, height: f64) -> Option<()> {
    let clients = hyprctl_json("clients")?;
    let client = clients.as_array()?.iter().find(|client| {
        client["class"] == class && client["pid"].as_u64() == Some(u64::from(pid))
    })?;
    let address = client["address"].as_str()?;
    let window = Rect {
        x: number(&client["at"], 0),
        y: number(&client["at"], 1),
        width: number(&client["size"], 0),
        height: number(&client["size"], 1),
    };
    if (window.width - width).abs() < 1. && (window.height - height).abs() < 1. {
        return Some(());
    }
    let monitors = hyprctl_json("monitors")?;
    let monitor = monitors
        .as_array()?
        .iter()
        .find(|monitor| monitor["id"] == client["monitor"])?;
    let scale = monitor["scale"].as_f64().filter(|scale| *scale > 0.)?;
    // Reserved space (the bar) is [left, top, right, bottom]; keep a gaps_out margin.
    let reserved = &monitor["reserved"];
    let margin = 10.;
    let area = Rect {
        x: monitor["x"].as_f64()? + number(reserved, 0) + margin,
        y: monitor["y"].as_f64()? + number(reserved, 1) + margin,
        width: monitor["width"].as_f64()? / scale
            - number(reserved, 0)
            - number(reserved, 2)
            - 2. * margin,
        height: monitor["height"].as_f64()? / scale
            - number(reserved, 1)
            - number(reserved, 3)
            - 2. * margin,
    };
    let target = anchored(window, area, width, height);
    let selector = format!("address:{address}");
    dispatch(&format!(
        "hl.dsp.window.resize({{ x = {}, y = {}, window = \"{selector}\" }})",
        target.width.round(),
        target.height.round()
    ))?;
    dispatch(&format!(
        "hl.dsp.window.move({{ x = {}, y = {}, window = \"{selector}\" }})",
        target.x.round(),
        target.y.round()
    ))
}

/// Whether this process's shared-screen windows are on a workspace that is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Visible {
    pub viewer: bool,
    pub peek: bool,
}

/// Work out visibility from `hyprctl -j clients` and `hyprctl -j monitors`. A window counts
/// as visible when its workspace is the active (or open special) workspace of a monitor.
pub fn visibility(clients: &Value, monitors: &Value, pid: u32) -> Visible {
    let shown: Vec<i64> = monitors
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|monitor| {
            [
                monitor["activeWorkspace"]["id"].as_i64(),
                monitor["specialWorkspace"]["id"]
                    .as_i64()
                    .filter(|id| *id != 0),
            ]
        })
        .flatten()
        .collect();
    let mut visible = Visible {
        viewer: false,
        peek: false,
    };
    for client in clients.as_array().into_iter().flatten() {
        if client["pid"].as_u64() != Some(u64::from(pid)) {
            continue;
        }
        let on_screen = client["workspace"]["id"]
            .as_i64()
            .is_some_and(|id| shown.contains(&id))
            && client["hidden"] != true
            && client["mapped"] != false;
        match client["class"].as_str() {
            Some("hopp.viewer") => visible.viewer |= on_screen,
            Some("hopp.peek") => visible.peek |= on_screen,
            _ => {}
        }
    }
    visible
}

/// Watch Hyprland's event socket and report visibility whenever it may have changed
/// (workspace switches, special workspaces, windows opening, closing or moving).
pub fn watch_visibility(
    wake: &std::sync::Arc<tokio::sync::Notify>,
) -> Option<mpsc::Receiver<Visible>> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let path = std::path::Path::new(&runtime)
        .join("hypr")
        .join(signature)
        .join(".socket2.sock");
    let stream = UnixStream::connect(path).ok()?;
    let pid = std::process::id();
    let (sender, receiver) = crate::wake::channel(wake);
    thread::Builder::new()
        .name("hopp-hypr-events".into())
        .spawn(move || {
            let check = |last: &mut Option<Visible>| -> bool {
                let (Some(clients), Some(monitors)) =
                    (hyprctl_json("clients"), hyprctl_json("monitors"))
                else {
                    return true;
                };
                let now = visibility(&clients, &monitors, pid);
                if *last != Some(now) {
                    *last = Some(now);
                    return sender.send(now).is_ok();
                }
                true
            };
            let mut last = None;
            if !check(&mut last) {
                return;
            }
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                let event = line.split(">>").next().unwrap_or_default();
                let relevant = matches!(
                    event.trim_end_matches("v2"),
                    "workspace"
                        | "activespecial"
                        | "focusedmon"
                        | "movewindow"
                        | "openwindow"
                        | "closewindow"
                        | "monitoradded"
                        | "monitorremoved"
                );
                if relevant && !check(&mut last) {
                    break;
                }
            }
        })
        .ok()?;
    Some(receiver)
}

fn dispatch(lua: &str) -> Option<()> {
    let status = Command::new("hyprctl")
        .args(["dispatch", lua])
        .output()
        .ok()?;
    (status.status.success() && String::from_utf8_lossy(&status.stdout).trim() == "ok")
        .then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_is_placed_from_its_monitors_corner() {
        let clients = serde_json::json!([
            {"class": "hopp.faces", "pid": 7, "at": [6058, 322], "size": [332, 1018], "monitor": 2},
            {"class": "hopp.faces", "pid": 8, "at": [0, 0], "size": [1, 1], "monitor": 1}
        ]);
        let monitors = serde_json::json!([
            {"id": 1, "name": "eDP-1", "x": 0, "y": 0, "width": 3200, "height": 2000, "scale": 2.0},
            {"id": 2, "name": "DP-2", "x": 3200, "y": 0, "width": 5120, "height": 2160, "scale": 1.6}
        ]);
        assert_eq!(
            placed_in(&clients, &monitors, "hopp.faces", 7),
            Some(Placed {
                window: Rect {
                    x: 2858.,
                    y: 322.,
                    width: 332.,
                    height: 1018.
                },
                monitor: (3200., 1350.),
                output: "DP-2".into(),
            })
        );
        assert_eq!(placed_in(&clients, &monitors, "hopp.call", 7), None);
    }

    #[test]
    fn reserved_space_is_read_for_the_named_monitor() {
        let monitors = serde_json::json!([
            {"name": "DP-2", "reserved": [0, 0, 0, 0]},
            {"name": "eDP-1", "reserved": [0, 37, 0, 0]}
        ]);
        assert_eq!(reserved_in(&monitors, "eDP-1"), Some([0., 37., 0., 0.]));
        assert_eq!(reserved_in(&monitors, "DP-2"), Some([0.; 4]));
        assert_eq!(reserved_in(&monitors, "HDMI-A-1"), None);
    }

    #[test]
    fn the_focused_monitor_is_found_at_its_logical_size() {
        let monitors = serde_json::json!([
            {"name": "eDP-1", "width": 3200, "height": 2000, "scale": 2.0, "focused": false},
            {"name": "DP-2", "width": 5120, "height": 2160, "scale": 1.6, "focused": true}
        ]);
        assert_eq!(focused_in(&monitors), Some(("DP-2".into(), (3200., 1350.))));
        assert_eq!(focused_in(&serde_json::json!([])), None);
    }

    #[test]
    fn the_share_plugin_is_found_by_name_in_the_plugin_list() {
        let list =
            br#"[{"name": "hypr-kinetic-scroll", "version": "1.0"}, {"name": "hopp-share-clean"}]"#;
        assert!(lists_plugin(list, SHARE_PLUGIN));
        assert!(!lists_plugin(
            br#"[{"name": "hypr-kinetic-scroll"}]"#,
            SHARE_PLUGIN
        ));
        assert!(!lists_plugin(b"no plugins loaded", SHARE_PLUGIN));
    }

    const SCREEN: Rect = Rect {
        x: 10.,
        y: 40.,
        width: 1708.,
        height: 1030.,
    };

    #[test]
    fn a_window_in_the_bottom_right_grows_up_and_left() {
        let window = Rect {
            x: 1594.,
            y: 762.,
            width: 124.,
            height: 308.,
        };
        let grown = anchored(window, SCREEN, 124., 426.);
        assert_eq!((grown.x, grown.y), (1594., 644.));
        assert_eq!(grown.y + grown.height, window.y + window.height);
    }

    #[test]
    fn a_window_moved_to_the_top_left_grows_down_and_right() {
        let window = Rect {
            x: 40.,
            y: 80.,
            width: 124.,
            height: 308.,
        };
        let grown = anchored(window, SCREEN, 306., 84.);
        assert_eq!((grown.x, grown.y), (40., 80.));
    }

    #[test]
    fn visibility_follows_the_active_and_special_workspaces() {
        let clients = serde_json::json!([
            {"pid": 7, "class": "hopp.viewer", "workspace": {"id": 2}, "mapped": true, "hidden": false},
            {"pid": 7, "class": "hopp.peek", "workspace": {"id": -98}, "mapped": true, "hidden": false},
            {"pid": 8, "class": "hopp.viewer", "workspace": {"id": 1}, "mapped": true, "hidden": false}
        ]);
        let on = |active: i64, special: i64| {
            let monitors = serde_json::json!([
                {"activeWorkspace": {"id": active}, "specialWorkspace": {"id": special}}
            ]);
            visibility(&clients, &monitors, 7)
        };
        assert_eq!(
            on(2, 0),
            Visible {
                viewer: true,
                peek: false
            }
        );
        // Another workspace: our viewer is off screen (pid 8's doesn't count).
        assert_eq!(
            on(1, 0),
            Visible {
                viewer: false,
                peek: false
            }
        );
        // special:hopp open over workspace 1.
        assert_eq!(
            on(1, -98),
            Visible {
                viewer: false,
                peek: true
            }
        );
    }

    #[test]
    fn growth_never_leaves_the_screen() {
        let window = Rect {
            x: 1594.,
            y: 900.,
            width: 124.,
            height: 160.,
        };
        let grown = anchored(window, SCREEN, 124., 2000.);
        assert_eq!(grown.y, SCREEN.y);
    }
}
