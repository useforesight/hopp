mod api;
mod badge;
mod camera;
mod capabilities;
mod capture;
mod cards;
mod cli;
mod clipboard;
mod convert;
mod engine;
#[cfg(test)]
mod engine_tests;
mod frames;
mod ink;
mod ink_raster;
mod input_probe;
mod instance;
mod lock;
mod logger;
mod mcp;
mod media;
mod native_input;
mod paths;
mod pipewire_audio;
mod presence;
mod protocol;
mod scope;
#[cfg(test)]
mod scrawl_e2e;
mod screencast;
mod settings;
mod ui;
mod wake;

use anyhow::Result;

/// Whether the Hopp bar plugin is enabled in the Omarchy shell.
fn bar_plugin_active() -> bool {
    std::process::Command::new("omarchy-shell")
        .args(["shell", "listPlugins"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok())
        .and_then(|plugins| {
            plugins
                .as_array()?
                .iter()
                .find(|plugin| plugin["id"] == "local.hopp")
                .map(|plugin| plugin["enabled"] == true && plugin["active"] == true)
        })
        .unwrap_or(false)
}

fn main() -> Result<()> {
    use clap::Parser;
    // A panic's first backtrace is the one that says what went wrong; by default only a
    // second panic during cleanup printed one (3 November). Written straight to stderr:
    // a panic during unwinding aborts before the log thread would get to it.
    std::panic::set_hook(Box::new(|info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        eprintln!("hopp: panic: {info}\n{backtrace}");
    }));
    logger::capture_libraries();
    let cli = cli::Cli::parse();
    let instance_name = cli.instance.clone();
    if let Some(command) = cli.command()?
        && !cli::run(command, &instance_name)?
    {
        return Ok(());
    }
    let cli::Launch {
        background,
        doctor,
        input_test,
        smoke_test,
        marks_demo,
        link,
        ..
    } = cli.launch;
    if let Some(seconds) = marks_demo {
        ui::marks_demo(seconds);
        return Ok(());
    }
    if let Some(link) = &link {
        api::parse_auth_deep_link(link)?;
    }
    if input_test {
        return input_probe::run();
    }
    if doctor {
        let report = capabilities::Capabilities::probe();
        let mut json = serde_json::to_value(&report)?;
        json["input_route"] = report.input_route().into();
        json["layer_shell_available"] = report.layer_shell().into();
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }
    if !smoke_test && instance::forward_existing(&instance_name, link.as_deref())? {
        return Ok(());
    }
    // From here on this process is the long-running client: give it its own scope and
    // memory cap rather than living inside whatever launched it (often Hyprland itself).
    if !smoke_test {
        scope::enter();
    }
    let mut settings = settings::load()?;
    if let Some(link) = link {
        settings.token = Some(api::parse_auth_deep_link(&link)?);
    }
    let engine = engine::Engine::start(settings.clone());
    let instance = if smoke_test {
        None
    } else {
        Some(instance::listen(
            &instance_name,
            engine.commands.clone(),
            settings.server.clone(),
        )?)
    };
    ui::open(
        engine,
        settings,
        ui::Options {
            smoke_test,
            background,
            only: None,
            publish: instance.as_ref().map(|instance| {
                let feed = instance.feed();
                Box::new(move |state: &str| feed.publish(state)) as ui::Publisher
            }),
        },
    );
    Ok(())
}
