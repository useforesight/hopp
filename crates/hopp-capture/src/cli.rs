//! The commands that drive the running app. What can be refused is refused here, against the
//! state the app published, so that you're told why in your terminal.

use anyhow::{Result, bail};
use clap::Subcommand;
use serde_json::Value;

use crate::app_socket::{self, AppRequest};

#[derive(Subcommand, Clone, Debug, PartialEq, Eq)]
pub enum AppCommand {
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
    /// Start or stop drawing on the screen you're sharing.
    Draw,
    /// Pick up or put down the stamp tool on the screen you're sharing: hold down on a spot and
    /// say your note (while the call is captured), or click and type one.
    Stamp,
    /// Open Hopp, starting it if it isn't running.
    Open,
    /// Open Hopp's settings.
    Settings,
}

pub fn run(command: AppCommand) -> Result<()> {
    match command {
        AppCommand::Status { json } => {
            let state = app_socket::snapshot()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&state)?);
            } else {
                print!("{}", describe(&state));
            }
            Ok(())
        }
        AppCommand::Open | AppCommand::Settings if app_socket::snapshot().is_err() => start_app(),
        command => {
            let state = app_socket::snapshot()?;
            app_socket::request(&plan(&command, &state)?).map(|_| ())
        }
    }
}

/// What a command asks of the app, if it can be done now.
fn plan(command: &AppCommand, state: &Value) -> Result<AppRequest> {
    Ok(match command {
        AppCommand::Status { .. } => AppRequest::Snapshot,
        AppCommand::Open => AppRequest::Open,
        AppCommand::Settings => AppRequest::Settings,
        AppCommand::Call { who } => {
            signed_in(state)?;
            AppRequest::Call(teammate(state, who)?)
        }
        AppCommand::Join { room } => {
            signed_in(state)?;
            AppRequest::JoinRoom(room_id(state, room)?)
        }
        AppCommand::Accept | AppCommand::Decline => {
            if !state["ringing"].is_object() {
                bail!("Nobody is calling you");
            }
            if *command == AppCommand::Accept {
                AppRequest::Accept
            } else {
                AppRequest::Decline
            }
        }
        AppCommand::Leave => {
            in_call(state)?;
            AppRequest::Leave
        }
        AppCommand::Mute => {
            in_call(state)?;
            AppRequest::Microphone
        }
        AppCommand::Camera => {
            in_call(state)?;
            AppRequest::Camera
        }
        AppCommand::Share => {
            in_call(state)?;
            AppRequest::Share
        }
        AppCommand::Draw => {
            in_call(state)?;
            if state["call"]["sharing"] != true {
                bail!(
                    "From here you can only draw on a screen you're sharing. \
                     On someone else's, use the pencil in its window"
                );
            }
            AppRequest::Draw
        }
        AppCommand::Stamp => {
            in_call(state)?;
            if state["call"]["sharing"] != true {
                bail!(
                    "From here you can only stamp a screen you're sharing. \
                     On someone else's, use the stamp in its window"
                );
            }
            AppRequest::Stamp
        }
    })
}

fn signed_in(state: &Value) -> Result<()> {
    if state["signed_in"] != true {
        bail!("Sign in to Hopp first");
    }
    Ok(())
}

pub fn in_call(state: &Value) -> Result<()> {
    if !state["call"].is_object() {
        bail!("You're not in a call");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn start_app() -> Result<()> {
    let status = std::process::Command::new("/usr/bin/open")
        .args(["-b", "com.hopp.app"])
        .status()?;
    if !status.success() {
        bail!("Couldn't start Hopp");
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn start_app() -> Result<()> {
    bail!(app_socket::NOT_RUNNING)
}

/// The capture recording this call, if any. The app has this from the core, which is the
/// only one who knows: a capture that was stopped stays open in the store while its last
/// words finish.
pub fn capture_id(state: &Value) -> Option<&str> {
    state["capture"]["id"].as_str()
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

/// The state as a few readable lines.
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
        "Signed in as {}\n",
        state["user"]["name"].as_str().unwrap_or("?")
    ));
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
    if let Some(id) = capture_id(state) {
        out.push_str(&format!("Capturing {id}\n"));
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
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            "teammates": [{"name": "Mira", "online": true, "room": "Core"}],
            "rooms": [{"name": "Core", "members": ["Mira"]}],
            "call": {"room": "Core", "participants": [{"name": "Mira"}],
                     "microphone": true, "camera": false, "sharing": true},
            "capture": {"id": "c_1"},
            "ringing": null, "dialing": null,
        }));
        assert!(text.contains("Signed in as Alex"), "{text}");
        assert!(
            text.contains("In a call with Mira · mic on · sharing"),
            "{text}"
        );
        assert!(text.contains("Capturing c_1"), "{text}");
        assert!(text.contains("● Mira · in Core"), "{text}");
    }

    #[test]
    fn says_why_a_command_cant_be_done() -> Result<()> {
        let idle = json!({ "signed_in": true, "teammates": [{"id": "u1", "name": "Mira"}] });
        fn why(command: AppCommand, state: &Value) -> String {
            plan(&command, state).unwrap_err().to_string()
        }
        assert_eq!(why(AppCommand::Mute, &idle), "You're not in a call");
        assert_eq!(why(AppCommand::Leave, &idle), "You're not in a call");
        assert_eq!(why(AppCommand::Accept, &idle), "Nobody is calling you");
        assert_eq!(
            why(AppCommand::Call { who: "mira".into() }, &json!({})),
            "Sign in to Hopp first"
        );
        assert_eq!(
            plan(&AppCommand::Call { who: "mira".into() }, &idle)?,
            AppRequest::Call("u1".into())
        );

        let viewing = json!({ "call": { "sharing": false, "viewing": "Mira" } });
        assert!(why(AppCommand::Draw, &viewing).contains("a screen you're sharing"));
        let sharing = json!({ "call": { "sharing": true } });
        assert_eq!(plan(&AppCommand::Draw, &sharing)?, AppRequest::Draw);
        assert!(why(AppCommand::Stamp, &viewing).contains("a screen you're sharing"));
        assert_eq!(plan(&AppCommand::Stamp, &sharing)?, AppRequest::Stamp);

        let ringing = json!({ "ringing": { "name": "Mira", "kind": "call" } });
        assert_eq!(plan(&AppCommand::Decline, &ringing)?, AppRequest::Decline);
        Ok(())
    }
}
