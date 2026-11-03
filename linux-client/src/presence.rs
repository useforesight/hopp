//! Hopp's control websocket: online status, keepalive and call-presence validation.

use crate::api::ApiClient;
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Clone)]
pub enum PresenceEvent {
    Connected,
    Disconnected,
    Changed,
    Message {
        kind: String,
        payload: serde_json::Value,
    },
}

impl std::fmt::Debug for PresenceEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connected => formatter.write_str("Connected"),
            Self::Disconnected => formatter.write_str("Disconnected"),
            Self::Changed => formatter.write_str("Changed"),
            Self::Message { kind, .. } => formatter
                .debug_struct("Message")
                .field("kind", kind)
                .field("payload", &"[redacted]")
                .finish(),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct IncomingCall {
    pub caller_id: String,
    pub initiated_at: Option<i64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct IncomingInvite {
    pub inviter_id: String,
    pub invite_id: String,
    pub initiated_at: Option<i64>,
}

/// Ignore expired rings delivered after a reconnect. Older servers may omit
/// the timestamp. Hopp uses Unix milliseconds; seconds are accepted as well.
pub fn recent(initiated_at: Option<i64>) -> bool {
    let Some(issued) = initiated_at else {
        return true;
    };
    let issued = if issued > 100_000_000_000 {
        issued / 1000
    } else {
        issued
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    now.saturating_sub(issued) < 60
}

pub fn call_request(callee_id: &str) -> serde_json::Value {
    serde_json::json!({"type":"call_request","payload":{"callee_id":callee_id}})
}

pub fn call_accept(caller_id: &str) -> serde_json::Value {
    serde_json::json!({"type":"call_accept","payload":{"caller_id":caller_id}})
}

pub fn call_reject(caller_id: &str, reason: &str) -> serde_json::Value {
    serde_json::json!({"type":"call_reject","payload":{"caller_id":caller_id,"reject_reason":reason}})
}

/// Also cancels a ringing direct call. For an active call use tokens.participant.
pub fn call_end(participant_id: &str) -> serde_json::Value {
    serde_json::json!({"type":"call_end","payload":{"participant_id":participant_id}})
}

pub fn invite_request(invitee_id: &str, invite_id: &str) -> serde_json::Value {
    serde_json::json!({"type":"invite_request","payload":{"invitee_id":invitee_id,"invite_id":invite_id}})
}

/// After this message, the invitee requests ApiClient::join_call(inviter_id).
pub fn invite_accept(inviter_id: &str, invite_id: &str) -> serde_json::Value {
    serde_json::json!({"type":"invite_accept","payload":{"inviter_id":inviter_id,"invite_id":invite_id}})
}

pub fn invite_reject(inviter_id: &str, invite_id: &str, reason: &str) -> serde_json::Value {
    serde_json::json!({"type":"invite_reject","payload":{"inviter_id":inviter_id,"invite_id":invite_id,"reject_reason":reason}})
}

pub fn invite_cancel(invitee_id: &str, invite_id: &str) -> serde_json::Value {
    serde_json::json!({"type":"invite_cancel","payload":{"invitee_id":invitee_id,"invite_id":invite_id}})
}

/// Reconnects until stop is true or its sender is dropped. `current_room` is set
/// only when the native LiveKit session connects and cleared immediately on leave.
/// Dropping the returned future also closes the websocket.
pub async fn run_presence(
    api: ApiClient,
    current_room: Arc<Mutex<Option<String>>>,
    mut stop: watch::Receiver<bool>,
    events: crate::wake::Sender<PresenceEvent>,
    mut outgoing: tokio::sync::mpsc::Receiver<serde_json::Value>,
) -> Result<()> {
    let url = api.websocket_url()?;
    let mut retry = Duration::from_secs(1);
    let mut outgoing_open = true;
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        let connected = tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(20), connect_async(url.as_str())) => result,
            _ = stop.changed() => return Ok(()),
        };
        if let Ok(Ok((mut socket, _))) = connected {
            retry = Duration::from_secs(1);
            let _ = events.send(PresenceEvent::Connected);
            let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    command = outgoing.recv(), if outgoing_open => {
                        match command {
                            Some(command) => {
                                if socket.send(Message::Text(command.to_string().into())).await.is_err() {
                                    let _ = events.send(PresenceEvent::Message { kind: "error".into(),
                                        payload: serde_json::json!({"error":"Hopp connection lost; the call action may not have been delivered"}) });
                                    break;
                                }
                            }
                            None => outgoing_open = false,
                        }
                    }
                    _ = stop.changed() => {
                        let _ = tokio::time::timeout(Duration::from_secs(2), socket.close(None)).await;
                        return Ok(());
                    }
                    _ = heartbeat.tick() => {
                        if socket.send(Message::Text(r#"{"type":"ping","payload":{"message":"ping"}}"#.into())).await.is_err() {
                            break;
                        }
                    }
                    message = socket.next() => {
                        match message {
                            Some(Ok(Message::Text(text))) => {
                                let Ok(message) = serde_json::from_str::<serde_json::Value>(&text) else { continue; };
                                match message.get("type").and_then(serde_json::Value::as_str) {
                                    Some("presence_changed") => { let _ = events.send(PresenceEvent::Changed); }
                                    Some("presence_check") => {
                                        let Some(room) = message.get("payload").and_then(|payload| payload.get("room"))
                                            .and_then(serde_json::Value::as_str) else { continue; };
                                        let in_call = current_room.lock().map(|current| current.as_deref() == Some(room)).unwrap_or(false);
                                        let ack = serde_json::json!({"type":"presence_ack", "payload":{"room":room,"in_call":in_call}});
                                        if socket.send(Message::Text(ack.to_string().into())).await.is_err() { break; }
                                    }
                                    Some("success" | "pong") | None => {}
                                    Some(kind) => {
                                        let _ = events.send(PresenceEvent::Message { kind: kind.to_string(),
                                            payload: message.get("payload").cloned().unwrap_or(serde_json::Value::Null) });
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(payload))) => {
                                if socket.send(Message::Pong(payload)).await.is_err() { break; }
                            }
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                            _ => {}
                        }
                    }
                }
            }
        }
        let _ = events.send(PresenceEvent::Disconnected);
        // Never propagate transport errors: their URLs can contain the JWT query.
        tokio::select! {
            _ = tokio::time::sleep(retry) => {},
            _ = stop.changed() => return Ok(()),
        }
        retry = (retry * 2).min(Duration::from_secs(8));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_direct_calls_and_existing_call_invites_distinct() {
        assert_eq!(
            call_request("callee"),
            serde_json::json!({"type":"call_request","payload":{"callee_id":"callee"}})
        );
        assert_eq!(
            call_accept("caller")["payload"],
            serde_json::json!({"caller_id":"caller"})
        );
        assert_eq!(
            call_reject("caller", "in-call")["payload"]["reject_reason"],
            "in-call"
        );
        assert_eq!(
            call_end("peer")["payload"],
            serde_json::json!({"participant_id":"peer"})
        );
        assert_eq!(
            invite_request("callee", "unique")["payload"],
            serde_json::json!({"invitee_id":"callee","invite_id":"unique"})
        );
        assert_eq!(
            invite_accept("caller", "unique")["payload"],
            serde_json::json!({"inviter_id":"caller","invite_id":"unique"})
        );
        assert_eq!(
            invite_cancel("callee", "unique")["payload"],
            serde_json::json!({"invitee_id":"callee","invite_id":"unique"})
        );
        assert_eq!(
            invite_reject("caller", "unique", "rejected")["payload"]["reject_reason"],
            "rejected"
        );
    }

    #[test]
    fn reads_incoming_call_and_invite_fixtures_without_logging_tokens() {
        let call: IncomingCall =
            serde_json::from_str(r#"{"caller_id":"usr-123","initiated_at":1770000000}"#).unwrap();
        assert_eq!(call.caller_id, "usr-123");
        assert_eq!(call.initiated_at, Some(1770000000));
        let invite: IncomingInvite =
            serde_json::from_str(r#"{"inviter_id":"usr-123","invite_id":"invite-456"}"#).unwrap();
        assert_eq!(invite.invite_id, "invite-456");
        assert!(invite.initiated_at.is_none());
        let event = PresenceEvent::Message {
            kind: "call_tokens".into(),
            payload: serde_json::json!({"audioToken":"secret"}),
        };
        assert!(!format!("{event:?}").contains("secret"));
    }

    #[tokio::test]
    #[ignore = "Requires explicitly authorized local Hopp stack and HOPP_API_TEST_FIXTURE private file"]
    async fn direct_call_negotiates_real_hopp_tokens() -> Result<()> {
        async fn receive(
            receiver: &std::sync::mpsc::Receiver<PresenceEvent>,
            kind: &str,
        ) -> Result<serde_json::Value> {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    while let Ok(event) = receiver.try_recv() {
                        if let PresenceEvent::Message {
                            kind: received,
                            payload,
                        } = event
                        {
                            if received == kind {
                                return Ok(payload);
                            }
                            if received == "error" {
                                anyhow::bail!("Local Hopp call negotiation returned an error");
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .map_err(|_| anyhow::anyhow!("Timed out waiting for local Hopp {kind}"))?
        }
        let path = std::env::var("HOPP_API_TEST_FIXTURE").map_err(|_| {
            anyhow::anyhow!("Set HOPP_API_TEST_FIXTURE to a private local test fixture")
        })?;
        let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let base = fixture["api_base"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing local API base"))?;
        if !base.starts_with("http://127.0.0.1:") {
            anyhow::bail!("Live call test requires a localhost fixture");
        }
        let user = |role: &str| -> Result<(&str, &str)> {
            Ok((
                fixture[role]["token"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing test token"))?,
                fixture[role]["user_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing test user ID"))?,
            ))
        };
        let (caller_token, caller_id) = user("publisher")?;
        let (callee_token, callee_id) = user("viewer")?;
        let caller_api = ApiClient::new(base, Some(caller_token.into()))?;
        let callee_api = ApiClient::new(base, Some(callee_token.into()))?;
        let media_url = caller_api.server_url().await?;
        let (stop, stop_rx) = watch::channel(false);
        let (caller_outgoing, caller_out_rx) = tokio::sync::mpsc::channel(8);
        let (callee_outgoing, callee_out_rx) = tokio::sync::mpsc::channel(8);
        let (caller_events, caller_rx) = crate::wake::detached();
        let (callee_events, callee_rx) = crate::wake::detached();
        let caller_worker = tokio::spawn(run_presence(
            caller_api.clone(),
            Arc::new(Mutex::new(None)),
            stop_rx.clone(),
            caller_events,
            caller_out_rx,
        ));
        let callee_worker = tokio::spawn(run_presence(
            callee_api.clone(),
            Arc::new(Mutex::new(None)),
            stop_rx,
            callee_events,
            callee_out_rx,
        ));
        let result: Result<()> = async {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let teammates = caller_api.teammates().await?;
                    if teammates
                        .iter()
                        .any(|person| person.id == callee_id && person.is_active)
                    {
                        return Ok::<_, anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await??;
            caller_outgoing.send(call_request(callee_id)).await?;
            let incoming = receive(&callee_rx, "incoming_call").await?;
            assert_eq!(incoming["caller_id"], caller_id);
            callee_outgoing
                .send(call_reject(caller_id, "rejected"))
                .await?;
            let rejected = receive(&caller_rx, "call_reject").await?;
            assert_eq!(rejected["reject_reason"], "rejected");
            // Reject and cancel must release the deduplication slot immediately.
            caller_outgoing.send(call_request(callee_id)).await?;
            receive(&callee_rx, "incoming_call").await?;
            caller_outgoing.send(call_end(callee_id)).await?;
            let canceled = receive(&callee_rx, "call_end").await?;
            assert_eq!(canceled["participant_id"], caller_id);
            caller_outgoing.send(call_request(callee_id)).await?;
            receive(&callee_rx, "incoming_call").await?;
            callee_outgoing.send(call_accept(caller_id)).await?;
            let caller_tokens = receive(&caller_rx, "call_tokens").await?;
            let callee_tokens = receive(&callee_rx, "call_tokens").await?;
            let caller_credentials =
                crate::api::RoomTokens::from_payload(caller_tokens, media_url.clone())?;
            let callee_credentials =
                crate::api::RoomTokens::from_payload(callee_tokens, media_url)?;
            assert_eq!(caller_credentials.participant, callee_id);
            assert_eq!(callee_credentials.participant, caller_id);
            caller_outgoing
                .send(invite_request(callee_id, "native-integration-invite"))
                .await?;
            let invited = receive(&callee_rx, "incoming_invite").await?;
            assert_eq!(invited["inviter_id"], caller_id);
            callee_outgoing
                .send(invite_accept(caller_id, "native-integration-invite"))
                .await?;
            let accepted = receive(&caller_rx, "invite_accept").await?;
            assert_eq!(accepted["invitee_id"], callee_id);
            let joined = callee_api.join_call(caller_id).await?;
            assert_eq!(joined.tokens.participant, caller_id);
            assert!(joined.room.is_none());
            caller_outgoing.send(call_end(callee_id)).await?;
            let ended = receive(&callee_rx, "call_end").await?;
            assert_eq!(ended["participant_id"], caller_id);
            Ok(())
        }
        .await;
        // Cleanup both fake users even if an assertion-independent step failed.
        let _ = caller_outgoing.send(call_end(callee_id)).await;
        let _ = callee_outgoing.send(call_end(caller_id)).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = stop.send(true);
        caller_worker.await??;
        callee_worker.await??;
        result
    }
}
