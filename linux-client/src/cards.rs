//! Cards: notices and yes/no questions that agents and scripts show in Hopp.
//!
//! An agent following a call (`hopp capture follow`, `hopp mcp`) can tell you something
//! without interrupting the call, or ask before it acts: "Deploy the fix to staging?".
//! `hopp notify` and `hopp ask`, or the MCP tools `notify_user` and
//! `request_user_decision`, send a card to the running Hopp, which shows it where
//! Omarchy's notifications appear. A question waits for Accept or Reject; the running
//! Hopp reports each card's state in its state snapshot, which is how the asker hears
//! the answer.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

use crate::protocol::UiRequest;

/// Longest title and body kept; a card is a glance, not a document.
pub const TITLE_LIMIT: usize = 120;
pub const BODY_LIMIT: usize = 600;
/// An ephemeral notice disappears after this long.
pub const EPHEMERAL: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Question {
    /// The labels of the two buttons.
    pub accept: String,
    pub reject: String,
}

/// A card as asked for.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CardRequest {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Who it is from: "Claude", "CI".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    /// A question waits for an answer; a notice is only dismissed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<Question>,
    /// A notice that goes by itself after [`EPHEMERAL`].
    #[serde(default)]
    pub ephemeral: bool,
}

impl CardRequest {
    /// Trimmed to the limits, with an empty body dropped.
    pub fn tidy(mut self) -> Self {
        fn cut(text: &str, limit: usize) -> String {
            let text = text.trim();
            match text.char_indices().nth(limit) {
                Some((end, _)) => format!("{}…", text[..end].trim_end()),
                None => text.to_owned(),
            }
        }
        self.title = cut(&self.title, TITLE_LIMIT);
        self.body = self
            .body
            .map(|body| cut(&body, BODY_LIMIT))
            .filter(|body| !body.is_empty());
        self.sender = self
            .sender
            .map(|sender| cut(&sender, 40))
            .filter(|sender| !sender.is_empty());
        if let Some(question) = &mut self.question {
            question.accept = cut(&question.accept, 24);
            question.reject = cut(&question.reject, 24);
        }
        self
    }
}

/// Where a card has got to. Everything but `Showing` is final.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CardState {
    Showing,
    /// A question's Accept.
    Accepted,
    /// A question's Reject.
    Rejected,
    /// Closed without an answer, or a notice read.
    Dismissed,
    /// Withdrawn by whoever posted it.
    Cancelled,
    /// An ephemeral notice that went by itself.
    Expired,
}

/// A new card id: time-ordered and unique enough for one machine.
pub fn new_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_nanos());
    format!(
        "card_{:x}{:04x}",
        nanos / 1_000,
        std::process::id() & 0xffff
    )
}

/// A card's state in a state snapshot from the running Hopp.
pub fn state_in(snapshot: &str, id: &str) -> Option<CardState> {
    let snapshot: serde_json::Value = serde_json::from_str(snapshot).ok()?;
    snapshot["cards"]
        .as_array()?
        .iter()
        .find(|card| card["id"] == id)
        .and_then(|card| serde_json::from_value(card["state"].clone()).ok())
}

/// Show `card` in the running Hopp.
pub fn post(instance: &str, card: CardRequest) -> Result<()> {
    if card.title.trim().is_empty() {
        bail!("A card needs a title");
    }
    if !crate::instance::request_existing(instance, UiRequest::Card(card.tidy()))? {
        bail!("Hopp isn't running");
    }
    Ok(())
}

/// Withdraw a card that is still showing.
pub fn cancel(instance: &str, id: &str) -> Result<()> {
    if !crate::instance::request_existing(instance, UiRequest::CancelCard(id.to_owned()))? {
        bail!("Hopp isn't running");
    }
    Ok(())
}

/// Wait until card `id` has an answer, or `timeout` passes (`None`: still showing).
pub fn wait(instance: &str, id: &str, timeout: Option<Duration>) -> Result<Option<CardState>> {
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    let mut seen = false;
    let outcome = crate::instance::watch(instance, deadline, |snapshot| {
        match state_in(snapshot, id) {
            Some(CardState::Showing) => {
                seen = true;
                None
            }
            Some(state) => Some(Ok(state)),
            // Not shown yet (its request may still be on the way), or forgotten.
            None if seen => Some(Err(())),
            None => None,
        }
    })
    .context("Lost touch with the running Hopp")?;
    match outcome {
        Some(Ok(state)) => Ok(Some(state)),
        Some(Err(())) => bail!("Hopp no longer knows card {id}"),
        None => Ok(None),
    }
}

/// `hopp ask`: show the question, wait, print the answer. Returns the exit code: 0 for
/// accepted, 1 for rejected or closed, 2 when `timeout` passed (and the card is
/// withdrawn, so nobody answers a question nobody is waiting for).
pub fn ask(instance: &str, card: CardRequest, timeout: Option<Duration>) -> Result<i32> {
    let id = card.id.clone();
    post(instance, card)?;
    let state = wait(instance, &id, timeout)?;
    let (word, code) = match state {
        Some(CardState::Accepted) => ("accepted", 0),
        Some(CardState::Rejected) => ("rejected", 1),
        Some(CardState::Dismissed) => ("dismissed", 1),
        Some(CardState::Cancelled) => ("cancelled", 1),
        Some(CardState::Expired) | Some(CardState::Showing) | None => {
            let _ = cancel(instance, &id);
            ("timed out", 2)
        }
    };
    println!("{word}");
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_trims_and_cuts_to_the_limits() {
        let card = CardRequest {
            id: "c".into(),
            title: format!("  {}  ", "t".repeat(200)),
            body: Some("   ".into()),
            sender: Some(" Claude ".into()),
            question: Some(Question {
                accept: "Deploy".into(),
                reject: "x".repeat(40),
            }),
            ephemeral: false,
        }
        .tidy();
        assert_eq!(
            card.title.chars().count(),
            TITLE_LIMIT + 1,
            "with an ellipsis"
        );
        assert!(card.title.ends_with('…'));
        assert_eq!(card.body, None, "an empty body goes");
        assert_eq!(card.sender.as_deref(), Some("Claude"));
        assert_eq!(card.question.unwrap().reject.chars().count(), 25);
    }

    #[test]
    fn reads_a_cards_state_from_a_snapshot() {
        let snapshot =
            r#"{"cards":[{"id":"card_1","state":"showing"},{"id":"card_2","state":"accepted"}]}"#;
        assert_eq!(state_in(snapshot, "card_1"), Some(CardState::Showing));
        assert_eq!(state_in(snapshot, "card_2"), Some(CardState::Accepted));
        assert_eq!(state_in(snapshot, "card_3"), None);
        assert_eq!(state_in("{}", "card_1"), None);
    }

    #[test]
    fn ids_are_distinct() {
        let first = new_id();
        std::thread::sleep(Duration::from_micros(2));
        assert_ne!(first, new_id());
        assert!(first.starts_with("card_"));
    }
}
