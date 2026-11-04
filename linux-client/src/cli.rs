//! The `hopp` command line. With no subcommand it starts Hopp (or brings the running one
//! forward); subcommands talk to the running Hopp over its private instance socket.

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use serde_json::Value;

use crate::{instance, protocol::UiRequest};

#[derive(Parser)]
#[command(
    name = "hopp",
    version,
    about = "Pair programming with Hopp, from the Omarchy desktop.",
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    /// Which local Hopp to talk to, when running more than one.
    #[arg(long, global = true, default_value = "main", value_name = "NAME")]
    pub instance: String,
    #[command(flatten)]
    pub launch: Launch,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Options for starting Hopp itself.
#[derive(Args)]
pub struct Launch {
    /// Start without opening a window (the bar plugin and autostart use this).
    #[arg(long)]
    pub background: bool,
    /// Print what this desktop supports, as JSON.
    #[arg(long)]
    pub doctor: bool,
    /// Test virtual input in a window of its own.
    #[arg(long)]
    pub input_test: bool,
    /// Start, then exit after three seconds.
    #[arg(long)]
    pub smoke_test: bool,
    /// Show made-up teammates' cursors and ink over your screen for this many seconds.
    #[arg(long, value_name = "SECONDS", num_args = 0..=1, default_missing_value = "12")]
    pub marks_demo: Option<u64>,
    // Old spellings of the subcommands, kept so existing key bindings keep working.
    #[arg(long, hide = true)]
    pub peek: bool,
    #[arg(long, hide = true)]
    pub panel: bool,
    #[arg(long, hide = true)]
    pub settings: bool,
    #[arg(long, hide = true)]
    pub quit: bool,
    #[arg(long, hide = true)]
    pub revoke_control: bool,
    /// A hopp: sign-in link from the browser.
    #[arg(value_name = "LINK")]
    pub link: Option<String>,
}

#[derive(Subcommand, Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Show who's online and what's happening in your call.
    Status {
        /// Print the full state as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Call a teammate, or invite them into your call.
    Call {
        /// Name or id.
        who: String,
    },
    /// Join a room.
    Join {
        /// Name or id.
        room: String,
    },
    /// Answer the incoming call.
    Accept,
    /// Decline the incoming call.
    Decline,
    /// Leave the call.
    Leave,
    /// Mute or unmute your microphone.
    Mute,
    /// Turn your camera on or off.
    Camera,
    /// Start or stop sharing your screen.
    Share,
    /// Show or hide the whole shared screen (Super+H).
    Peek,
    /// Start or stop drawing: on a screen someone shares with you, or on your own while
    /// you share it (Super+Alt+D; Escape or right-click stops).
    Draw,
    /// Pick up or put down the stamp tool (Super+Alt+P): click a spot on the shared
    /// screen, then say (while the call is captured) or type a note; Enter finishes it.
    Stamp,
    /// Open Hopp in the bar, or its window if the bar plugin is off (Super+Alt+H).
    Panel,
    /// Open Hopp's settings.
    Settings,
    /// Take back remote control at once (Super+Ctrl+Esc).
    Revoke,
    /// Leave any call and quit Hopp.
    Quit,
    /// Captured calls: transcripts saved on this computer.
    Capture {
        #[command(subcommand)]
        command: crate::capture::commands::CaptureCommand,
    },
    /// Show a notice in Hopp, for agents and scripts. Prints the card's id.
    Notify {
        /// One line, the gist.
        #[arg(long)]
        title: String,
        /// More detail, a few lines at most.
        #[arg(long)]
        body: Option<String>,
        /// Who it is from, such as "Claude".
        #[arg(long)]
        sender: Option<String>,
        /// Go by itself after ten seconds.
        #[arg(long)]
        ephemeral: bool,
    },
    /// Ask a yes/no question in Hopp and wait for the answer. Exits 0 on accept, 1 on
    /// reject or close, 2 when the timeout passes (the card is then withdrawn).
    Ask {
        /// The question, one line.
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        sender: Option<String>,
        /// The accept button's label.
        #[arg(long, default_value = "Accept")]
        accept: String,
        /// The reject button's label.
        #[arg(long, default_value = "Reject")]
        reject: String,
        /// Give up after this many seconds; wait as long as it takes when unset.
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Serve captured calls to coding agents over MCP (stdin/stdout).
    ///
    /// Claude Code: claude mcp add hopp -- hopp mcp
    Mcp,
}

impl Cli {
    /// The subcommand to run, including the old `--peek` style flags. `None` starts Hopp.
    pub fn command(&self) -> Result<Option<Command>> {
        let launch = &self.launch;
        let legacy = [
            (launch.peek, Command::Peek),
            (launch.panel, Command::Panel),
            (launch.settings, Command::Settings),
            (launch.quit, Command::Quit),
            (launch.revoke_control, Command::Revoke),
        ]
        .into_iter()
        .filter_map(|(set, command)| set.then_some(command))
        .collect::<Vec<_>>();
        if legacy.len() > 1 || (!legacy.is_empty() && self.command.is_some()) {
            bail!("Choose one command at a time");
        }
        let command = self.command.clone().or(legacy.into_iter().next());
        if command.is_some()
            && (launch.background
                || launch.doctor
                || launch.input_test
                || launch.smoke_test
                || launch.link.is_some())
        {
            bail!("Commands accept only --instance NAME");
        }
        if let Some(link) = &launch.link
            && !link.starts_with("hopp:")
        {
            bail!("Unknown command '{link}'; see hopp --help");
        }
        Ok(command)
    }
}

/// What a command asks of the running Hopp, and whether Hopp should start for it.
enum Plan {
    /// Ask the running Hopp; if none is running, report that (or say nothing, for the
    /// key-binding commands).
    Request(UiRequest, Missing),
    /// Starts Hopp when nothing is running.
    RequestOrStart(UiRequest),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Missing {
    Quiet,
    Error,
}

/// Run a subcommand. Returns true when Hopp should be started instead (settings or
/// panel with nothing running).
pub fn run(command: Command, instance_name: &str) -> Result<bool> {
    let plan = match command {
        Command::Status { json } => {
            let state = running_state(instance_name)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&state)?);
            } else {
                print!("{}", describe(&state));
            }
            return Ok(false);
        }
        Command::Revoke => {
            instance::revoke_existing(instance_name)?;
            return Ok(false);
        }
        Command::Capture { command } => {
            crate::capture::commands::run(command, instance_name)?;
            return Ok(false);
        }
        Command::Mcp => {
            crate::mcp::serve(instance_name)?;
            return Ok(false);
        }
        Command::Notify {
            title,
            body,
            sender,
            ephemeral,
        } => {
            let id = crate::cards::new_id();
            crate::cards::post(
                instance_name,
                crate::cards::CardRequest {
                    id: id.clone(),
                    title,
                    body,
                    sender,
                    question: None,
                    ephemeral,
                },
            )?;
            println!("{id}");
            return Ok(false);
        }
        Command::Ask {
            title,
            body,
            sender,
            accept,
            reject,
            timeout,
        } => {
            let code = crate::cards::ask(
                instance_name,
                crate::cards::CardRequest {
                    id: crate::cards::new_id(),
                    title,
                    body,
                    sender,
                    question: Some(crate::cards::Question { accept, reject }),
                    ephemeral: false,
                },
                timeout.map(std::time::Duration::from_secs),
            )?;
            std::process::exit(code);
        }
        Command::Panel => {
            if crate::bar_plugin_active() {
                let toggled = std::process::Command::new("omarchy-shell")
                    .args(["shell", "toggle", "local.hopp"])
                    .status()
                    .is_ok_and(|status| status.success());
                if toggled {
                    return Ok(false);
                }
            }
            Plan::RequestOrStart(UiRequest::Open)
        }
        Command::Call { who } => {
            let state = running_state(instance_name)?;
            Plan::Request(UiRequest::Call(teammate(&state, &who)?), Missing::Error)
        }
        Command::Join { room } => {
            let state = running_state(instance_name)?;
            Plan::Request(UiRequest::JoinRoom(room_id(&state, &room)?), Missing::Error)
        }
        Command::Accept => Plan::Request(UiRequest::Accept, Missing::Error),
        Command::Decline => Plan::Request(UiRequest::Decline, Missing::Error),
        Command::Leave => Plan::Request(UiRequest::Leave, Missing::Error),
        Command::Mute => Plan::Request(UiRequest::Microphone, Missing::Error),
        Command::Camera => Plan::Request(UiRequest::Camera, Missing::Error),
        Command::Share => Plan::Request(UiRequest::Share, Missing::Error),
        Command::Peek => Plan::Request(UiRequest::Peek, Missing::Quiet),
        Command::Draw => Plan::Request(UiRequest::Draw, Missing::Error),
        Command::Stamp => Plan::Request(UiRequest::Stamp, Missing::Error),
        Command::Quit => Plan::Request(UiRequest::Quit, Missing::Quiet),
        Command::Settings => Plan::RequestOrStart(UiRequest::Settings),
    };
    match plan {
        Plan::Request(request, missing) => {
            if !instance::request_existing(instance_name, request)? && missing == Missing::Error {
                bail!("Hopp isn't running");
            }
            Ok(false)
        }
        Plan::RequestOrStart(request) => Ok(!instance::request_existing(instance_name, request)?),
    }
}

fn running_state(instance_name: &str) -> Result<Value> {
    let Some(state) = instance::snapshot(instance_name)? else {
        bail!("Hopp isn't running");
    };
    Ok(serde_json::from_str(&state)?)
}

fn named<'a>(items: &'a [Value], wanted: &str, what: &str) -> Result<&'a Value> {
    let text = |item: &Value, key: &str| item[key].as_str().unwrap_or_default().to_lowercase();
    let wanted_lower = wanted.to_lowercase();
    if let Some(item) = items.iter().find(|item| item["id"] == wanted) {
        return Ok(item);
    }
    let exact: Vec<_> = items
        .iter()
        .filter(|item| text(item, "name") == wanted_lower)
        .collect();
    let candidates = if exact.is_empty() {
        items
            .iter()
            .filter(|item| {
                let name = text(item, "name");
                name.starts_with(&wanted_lower)
                    || name
                        .split_whitespace()
                        .any(|word| word.starts_with(&wanted_lower))
            })
            .collect()
    } else {
        exact
    };
    match candidates.as_slice() {
        [one] => Ok(one),
        [] => bail!("No {what} called '{wanted}'"),
        many => bail!(
            "'{wanted}' could be {}",
            many.iter()
                .map(|item| text(item, "name"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn teammate(state: &Value, who: &str) -> Result<String> {
    let teammates = state["teammates"].as_array().cloned().unwrap_or_default();
    Ok(named(&teammates, who, "teammate")?["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned())
}

fn room_id(state: &Value, room: &str) -> Result<String> {
    let rooms = state["rooms"].as_array().cloned().unwrap_or_default();
    Ok(named(&rooms, room, "room")?["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned())
}

/// The snapshot as a few readable lines.
pub fn describe(state: &Value) -> String {
    let mut out = String::new();
    let names = |list: &Value| {
        list.as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().or_else(|| item["name"].as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    };
    if state["signed_in"] != true {
        out.push_str("Not signed in\n");
        return out;
    }
    out.push_str(&format!(
        "Signed in as {}",
        state["user"]["name"].as_str().unwrap_or("?")
    ));
    if let Some(status) = state["status"].as_str().filter(|status| !status.is_empty()) {
        out.push_str(&format!(" · {status}"));
    }
    out.push('\n');
    if let Some(ringing) = state["ringing"]["name"].as_str() {
        let kind = if state["ringing"]["kind"] == "invite" {
            "invites you"
        } else {
            "is calling"
        };
        out.push_str(&format!("{ringing} {kind} · hopp accept / hopp decline\n"));
    }
    if let Some(dialing) = state["dialing"]["name"].as_str() {
        out.push_str(&format!("Calling {dialing}…\n"));
    }
    let call = &state["call"];
    if call.is_object() {
        let with = names(&call["participants"]);
        let mut parts = vec![if with.is_empty() {
            format!("In {}", call["room"].as_str().unwrap_or("a call"))
        } else {
            format!("In a call with {with}")
        }];
        if let Some(started) = call["started"].as_u64() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_secs())
                .unwrap_or(started);
            let minutes = now.saturating_sub(started) / 60;
            parts.push(format!("{minutes} min"));
        }
        parts.push(
            if call["microphone"] == true {
                "mic on"
            } else {
                "muted"
            }
            .into(),
        );
        if call["camera"] == true {
            parts.push("camera on".into());
        }
        if call["sharing"] == true {
            parts.push("sharing".into());
        }
        if let Some(viewing) = call["viewing"].as_str() {
            parts.push(format!("viewing {viewing}"));
        }
        out.push_str(&parts.join(" · "));
        out.push('\n');
    }
    if let Some(teammates) = state["teammates"].as_array()
        && !teammates.is_empty()
    {
        out.push_str("\nTeammates\n");
        for teammate in teammates {
            let mark = if teammate["online"] == true {
                "●"
            } else {
                "○"
            };
            let name = teammate["name"].as_str().unwrap_or("?");
            match teammate["room"].as_str() {
                Some(room) => out.push_str(&format!("  {mark} {name} · in {room}\n")),
                None => out.push_str(&format!("  {mark} {name}\n")),
            }
        }
    }
    if let Some(rooms) = state["rooms"].as_array()
        && !rooms.is_empty()
    {
        out.push_str("\nRooms\n");
        for room in rooms {
            let members = names(&room["members"]);
            let name = room["name"].as_str().unwrap_or("?");
            if members.is_empty() {
                out.push_str(&format!("  {name}\n"));
            } else {
                out.push_str(&format!("  {name} · {members}\n"));
            }
        }
    }
    for error in state["errors"].as_array().into_iter().flatten() {
        if let Some(error) = error.as_str() {
            out.push_str(&format!("! {error}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(arguments: &[&str]) -> Result<Option<Command>> {
        Cli::try_parse_from(std::iter::once("hopp").chain(arguments.iter().copied()))?.command()
    }

    #[test]
    fn old_flags_still_work_for_existing_bindings() -> Result<()> {
        assert_eq!(parse(&["--peek"])?, Some(Command::Peek));
        assert_eq!(parse(&["--revoke-control"])?, Some(Command::Revoke));
        assert_eq!(
            parse(&["--quit", "--instance", "publisher"])?,
            Some(Command::Quit)
        );
        assert!(parse(&["--peek", "--quit"]).is_err());
        assert!(parse(&["--peek", "--background"]).is_err());
        Ok(())
    }

    #[test]
    fn subcommands_links_and_plain_launches() -> Result<()> {
        assert_eq!(parse(&[])?, None);
        assert_eq!(parse(&["--background"])?, None);
        assert_eq!(parse(&["hopp:///authenticate?token=x"])?, None);
        assert_eq!(
            parse(&["call", "mira"])?,
            Some(Command::Call { who: "mira".into() })
        );
        assert_eq!(
            parse(&["status", "--json", "--instance", "viewer"])?,
            Some(Command::Status { json: true })
        );
        assert!(parse(&["nonsense"]).is_err());
        Ok(())
    }

    #[test]
    fn finds_teammates_and_rooms_by_name_or_id() -> Result<()> {
        let state = json!({
            "teammates": [
                {"id": "u1", "name": "Mira Jones"},
                {"id": "u2", "name": "Jonas Berg"},
                {"id": "u3", "name": "Ada Ive"},
            ],
            "rooms": [{"id": "r1", "name": "Core"}],
        });
        assert_eq!(teammate(&state, "mira")?, "u1");
        assert_eq!(teammate(&state, "u2")?, "u2");
        assert_eq!(teammate(&state, "Ada ive")?, "u3");
        assert_eq!(teammate(&state, "berg")?, "u2");
        assert!(teammate(&state, "jon").is_err(), "ambiguous");
        assert!(teammate(&state, "zed").is_err());
        assert_eq!(room_id(&state, "core")?, "r1");
        Ok(())
    }

    #[test]
    fn describes_a_call() {
        let text = describe(&json!({
            "signed_in": true,
            "user": {"name": "Alex"},
            "status": "",
            "teammates": [{"name": "Mira", "online": true, "room": "Core"}],
            "rooms": [{"name": "Core", "members": ["Mira"]}],
            "call": {"room": "Core", "participants": [{"name": "Mira"}],
                     "microphone": true, "camera": false, "sharing": true},
            "ringing": null, "dialing": null, "errors": [],
        }));
        assert!(text.contains("Signed in as Alex"));
        assert!(text.contains("In a call with Mira · mic on · sharing"));
        assert!(text.contains("● Mira · in Core"));
    }
}
