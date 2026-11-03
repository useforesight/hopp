//! Test driver limited to two known windows in a device-isolated compositor.
#[allow(dead_code)]
#[path = "../../../src/native_input.rs"]
mod native_input;
use anyhow::{Context, Result, bail, ensure};
use native_input::{InputModifiers, NativeInput};
use std::{path::Path, process::Command, thread, time::Duration};

fn query(name: &str) -> Result<serde_json::Value> {
    let result = Command::new("hyprctl").args(["-j", name]).output()?;
    ensure!(result.status.success(), "Compositor query failed");
    Ok(serde_json::from_slice(&result.stdout)?)
}
fn focused(class: &str) -> Result<serde_json::Value> {
    ensure!(
        query("locked")?["locked"] == false,
        "Test desktop must be unlocked"
    );
    let window = query("activewindow")?;
    ensure!(
        window["class"].as_str() == Some(class),
        "Test window lost focus; input stopped"
    );
    Ok(window)
}
fn main() -> Result<()> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").context("Private runtime required")?;
    ensure!(
        Path::new(&runtime).starts_with(std::env::temp_dir()),
        "Use a private /tmp desktop runtime"
    );
    ensure!(
        !Path::new("/dev/input").exists() && !Path::new("/run/user").exists(),
        "Actual devices and session sockets must be hidden"
    );
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() >= 3,
        "Usage: hopp-private-input CLASS click X Y | key DOM_KEY [ctrl shift alt meta]"
    );
    let class = args[0].as_str();
    ensure!(
        matches!(class, "hopp-native-editor" | "hyprland-share-picker"),
        "Only the isolated editor or portal chooser may receive input"
    );
    let window = focused(class)?;
    let monitors = query("monitors")?;
    let monitor = monitors
        .as_array()
        .context("No monitors")?
        .iter()
        .find(|m| m["id"] == window["monitor"])
        .context("Target output disappeared")?;
    let output = monitor["name"].as_str().context("Output name missing")?;
    let mut input = NativeInput::connect(output)?;
    // Qt needs time to receive the newly created seat keyboard and its keymap.
    thread::sleep(Duration::from_millis(200));
    match args[1].as_str() {
        "click" => {
            ensure!(args.len() == 4, "Click needs window-relative X Y");
            let x: f64 = args[2].parse()?;
            let y: f64 = args[3].parse()?;
            ensure!(
                x.is_finite()
                    && y.is_finite()
                    && (0.0..=1.0).contains(&x)
                    && (0.0..=1.0).contains(&y),
                "Click is outside the test window"
            );
            let scale = monitor["scale"].as_f64().context("Scale missing")?;
            let mut width = monitor["width"].as_f64().context("Width missing")? / scale;
            let mut height = monitor["height"].as_f64().context("Height missing")? / scale;
            if monitor["transform"].as_u64().unwrap_or(0) % 2 != 0 {
                std::mem::swap(&mut width, &mut height);
            }
            let coord = |index: usize, ratio: f64, dimension: f64, offset: &str| -> Result<f64> {
                Ok((window["at"][index]
                    .as_f64()
                    .context("Window position missing")?
                    + window["size"][index]
                        .as_f64()
                        .context("Window size missing")?
                        * ratio
                    - monitor[offset]
                        .as_f64()
                        .context("Output position missing")?)
                    / dimension)
            };
            focused(class)?;
            input.move_to(coord(0, x, width, "x")?, coord(1, y, height, "y")?)?;
            thread::sleep(Duration::from_millis(35));
            focused(class)?;
            input.button(272, true)?;
            thread::sleep(Duration::from_millis(35));
            input.button(272, false)?;
        }
        "key" => {
            let mut modifiers = InputModifiers::default();
            for argument in &args[3..] {
                match argument.as_str() {
                    "ctrl" => modifiers.ctrl = true,
                    "shift" => modifiers.shift = true,
                    "alt" => modifiers.alt = true,
                    "meta" => modifiers.meta = true,
                    _ => bail!("Unknown key modifier"),
                }
            }
            focused(class)?;
            input.key(&args[2], true, modifiers)?;
            thread::sleep(Duration::from_millis(35));
            input.key(&args[2], false, InputModifiers::default())?;
        }
        _ => bail!("Choose click or key"),
    }
    input.release_all()?;
    // Let the test surface consume the release before this short-lived
    // connection disappears. Production input stays connected for the grant.
    thread::sleep(Duration::from_millis(150));
    Ok(())
}
