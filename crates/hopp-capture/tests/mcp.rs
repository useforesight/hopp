//! `hopp mcp` as an agent meets it: the real program, JSON lines on its stdin and stdout, and
//! a real store in a throwaway home.

use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use hopp_capture::{
    app_socket::{AppReply, AppRequest, serve},
    store::{Kind, NewLine, Store, now_ms},
};
use serde_json::{Value, json};

struct Agent {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Agent {
    fn start(home: &Path) -> Result<Self> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_hopp"))
            .arg("mcp")
            .env("HOPP_CAPTURE_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().context("stdout")?);
        Ok(Self {
            child,
            stdin,
            stdout,
            next_id: 0,
        })
    }

    fn send(&mut self, method: &str, params: Value) -> Result<u64> {
        self.next_id += 1;
        let message =
            json!({ "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params });
        let stdin = self.stdin.as_mut().context("hung up")?;
        writeln!(stdin, "{message}")?;
        stdin.flush()?;
        Ok(self.next_id)
    }

    fn reply(&mut self, id: u64) -> Result<Value> {
        let mut line = String::new();
        self.stdout.read_line(&mut line)?;
        let reply: Value = serde_json::from_str(&line)?;
        assert_eq!(reply["id"], id, "{line}");
        Ok(reply["result"].clone())
    }

    fn start_tool(&mut self, name: &str, arguments: Value) -> Result<u64> {
        self.send(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
    }

    fn tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        let id = self.start_tool(name, arguments)?;
        self.reply(id)
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn text(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn line(t_ms: i64, kind: Kind, who: Option<&str>, text: &str) -> NewLine {
    NewLine {
        t_ms,
        end_ms: Some(t_ms + 500),
        kind,
        speaker_id: who.map(str::to_lowercase),
        speaker: who.map(str::to_owned),
        text: text.into(),
    }
}

/// A finished call with one picture and a live one, as a recorder leaves them.
fn seeded_home() -> Result<(tempfile::TempDir, Store)> {
    let home = tempfile::tempdir()?;
    let store = Store::open(&home.path().join("capture/index.db"))?;
    store.begin_call("c_ended", "Core", 1_000, "parakeet-en")?;
    store.add_line(
        "c_ended",
        &line(
            2_000,
            Kind::Speech,
            Some("Mira"),
            "The flood starts on a profile switch.",
        ),
    )?;
    store.add_line(
        "c_ended",
        &line(3_000, Kind::Frame, None, "#1 Mira drew a circle"),
    )?;
    store.set_participants("c_ended", &["Mira".into()])?;
    store.end_call("c_ended", 61_000)?;

    let frames = home.path().join("capture/frames/c_ended");
    std::fs::create_dir_all(&frames)?;
    std::fs::write(
        frames.join("frame-001.json"),
        json!({
            "number": 1, "t_ms": 2_000, "width": 40, "height": 40, "trigger": "marks",
            "caption": "#1 Mira drew a circle",
            "marks": [{ "by": "Mira", "shape": "circle", "bounds": [0.25, 0.25, 0.5, 0.5], "colour": "pink" }],
            "ink": [{ "by": "Mira", "shape": "circle", "bounds": [0.25, 0.25, 0.5, 0.5], "colour": "pink" }],
        })
        .to_string(),
    )?;
    std::fs::write(frames.join("frame-001.webp"), b"small copy")?;
    image::RgbaImage::new(40, 40).save(frames.join("frame-001.png"))?;
    Ok((home, store))
}

#[test]
fn agents_negotiate_and_cant_start_a_capture() -> Result<()> {
    let (home, _store) = seeded_home()?;
    let mut agent = Agent::start(home.path())?;
    let id = agent.send(
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } }),
    )?;
    assert_eq!(agent.reply(id)?["serverInfo"]["name"], "hopp");
    let id = agent.send("tools/list", json!({}))?;
    let tools = agent.reply(id)?;
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .context("tools")?
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&"capture_follow"), "{names:?}");
    assert!(
        !names.iter().any(|name| name.contains("start")),
        "{names:?}"
    );
    Ok(())
}

#[test]
fn agents_can_ask_about_the_call_with_or_without_the_app() -> Result<()> {
    let (home, _store) = seeded_home()?;
    let mut agent = Agent::start(home.path())?;
    let nothing = agent.tool("call_status", json!({}))?;
    assert_eq!(text(&nothing), "Hopp isn't running.", "{nothing}");
    assert_ne!(nothing["isError"], true, "{nothing}");

    let socket = home.path().join("run/hopp.sock");
    let listening = socket.clone();
    std::thread::spawn(move || {
        let _ = serve(&listening, |_| {
            AppReply::state(json!({
                "signed_in": true,
                "user": { "name": "Alex" },
                "call": { "participants": [{ "name": "Mira" }], "microphone": true },
            }))
        });
    });
    while !socket.exists() {
        std::thread::sleep(Duration::from_millis(5));
    }
    let status = text(&agent.tool("call_status", json!({}))?);
    assert!(status.contains("In a call with Mira"), "{status}");
    Ok(())
}

#[test]
fn agents_ask_through_cards_and_hear_the_answer() -> Result<()> {
    let (home, _store) = seeded_home()?;
    let mut agent = Agent::start(home.path())?;
    let nobody = agent.tool("notify_user", json!({ "title": "Fixed" }))?;
    assert_eq!(nobody["isError"], true, "{nobody}");
    assert_eq!(text(&nobody), "Hopp isn't running", "{nobody}");

    // "Delete?" is answered at once; the others stay up for the agent to wait on.
    let socket = home.path().join("run/hopp.sock");
    let listening = socket.clone();
    std::thread::spawn(move || {
        let cards = Arc::new(Mutex::new(json!([])));
        let _ = serve(&listening, move |request| match request {
            AppRequest::Snapshot => AppReply::state(json!({ "cards": *cards.lock().unwrap() })),
            AppRequest::Card(card) => {
                let answer = if card.title == "Delete?" {
                    "rejected"
                } else {
                    "showing"
                };
                let mut cards = cards.lock().unwrap();
                let list = cards.as_array_mut().unwrap();
                list.push(json!({ "id": card.id, "state": answer, "title": card.title }));
                AppReply::done()
            }
            AppRequest::CancelCard(id) => {
                let mut cards = cards.lock().unwrap();
                for card in cards.as_array_mut().unwrap() {
                    if card["id"] == id.as_str() {
                        card["state"] = json!("cancelled");
                    }
                }
                AppReply::done()
            }
            _ => AppReply::refused("Not for agents"),
        });
    });
    while !socket.exists() {
        std::thread::sleep(Duration::from_millis(5));
    }

    let told: Value = serde_json::from_str(&text(&agent.tool(
        "notify_user",
        json!({ "title": "Fixed", "ephemeral": true }),
    )?))?;
    assert_eq!(told["state"], "showing");
    assert!(told["card"].as_str().unwrap().starts_with("card_"));

    let no: Value = serde_json::from_str(&text(&agent.tool(
        "request_user_decision",
        json!({ "title": "Delete?", "timeout_seconds": 5 }),
    )?))?;
    assert_eq!(no["state"], "rejected");

    let waiting: Value = serde_json::from_str(&text(&agent.tool(
        "request_user_decision",
        json!({ "title": "Deploy?", "timeout_seconds": 0 }),
    )?))?;
    assert_eq!(waiting["state"], "showing", "no answer yet");
    let still: Value = serde_json::from_str(&text(&agent.tool(
        "wait_for_decision",
        json!({ "card": waiting["card"], "timeout_seconds": 0 }),
    )?))?;
    assert_eq!(still["state"], "showing");
    let taken_back: Value = serde_json::from_str(&text(
        &agent.tool("cancel_card", json!({ "card": waiting["card"] }))?,
    ))?;
    assert_eq!(taken_back["state"], "cancelled");
    let finally: Value = serde_json::from_str(&text(&agent.tool(
        "wait_for_decision",
        json!({ "card": waiting["card"], "timeout_seconds": 0 }),
    )?))?;
    assert_eq!(finally["state"], "cancelled");

    let unknown = agent.tool(
        "wait_for_decision",
        json!({ "card": "card_none", "timeout_seconds": 5 }),
    )?;
    assert_eq!(unknown["isError"], true, "{unknown}");
    Ok(())
}

#[test]
fn agents_read_what_was_said_and_shown() -> Result<()> {
    let (home, _store) = seeded_home()?;
    let mut agent = Agent::start(home.path())?;

    let listed = text(&agent.tool("capture_list", json!({}))?);
    assert!(listed.contains("c_ended"), "{listed}");

    let found = text(&agent.tool("capture_search", json!({ "query": "profile" }))?);
    assert!(found.contains("c_ended"), "{found}");

    let read = text(&agent.tool("capture_transcript", json!({ "id": "c_ended" }))?);
    assert!(read.contains("The flood starts"), "{read}");

    let frame = agent.tool("capture_frame", json!({ "id": "c_ended" }))?;
    assert!(text(&frame).contains("Mira circle in pink"), "{frame}");
    assert_eq!(frame["content"][1]["type"], "image");
    assert_eq!(frame["content"][1]["mimeType"], "image/webp");

    let zoomed = agent.tool(
        "capture_frame",
        json!({ "id": "c_ended", "frame": 1, "zoom": true }),
    )?;
    assert!(text(&zoomed).contains("Zoomed to the marks"), "{zoomed}");

    let missing = agent.tool("capture_frame", json!({ "id": "c_ended", "frame": 9 }))?;
    assert_eq!(missing["isError"], true, "{missing}");

    let annotated = agent.tool(
        "capture_annotate",
        json!({ "id": "c_ended", "title": "The flood", "summary": "Fixed." }),
    )?;
    assert!(text(&annotated).contains("Updated c_ended"), "{annotated}");
    let read = text(&agent.tool("capture_transcript", json!({ "id": "c_ended" }))?);
    assert!(read.contains("# The flood"), "{read}");
    let copy = std::fs::read_to_string(home.path().join("capture/c_ended.md"))?;
    assert!(copy.contains("# The flood"), "{copy}");
    Ok(())
}

#[test]
fn a_follow_holds_nothing_up_and_ends_when_the_agent_hangs_up() -> Result<()> {
    let (home, store) = seeded_home()?;
    store.begin_call("c_live", "Core", now_ms(), "parakeet-en")?;
    let mut agent = Agent::start(home.path())?;
    agent.start_tool("capture_follow", json!({ "timeout_seconds": 300 }))?;
    std::thread::sleep(Duration::from_millis(300));

    let id = agent.send("ping", json!({}))?;
    agent.reply(id)?;
    let listed = text(&agent.tool("capture_list", json!({}))?);
    assert!(listed.contains("c_ended"), "{listed}");

    agent.stdin = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = agent.child.try_wait()? {
            break status;
        }
        assert!(Instant::now() < deadline, "still running after hanging up");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "{status}");
    Ok(())
}

#[test]
fn an_agent_waiting_for_its_name_hears_nothing_until_then() -> Result<()> {
    let (home, store) = seeded_home()?;
    store.begin_call("c_live", "Core", now_ms(), "parakeet-en")?;
    store.add_line(
        "c_live",
        &line(500, Kind::Speech, Some("Mira"), "Before the agent came."),
    )?;
    let mut agent = Agent::start(home.path())?;
    let waited = agent.tool(
        "capture_follow",
        json!({ "wake_phrase": "hey claude", "timeout_seconds": 1 }),
    )?;
    let body: Value = serde_json::from_str(&text(&waited))?;
    assert_eq!(body["timed_out"], true, "{body}");
    assert_eq!(body["lines"], json!([]), "{body}");
    assert_eq!(waited["content"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["after_line"], store.latest_rev("c_live")?);
    Ok(())
}

#[test]
fn an_agent_follows_a_call_until_someone_wakes_it() -> Result<()> {
    let (home, store) = seeded_home()?;
    // Started just now: nobody holds its lock here, and an older call would be an orphan.
    let now = now_ms();
    store.begin_call("c_live", "Core", now, "parakeet-en")?;
    store.add_line(
        "c_live",
        &line(500, Kind::Speech, Some("Mira"), "Before the agent came."),
    )?;
    let mut agent = Agent::start(home.path())?;

    let waiting = agent.start_tool(
        "capture_follow",
        json!({ "wake_phrase": "hey claude", "timeout_seconds": 20 }),
    )?;
    std::thread::sleep(Duration::from_millis(800));
    store.add_line(
        "c_live",
        &line(1_000, Kind::Speech, Some("Mira"), "Let's look at the logs."),
    )?;
    store.add_line(
        "c_live",
        &line(
            2_000,
            Kind::Speech,
            Some("Mira"),
            "Hey Claude, what did cpal do?",
        ),
    )?;
    let woken: Value = serde_json::from_str(&text(&agent.reply(waiting)?))?;
    assert_eq!(woken["call"], "c_live");
    assert_eq!(woken["ended"], false);
    assert_eq!(woken["timed_out"], false);
    let said = woken["lines"].to_string();
    assert!(said.contains("Hey Claude"), "{said}");
    assert!(said.contains("look at the logs"), "{said}");
    assert!(
        said.contains("Before the agent"),
        "with what led up to it: {said}"
    );

    // Carrying on from its cursor, it hears that the call ended, not an error.
    store.end_call("c_live", now + 10_000)?;
    let ended = agent.tool(
        "capture_follow",
        json!({ "after_line": woken["after_line"], "timeout_seconds": 5 }),
    )?;
    let ended: Value = serde_json::from_str(&text(&ended))?;
    assert_eq!(ended["ended"], true, "{ended}");
    assert_eq!(ended["call"], "c_live");
    Ok(())
}
