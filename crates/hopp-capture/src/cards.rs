//! Cards: notices and yes/no questions that agents and scripts show in Hopp.
//!
//! An agent following a call (`hopp capture follow`, `hopp mcp`) can tell you something
//! without interrupting the call, or ask before it acts: "Deploy the fix to staging?".
//! `hopp notify` and `hopp ask`, or the MCP tools `notify_user` and
//! `request_user_decision`, send a card to the running app, which shows it where
//! notifications appear. A question waits for Accept or Reject; the app reports each card's
//! state in its snapshot, which is how the asker hears the answer.

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

use crate::app_socket::{self, AppRequest};

/// Longest title and body kept; a card is a glance, not a document.
pub const TITLE_LIMIT: usize = 120;
pub const BODY_LIMIT: usize = 600;
/// An ephemeral notice disappears after this long.
pub const EPHEMERAL: Duration = Duration::from_secs(10);
/// Cards remembered after they are answered, so an asker that looks a little late still
/// hears the answer.
const CARDS_KEPT: usize = 50;
/// How often someone waiting for an answer looks again.
const LOOK_EVERY: Duration = Duration::from_millis(250);

#[derive(Subcommand, Clone, Debug, PartialEq, Eq)]
pub enum CardCommand {
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
}

/// Returns the exit code.
pub fn run(command: CardCommand) -> Result<i32> {
    match command {
        CardCommand::Notify {
            title,
            body,
            sender,
            ephemeral,
        } => {
            let id = new_id();
            post(CardRequest {
                id: id.clone(),
                title,
                body,
                sender,
                question: None,
                ephemeral,
            })?;
            println!("{id}");
            Ok(0)
        }
        CardCommand::Ask {
            title,
            body,
            sender,
            accept,
            reject,
            timeout,
        } => ask(
            CardRequest {
                id: new_id(),
                title,
                body,
                sender,
                question: Some(Question { accept, reject }),
                ephemeral: false,
            },
            timeout.map(Duration::from_secs),
        ),
    }
}

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

    /// A question always waits for its answer.
    pub fn goes_by_itself(&self) -> bool {
        self.ephemeral && self.question.is_none()
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

/// A card and where it has got to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Card {
    #[serde(flatten)]
    pub request: CardRequest,
    pub state: CardState,
}

/// The cards the running app knows, oldest first.
#[derive(Debug, Default)]
pub struct Stack {
    cards: Vec<Card>,
}

impl Stack {
    /// Show a card. Posting an id again replaces that card.
    pub fn post(&mut self, request: CardRequest) {
        self.cards.retain(|card| card.request.id != request.id);
        self.cards.push(Card {
            request,
            state: CardState::Showing,
        });
        // Forget the oldest answered cards; showing ones stay until answered.
        while self.cards.len() > CARDS_KEPT {
            let Some(index) = self
                .cards
                .iter()
                .position(|card| card.state != CardState::Showing)
            else {
                break;
            };
            self.cards.remove(index);
        }
    }

    /// Answer, dismiss or withdraw a card that is still showing. Says whether it was.
    pub fn settle(&mut self, id: &str, state: CardState) -> bool {
        match self
            .cards
            .iter_mut()
            .find(|card| card.request.id == id && card.state == CardState::Showing)
        {
            Some(card) => {
                card.state = state;
                true
            }
            None => false,
        }
    }

    /// Cards still waiting, newest first.
    pub fn showing(&self) -> Vec<Card> {
        self.cards
            .iter()
            .rev()
            .filter(|card| card.state == CardState::Showing)
            .cloned()
            .collect()
    }

    /// What a snapshot says about them, which is how an asker hears the answer.
    pub fn snapshot(&self) -> Value {
        Value::Array(
            self.cards
                .iter()
                .map(|card| {
                    json!({
                        "id": card.request.id,
                        "state": card.state,
                        "title": card.request.title,
                        "question": card.request.question.is_some(),
                    })
                })
                .collect(),
        )
    }
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

/// A card's state in a snapshot from the running app.
pub fn state_in(snapshot: &Value, id: &str) -> Option<CardState> {
    snapshot["cards"]
        .as_array()?
        .iter()
        .find(|card| card["id"] == id)
        .and_then(|card| serde_json::from_value(card["state"].clone()).ok())
}

/// Show `card` in the running app.
pub fn post(card: CardRequest) -> Result<()> {
    let card = card.tidy();
    if card.title.is_empty() {
        bail!("A card needs a title");
    }
    app_socket::request(&AppRequest::Card(card)).map(|_| ())
}

/// Withdraw a card that is still showing.
pub fn cancel(id: &str) -> Result<()> {
    app_socket::request(&AppRequest::CancelCard(id.to_owned())).map(|_| ())
}

/// Wait until card `id` has an answer, or `timeout` passes (`None`: still showing).
pub fn wait(id: &str, timeout: Option<Duration>) -> Result<Option<CardState>> {
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    loop {
        let snapshot = app_socket::request(&AppRequest::Snapshot)
            .context("Lost touch with the running Hopp")?
            .state;
        // No snapshot means the app's window is still loading, and it will have one soon.
        if let Some(snapshot) = snapshot {
            match state_in(&snapshot, id) {
                Some(CardState::Showing) => {}
                Some(state) => return Ok(Some(state)),
                None => bail!("Hopp no longer knows card {id}"),
            }
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Ok(None);
        }
        std::thread::sleep(LOOK_EVERY);
    }
}

/// `hopp ask`: show the question, wait, print the answer. Returns the exit code: 0 for
/// accepted, 1 for rejected or closed, 2 when `timeout` passed (and the card is
/// withdrawn, so nobody answers a question nobody is waiting for).
pub fn ask(card: CardRequest, timeout: Option<Duration>) -> Result<i32> {
    let id = card.id.clone();
    post(card)?;
    let (word, code) = match wait(&id, timeout)? {
        Some(CardState::Accepted) => ("accepted", 0),
        Some(CardState::Rejected) => ("rejected", 1),
        Some(CardState::Dismissed) => ("dismissed", 1),
        Some(CardState::Cancelled) => ("cancelled", 1),
        Some(CardState::Expired) | Some(CardState::Showing) | None => {
            let _ = cancel(&id);
            ("timed out", 2)
        }
    };
    println!("{word}");
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(id: &str) -> CardRequest {
        CardRequest {
            id: id.into(),
            title: format!("Title of {id}"),
            body: None,
            sender: None,
            question: None,
            ephemeral: false,
        }
    }

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
            json!({"cards":[{"id":"card_1","state":"showing"},{"id":"card_2","state":"accepted"}]});
        assert_eq!(state_in(&snapshot, "card_1"), Some(CardState::Showing));
        assert_eq!(state_in(&snapshot, "card_2"), Some(CardState::Accepted));
        assert_eq!(state_in(&snapshot, "card_3"), None);
        assert_eq!(state_in(&json!({}), "card_1"), None);
    }

    #[test]
    fn ids_are_distinct() {
        let first = new_id();
        std::thread::sleep(Duration::from_micros(2));
        assert_ne!(first, new_id());
        assert!(first.starts_with("card_"));
    }

    #[test]
    fn a_stack_shows_the_newest_first_until_each_is_settled() {
        let mut stack = Stack::default();
        stack.post(notice("a"));
        stack.post(notice("b"));
        let ids = |stack: &Stack| -> Vec<String> {
            stack
                .showing()
                .into_iter()
                .map(|card| card.request.id)
                .collect()
        };
        assert_eq!(ids(&stack), ["b", "a"]);

        assert!(stack.settle("b", CardState::Accepted));
        assert_eq!(ids(&stack), ["a"]);
        assert!(
            !stack.settle("b", CardState::Rejected),
            "an answer is final"
        );
        assert!(!stack.settle("nobody", CardState::Dismissed));
        assert_eq!(
            state_in(&json!({ "cards": stack.snapshot() }), "b"),
            Some(CardState::Accepted)
        );

        stack.post(notice("a"));
        assert_eq!(stack.cards.len(), 2, "posting an id again replaces it");
    }

    #[test]
    fn answers_are_forgotten_but_nothing_still_showing_is() {
        let mut stack = Stack::default();
        stack.post(notice("waiting"));
        for number in 0..CARDS_KEPT * 2 {
            let id = format!("old-{number}");
            stack.post(notice(&id));
            stack.settle(&id, CardState::Dismissed);
        }
        assert_eq!(stack.cards.len(), CARDS_KEPT);
        assert_eq!(stack.showing().len(), 1);
        assert_eq!(stack.showing()[0].request.id, "waiting");

        for number in 0..CARDS_KEPT * 2 {
            stack.post(notice(&format!("new-{number}")));
        }
        assert_eq!(stack.showing().len(), CARDS_KEPT * 2 + 1);
    }

    #[test]
    fn only_a_notice_goes_by_itself() {
        let mut card = notice("a");
        assert!(!card.goes_by_itself());
        card.ephemeral = true;
        assert!(card.goes_by_itself());
        card.question = Some(Question {
            accept: "Yes".into(),
            reject: "No".into(),
        });
        assert!(!card.goes_by_itself());
    }
}
