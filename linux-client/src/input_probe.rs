//! Explicit, scoped native-input check. Every injected step verifies this
//! probe is the focused window and the compositor is unlocked.
use crate::{
    native_input::{InputModifiers, NativeInput},
    protocol::ControlGrant,
};
use anyhow::{Context as _, Result, bail, ensure};
use gpui_kit::{
    AppContext, Bounds, Context, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, Styled, TitlebarOptions, Window, WindowBounds, WindowOptions, div, px,
    size,
};
use std::{
    collections::HashSet,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const CLASS: &str = "hopp-input-probe";
#[derive(Default)]
struct Observed {
    moves: usize,
    downs: usize,
    ups: usize,
    typed: String,
    pressed: HashSet<String>,
    released: HashSet<String>,
}
type Seen = Arc<Mutex<Observed>>;
type Outcome = Arc<Mutex<Option<std::result::Result<(), String>>>>;
struct Probe {
    seen: Seen,
    focus: FocusHandle,
}
impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(gpui_kit::rgb(0x162129)).text_color(gpui_kit::rgb(0xe6edf3)).p(px(36.))
            .track_focus(&self.focus)
            .on_mouse_move(cx.listener(|view,_:&gpui_kit::MouseMoveEvent,_,_|{view.seen.lock().unwrap().moves+=1;}))
            .on_mouse_down(MouseButton::Left,cx.listener(|view,_:&gpui_kit::MouseDownEvent,window,cx|{view.focus.focus(window,cx);view.seen.lock().unwrap().downs+=1;}))
            .on_mouse_up(MouseButton::Left,cx.listener(|view,_:&gpui_kit::MouseUpEvent,_,_|{view.seen.lock().unwrap().ups+=1;}))
            .on_key_down(cx.listener(|view,event:&gpui_kit::KeyDownEvent,_,_|{
                let mut seen=view.seen.lock().unwrap();seen.pressed.insert(event.keystroke.key.to_string());
                if let Some(character)=&event.keystroke.key_char {seen.typed.push_str(character);}
            }))
            .on_key_up(cx.listener(|view,event:&gpui_kit::KeyUpEvent,_,_|{view.seen.lock().unwrap().released.insert(event.keystroke.key.to_string());}))
            .child("Hopp native input probe")
            .child(div().mt(px(16.)).child("This test sends pointer clicks and ‘hopp’ only to this window. Changing focus or locking the desktop stops the test."))
    }
}

pub fn run() -> Result<()> {
    ensure!(
        !desktop_locked()?,
        "Unlock the desktop before running the native input probe"
    );
    let seen: Seen = Default::default();
    let outcome: Outcome = Default::default();
    let ui_seen = seen.clone();
    let ui_outcome = outcome.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_omarchy::init(cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(px(780.), px(460.)),
                            cx,
                        ))),
                        titlebar: Some(TitlebarOptions {
                            title: Some("Hopp native input probe".into()),
                            ..Default::default()
                        }),
                        app_id: Some(CLASS.into()),
                        ..Default::default()
                    },
                    move |window, cx| {
                        cx.new(|cx| {
                            let focus = cx.focus_handle();
                            focus.focus(window, cx);
                            Probe {
                                seen: ui_seen.clone(),
                                focus,
                            }
                        })
                    },
                )
                .expect("open native input probe");
            let worker_seen = seen.clone();
            let worker_outcome = ui_outcome.clone();
            thread::spawn(move || {
                let result = worker(worker_seen).map_err(|error| format!("{error:#}"));
                *worker_outcome.lock().unwrap() = Some(result);
            });
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.spawn(async move |cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(50))
                        .await;
                    if ui_outcome.lock().unwrap().is_some() {
                        cx.update(|cx| cx.quit());
                        break;
                    }
                    if window.update(cx, |_, _, _| ()).is_err() {
                        break;
                    }
                }
            })
            .detach();
            cx.activate(true);
        });
    match outcome.lock().unwrap().take() {
        Some(Ok(())) => {
            println!(
                "NATIVE INPUT PASS: scoped pointer movement, clicks, typed text, key/button release on revoke, no typing after revoke."
            );
            Ok(())
        }
        Some(Err(error)) => bail!(error),
        None => bail!("Native input probe was closed before completing"),
    }
}

fn desktop_locked() -> Result<bool> {
    query("locked")?["locked"]
        .as_bool()
        .context("Compositor did not return its lock state")
}
fn query(command: &str) -> Result<serde_json::Value> {
    let output = Command::new("hyprctl")
        .args(["-j", command])
        .output()
        .context("Reading probe compositor state")?;
    ensure!(output.status.success(), "Could not read compositor state");
    serde_json::from_slice(&output.stdout).context("Invalid compositor response")
}
struct Target {
    output: String,
    x: f64,
    y: f64,
}
fn target(x_ratio: f64) -> Result<Target> {
    ensure!(
        !desktop_locked()?,
        "Desktop locked; native input probe stopped"
    );
    let window = query("activewindow")?;
    ensure!(
        window["class"].as_str() == Some(CLASS),
        "Probe window lost focus; no input was injected"
    );
    let monitor = window["monitor"]
        .as_i64()
        .context("Probe has no selected monitor")?;
    let monitors = query("monitors")?;
    let output = monitors
        .as_array()
        .context("Invalid monitor list")?
        .iter()
        .find(|m| m["id"].as_i64() == Some(monitor))
        .context("Probe monitor disappeared")?;
    let name = output["name"]
        .as_str()
        .context("Probe monitor has no name")?
        .to_owned();
    let scale = output["scale"]
        .as_f64()
        .context("Probe monitor has no scale")?;
    let mut width = output["width"].as_f64().context("Monitor width missing")? / scale;
    let mut height = output["height"]
        .as_f64()
        .context("Monitor height missing")?
        / scale;
    if output["transform"].as_i64().unwrap_or(0) % 2 != 0 {
        std::mem::swap(&mut width, &mut height);
    }
    let x = (window["at"][0].as_f64().context("Probe x missing")?
        + window["size"][0].as_f64().context("Probe width missing")? * x_ratio
        - output["x"].as_f64().unwrap_or(0.))
        / width;
    let y = (window["at"][1].as_f64().context("Probe y missing")?
        + window["size"][1].as_f64().context("Probe height missing")? * 0.5
        - output["y"].as_f64().unwrap_or(0.))
        / height;
    ensure!(
        (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
        "Probe target is outside its monitor"
    );
    Ok(Target { output: name, x, y })
}
fn guarded(
    input: &mut NativeInput,
    output: &str,
    action: impl FnOnce(&mut NativeInput) -> Result<()>,
) -> Result<()> {
    ensure!(
        target(0.5)?.output == output,
        "Probe moved to another monitor; input stopped"
    );
    action(input)?;
    thread::sleep(Duration::from_millis(35));
    Ok(())
}
fn wait(seen: &Seen, check: impl Fn(&Observed) -> bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let _ = target(0.5)?;
        if check(&seen.lock().unwrap()) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    bail!("Probe did not receive the expected native input callback")
}
fn worker(seen: Seen) -> Result<()> {
    // Window creation precedes activation; give the compositor a bounded chance
    // to focus it. No input is sent while waiting.
    let mut initial = None;
    for _ in 0..100 {
        if let Ok(point) = target(0.4) {
            initial = Some(point);
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let initial = initial.context("Probe window was not focused within two seconds")?;
    let mut input = NativeInput::connect(&initial.output)?;
    guarded(&mut input, &initial.output, |input| {
        input.move_to(initial.x, initial.y)
    })?;
    let center = target(0.5)?;
    guarded(&mut input, &initial.output, |input| {
        input.move_to(center.x, center.y)
    })?;
    guarded(&mut input, &initial.output, |input| input.button(272, true))?;
    guarded(&mut input, &initial.output, |input| {
        input.button(272, false)
    })?;
    wait(&seen, |events| {
        events.moves > 0 && events.downs >= 1 && events.ups >= 1
    })?;
    let mut grant = ControlGrant::default();
    grant.grant("probe-peer".into());
    for key in ["h", "o", "p", "p"] {
        ensure!(
            grant.allows("probe-peer", true),
            "Probe grant unexpectedly revoked"
        );
        guarded(&mut input, &initial.output, |input| {
            input.key(key, true, InputModifiers::default())
        })?;
        guarded(&mut input, &initial.output, |input| {
            input.key(key, false, InputModifiers::default())
        })?;
    }
    wait(&seen, |events| events.typed == "hopp")?;
    guarded(&mut input, &initial.output, |input| {
        input.key("q", true, InputModifiers::default())
    })?;
    guarded(&mut input, &initial.output, |input| input.button(272, true))?;
    wait(&seen, |events| {
        events.pressed.contains("q") && events.downs >= 2
    })?;
    grant.revoke();
    // Revocation sends release events only. Further press commands fail the
    // same grant check as the engine before they reach a native input device.
    guarded(&mut input, &initial.output, NativeInput::release_all)?;
    wait(&seen, |events| {
        events.released.contains("q") && events.ups >= 2
    })?;
    if grant.allows("probe-peer", true) {
        guarded(&mut input, &initial.output, |input| {
            input.key("z", true, InputModifiers::default())
        })?;
    }
    thread::sleep(Duration::from_millis(120));
    let observed = seen.lock().unwrap();
    ensure!(
        !observed.pressed.contains("z") && observed.typed == "hoppq",
        "Input arrived after revocation"
    );
    Ok(())
}
