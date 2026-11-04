//! `hopp` as you meet it in a terminal: the real program, against an app that's only its socket.

use std::{
    path::Path,
    process::{Command, Output},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use hopp_capture::{
    app_socket::{AppReply, AppRequest, serve},
    cards::CardState,
    store::{Store, now_ms},
};
use serde_json::{Value, json};

fn hopp(home: &Path, arguments: &[&str]) -> Result<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_hopp"))
        .args(arguments)
        .env("HOPP_CAPTURE_HOME", home)
        .output()?)
}

fn said(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn complained(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// An app with this state, saying yes to everything it's asked after that.
fn running_app(home: &Path, state: Value) -> Arc<Mutex<Vec<AppRequest>>> {
    running_app_doing(home, state, |_, _| {})
}

fn running_app_doing(
    home: &Path,
    state: Value,
    does: impl Fn(&AppRequest, Arc<Mutex<Value>>) + Send + 'static,
) -> Arc<Mutex<Vec<AppRequest>>> {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let told = asked.clone();
    let state = Arc::new(Mutex::new(state));
    let socket = home.join("run/hopp.sock");
    let listening = socket.clone();
    std::thread::spawn(move || {
        let _ = serve(&listening, |request| {
            if request == AppRequest::Snapshot {
                return AppReply::state(state.lock().unwrap().clone());
            }
            does(&request, state.clone());
            told.lock().unwrap().push(request);
            AppReply::done()
        });
    });
    let started = Instant::now();
    while !socket.exists() {
        assert!(started.elapsed() < Duration::from_secs(5), "never listened");
        std::thread::sleep(Duration::from_millis(5));
    }
    asked
}

/// An app that answers questions the way a person would, once they've seen the card.
fn app_that_answers(home: &Path, answer: Option<CardState>) -> Arc<Mutex<Vec<AppRequest>>> {
    running_app_doing(home, json!({ "signed_in": true }), move |request, state| {
        let mut state = state.lock().unwrap();
        match request {
            AppRequest::Card(card) => {
                state["cards"] =
                    json!([{ "id": card.id, "state": answer.unwrap_or(CardState::Showing) }]);
            }
            AppRequest::CancelCard(id) => {
                state["cards"] = json!([{ "id": id, "state": CardState::Cancelled }]);
            }
            _ => {}
        }
    })
}

fn in_a_call() -> Value {
    json!({
        "signed_in": true,
        "user": { "name": "Alex" },
        "ringing": null,
        "dialing": null,
        "call": {
            "room": null,
            "participants": [{ "name": "Mira Jones" }],
            "started": 1000,
            "microphone": true,
            "camera": false,
            "sharing": false,
            "viewing": null,
        },
        "teammates": [
            { "id": "u1", "name": "Mira Jones", "online": true, "room": "a call" },
            { "id": "u2", "name": "Jonas Berg", "online": false, "room": null },
        ],
        "rooms": [{ "id": "r1", "name": "Core", "members": [] }],
    })
}

#[test]
fn without_the_app_it_says_so() -> Result<()> {
    let home = tempfile::tempdir()?;
    for command in [vec!["status"], vec!["mute"], vec!["call", "mira"]] {
        let output = hopp(home.path(), &command)?;
        assert!(!output.status.success(), "{command:?}");
        assert!(
            complained(&output).contains("Hopp isn't running"),
            "{command:?}: {}",
            complained(&output)
        );
    }
    Ok(())
}

#[test]
fn status_says_what_the_app_says() -> Result<()> {
    let home = tempfile::tempdir()?;
    running_app(home.path(), in_a_call());

    let text = said(&hopp(home.path(), &["status"])?);
    assert!(text.contains("Signed in as Alex"), "{text}");
    assert!(text.contains("In a call with Mira Jones"), "{text}");
    assert!(text.contains("● Mira Jones · in a call"), "{text}");
    assert!(text.contains("○ Jonas Berg"), "{text}");
    assert!(!text.contains("Capturing"), "{text}");

    let state: Value = serde_json::from_str(&said(&hopp(home.path(), &["status", "--json"])?))?;
    assert_eq!(state["call"]["microphone"], true);
    Ok(())
}

#[test]
fn commands_reach_the_app_by_name() -> Result<()> {
    let home = tempfile::tempdir()?;
    let asked = running_app(home.path(), in_a_call());

    for command in [
        vec!["call", "mira"],
        vec!["join", "core"],
        vec!["leave"],
        vec!["mute"],
        vec!["camera"],
        vec!["share"],
        vec!["open"],
        vec!["settings"],
        vec!["capture", "window"],
    ] {
        let output = hopp(home.path(), &command)?;
        assert!(
            output.status.success(),
            "{command:?}: {}",
            complained(&output)
        );
    }
    assert_eq!(
        *asked.lock().unwrap(),
        vec![
            AppRequest::Call("u1".into()),
            AppRequest::JoinRoom("r1".into()),
            AppRequest::Leave,
            AppRequest::Microphone,
            AppRequest::Camera,
            AppRequest::Share,
            AppRequest::Open,
            AppRequest::Settings,
            AppRequest::CaptureWindow(None),
        ]
    );
    Ok(())
}

#[test]
fn what_cant_be_done_is_refused_here_and_never_asked() -> Result<()> {
    let home = tempfile::tempdir()?;
    let asked = running_app(
        home.path(),
        json!({
            "signed_in": true,
            "user": { "name": "Alex" },
            "ringing": null,
            "dialing": null,
            "call": null,
            "teammates": [
                { "id": "u1", "name": "Mira Jones", "online": true, "room": null },
                { "id": "u2", "name": "Mira Berg", "online": true, "room": null },
            ],
            "rooms": [],
        }),
    );

    for (command, why) in [
        (vec!["mute"], "You're not in a call"),
        (vec!["leave"], "You're not in a call"),
        (vec!["draw"], "You're not in a call"),
        (vec!["accept"], "Nobody is calling you"),
        (vec!["call", "zed"], "No teammate called 'zed'"),
        (vec!["call", "mira"], "'mira' could be"),
        (vec!["join", "core"], "No room called 'core'"),
        (vec!["capture", "start"], "Join a call first"),
        (vec!["capture", "bookmark"], "Nothing is being captured"),
        (
            vec!["capture", "window", "live"],
            "Nothing is being captured",
        ),
        (
            vec!["capture", "window", "c_gone"],
            "No captured call 'c_gone'",
        ),
    ] {
        let output = hopp(home.path(), &command)?;
        assert!(!output.status.success(), "{command:?}");
        assert!(
            complained(&output).contains(why),
            "{command:?}: {}",
            complained(&output)
        );
    }
    assert_eq!(*asked.lock().unwrap(), vec![]);
    Ok(())
}

#[test]
fn a_capture_is_started_opened_and_marked() -> Result<()> {
    let home = tempfile::tempdir()?;
    // The recorder appears a moment after it's asked for, as the real one does.
    let database = home.path().join("capture/index.db");
    let store = Store::open(&database)?;
    store.begin_call("c_live", "Core", now_ms(), "parakeet-en")?;
    let asked = running_app_doing(home.path(), in_a_call(), |request, state| match request {
        AppRequest::CaptureStart => {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                state.lock().unwrap()["capture"] = json!({ "id": "c_live" });
            });
        }
        AppRequest::CaptureStop => state.lock().unwrap()["capture"] = Value::Null,
        _ => {}
    });

    let nothing = hopp(home.path(), &["capture", "stop"])?;
    assert!(
        said(&nothing).contains("Nothing is being captured"),
        "{}",
        said(&nothing)
    );

    let started = hopp(home.path(), &["capture", "start"])?;
    assert!(started.status.success(), "{}", complained(&started));
    assert!(
        said(&started).contains("Capturing (c_live)"),
        "{}",
        said(&started)
    );

    let again = hopp(home.path(), &["capture", "start"])?;
    assert!(
        said(&again).contains("Already capturing (c_live)"),
        "{}",
        said(&again)
    );
    let text = said(&hopp(home.path(), &["status"])?);
    assert!(text.contains("Capturing c_live"), "{text}");

    for command in [
        vec!["capture", "bookmark", "the", "flood"],
        vec!["capture", "bookmark"],
        vec!["capture", "window", "live"],
        vec!["capture", "stop"],
    ] {
        let output = hopp(home.path(), &command)?;
        assert!(
            output.status.success(),
            "{command:?}: {}",
            complained(&output)
        );
    }

    // Stopped, its last words are still finishing and its row is still open: this call isn't
    // being captured any more, whatever the store says, and it can be started again.
    assert!(store.call("current")?.is_some());
    let text = said(&hopp(home.path(), &["status"])?);
    assert!(!text.contains("Capturing"), "{text}");
    let restarted = hopp(home.path(), &["capture", "start"])?;
    assert!(
        said(&restarted).contains("Capturing (c_live)"),
        "{}",
        said(&restarted)
    );

    assert_eq!(
        *asked.lock().unwrap(),
        vec![
            AppRequest::CaptureStart,
            AppRequest::CaptureBookmark(Some("the flood".into())),
            AppRequest::CaptureBookmark(None),
            AppRequest::CaptureWindow(Some("c_live".into())),
            AppRequest::CaptureStop,
            AppRequest::CaptureStart,
        ]
    );
    Ok(())
}

#[test]
fn a_capture_that_never_appears_is_reported() -> Result<()> {
    let home = tempfile::tempdir()?;
    running_app(home.path(), in_a_call());
    let output = hopp(home.path(), &["capture", "start"])?;
    assert!(!output.status.success());
    assert!(
        complained(&output).contains("didn't start capturing"),
        "{}",
        complained(&output)
    );
    Ok(())
}

#[test]
fn ask_says_what_was_answered_in_its_exit_code() -> Result<()> {
    for (answer, word, code) in [
        (CardState::Accepted, "accepted", 0),
        (CardState::Rejected, "rejected", 1),
        (CardState::Dismissed, "dismissed", 1),
    ] {
        let home = tempfile::tempdir()?;
        let asked = app_that_answers(home.path(), Some(answer));
        let output = hopp(
            home.path(),
            &[
                "ask", "--title", "Deploy?", "--sender", "Claude", "--accept", "Ship it",
            ],
        )?;
        assert_eq!(output.status.code(), Some(code), "{}", complained(&output));
        assert_eq!(said(&output).trim(), word);
        let asked = asked.lock().unwrap();
        let [AppRequest::Card(card)] = asked.as_slice() else {
            panic!("{asked:?}");
        };
        assert_eq!(card.title, "Deploy?");
        assert_eq!(card.sender.as_deref(), Some("Claude"));
        let question = card.question.as_ref().expect("a question");
        assert_eq!(question.accept, "Ship it");
        assert_eq!(question.reject, "Reject");
    }
    Ok(())
}

#[test]
fn a_question_nobody_answers_is_withdrawn() -> Result<()> {
    let home = tempfile::tempdir()?;
    let asked = app_that_answers(home.path(), None);
    let output = hopp(
        home.path(),
        &["ask", "--title", "Deploy?", "--timeout", "0"],
    )?;
    assert_eq!(output.status.code(), Some(2), "{}", complained(&output));
    assert_eq!(said(&output).trim(), "timed out");
    let asked = asked.lock().unwrap();
    let [AppRequest::Card(card), AppRequest::CancelCard(cancelled)] = asked.as_slice() else {
        panic!("{asked:?}");
    };
    assert_eq!(&card.id, cancelled);
    Ok(())
}

#[test]
fn a_notice_prints_its_id_and_a_question_without_the_app_isnt_a_yes() -> Result<()> {
    let home = tempfile::tempdir()?;
    let nobody = hopp(home.path(), &["ask", "--title", "Deploy?"])?;
    assert_eq!(nobody.status.code(), Some(1));
    assert!(
        complained(&nobody).contains("Hopp isn't running"),
        "{}",
        complained(&nobody)
    );

    let asked = app_that_answers(home.path(), None);
    let empty = hopp(home.path(), &["notify", "--title", "   "])?;
    assert!(
        complained(&empty).contains("A card needs a title"),
        "{}",
        complained(&empty)
    );

    let notice = hopp(
        home.path(),
        &[
            "notify",
            "--title",
            "Fixed",
            "--body",
            " It works. ",
            "--ephemeral",
        ],
    )?;
    assert!(notice.status.success(), "{}", complained(&notice));
    let asked = asked.lock().unwrap();
    let [AppRequest::Card(card)] = asked.as_slice() else {
        panic!("{asked:?}");
    };
    assert_eq!(said(&notice).trim(), card.id);
    assert_eq!(card.body.as_deref(), Some("It works."));
    assert!(card.ephemeral);
    assert_eq!(card.question, None);
    Ok(())
}
