//! Worker regressions without capture, clipboard commands, or injected input.
use super::{ClipboardResult, Event, Participant, Recovery, SharedFrames, Worker};
use crate::{protocol::ClipboardPacket, settings::Settings};

fn worker() -> Worker {
    let (events, _) = crate::wake::detached();
    Worker::new(Settings::default(), SharedFrames::default(), events)
}

async fn selection_finished(worker: &Worker) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !worker.pending_capture.as_ref().unwrap().task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Mock chooser did not finish after its response");
}

#[tokio::test]
async fn canceling_pending_selection_returns_without_waiting_and_blocks_a_second_picker() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;
    let (events, receive) = crate::wake::detached();
    let mut worker = Worker::new(Settings::default(), SharedFrames::default(), events);
    let canceled = Arc::new(AtomicBool::new(false));
    let (dismiss, picker) = tokio::sync::oneshot::channel::<()>();
    worker.pending_capture = Some(super::PendingCapture {
        task: tokio::spawn(async move {
            picker.await?;
            anyhow::bail!("Sharing canceled");
        }),
        canceled: canceled.clone(),
        generation: worker.capture_generation,
    });
    let task_id = worker.pending_capture.as_ref().unwrap().task.id();
    worker.grant.grant("room:test:alice:audio".into());
    tokio::time::timeout(
        Duration::from_millis(100),
        worker.command(super::Command::StopShare),
    )
    .await
    .expect("StopShare waited for the system chooser")
    .unwrap();
    assert!(canceled.load(Ordering::Acquire));
    assert!(!worker.grant.active());
    assert!(!worker.pending_capture.as_ref().unwrap().task.is_finished());
    // Even a queued hotkey must not open another picker while the first one
    // owns the portal's synchronous source-selection call.
    worker
        .command(super::Command::Share { window_only: true })
        .await
        .unwrap();
    assert_eq!(worker.pending_capture.as_ref().unwrap().task.id(), task_id);
    assert!(receive.try_iter().all(|event| !matches!(
        event,
        Event::SelectingSource(false) | Event::Sharing { active: true, .. } | Event::Error(_)
    )));
    dismiss.send(()).unwrap();
    selection_finished(&worker).await;
    worker.finish_operations().await;
    assert!(worker.pending_capture.is_none());
    assert!(
        receive
            .try_iter()
            .any(|event| matches!(event, Event::SelectingSource(false)))
    );
}

#[tokio::test]
async fn late_picker_consent_after_leave_cannot_deliver_frames_or_reopen_sharing() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let (events, receive) = crate::wake::detached();
    let mut worker = Worker::new(Settings::default(), SharedFrames::default(), events);
    let old_frames = worker.capture_frames.clone();
    let canceled = Arc::new(AtomicBool::new(false));
    let flag = canceled.clone();
    let producer_frames = old_frames.clone();
    let (accept, picker) = tokio::sync::oneshot::channel::<()>();
    let (reported, report) = tokio::sync::oneshot::channel();
    worker.pending_capture = Some(super::PendingCapture {
        task: tokio::spawn(async move {
            picker.await?;
            // Model consent arriving after cancellation, including a producer
            // racing to deliver its first frame into the old private slot.
            *producer_frames.lock().unwrap() =
                Some(crate::convert::ScreenFrame::from_bgra(1, 1, &[40, 50, 60, 255]).unwrap());
            let _ = reported.send(flag.load(Ordering::Acquire));
            anyhow::bail!("Sharing canceled");
        }),
        canceled,
        generation: worker.capture_generation,
    });
    worker.leave().await;
    // The next call owns a fresh event generation while the old picker drains.
    worker.reset_session_events();
    assert!(!Arc::ptr_eq(&old_frames, &worker.capture_frames));
    accept.send(()).unwrap();
    assert!(
        report.await.unwrap(),
        "late consent did not observe cancellation"
    );
    selection_finished(&worker).await;
    worker.finish_operations().await;
    worker.flush_frames();
    assert!(worker.capture.is_none() && worker.pending_capture.is_none());
    assert!(worker.capture_frames.lock().unwrap().is_none());
    let events = receive.try_iter().collect::<Vec<_>>();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::SelectingSource(false)))
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, Event::Error(_) | Event::Sharing { active: true, .. }))
    );
}

#[tokio::test]
async fn cancel_in_the_system_picker_clears_selection_without_an_error() {
    use std::sync::{Arc, atomic::AtomicBool};
    let (events, receive) = crate::wake::detached();
    let mut worker = Worker::new(Settings::default(), SharedFrames::default(), events);
    worker.pending_capture = Some(super::PendingCapture {
        task: tokio::spawn(async {
            Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled).into())
        }),
        canceled: Arc::new(AtomicBool::new(false)),
        generation: worker.capture_generation,
    });
    selection_finished(&worker).await;
    worker.finish_operations().await;
    assert!(worker.pending_capture.is_none());
    let events = receive.try_iter().collect::<Vec<_>>();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::SelectingSource(false)))
    );
    assert!(events.iter().any(
        |event| matches!(event, Event::Status(status) if status == "Screen selection canceled")
    ));
    assert!(events.iter().all(|event| !matches!(event, Event::Error(_))));
}

#[tokio::test]
async fn only_the_granted_controller_can_relinquish_its_host_grant() {
    let mut worker = worker();
    worker.participants = ["room:test:alice:audio", "room:test:bob:audio"]
        .into_iter()
        .map(|identity| Participant {
            identity: identity.into(),
            name: identity.into(),
            ..Default::default()
        })
        .collect();
    worker.grant.grant("room:test:alice:audio".into());
    let disabled = serde_json::to_vec(&crate::protocol::InputEvent::RemoteControlEnabled(
        crate::protocol::Enabled { enabled: false },
    ))
    .unwrap();
    worker
        .receive_data(
            "room:test:bob:audio",
            &disabled,
            Some("remote_control_enabled"),
        )
        .await
        .unwrap();
    assert!(
        worker.grant.active(),
        "another participant revoked Alice's grant"
    );
    worker
        .receive_data(
            "room:test:alice:audio",
            &disabled,
            Some("remote_control_enabled"),
        )
        .await
        .unwrap();
    assert!(!worker.grant.active());
    let enabled = serde_json::to_vec(&crate::protocol::InputEvent::RemoteControlEnabled(
        crate::protocol::Enabled { enabled: true },
    ))
    .unwrap();
    worker
        .receive_data(
            "room:test:alice:audio",
            &enabled,
            Some("remote_control_enabled"),
        )
        .await
        .unwrap();
    assert!(!worker.grant.active(), "the peer granted itself control");
}

#[tokio::test]
async fn switching_shared_host_drops_the_previous_control_and_clipboard_lease() {
    let mut worker = worker();
    worker.remote_host = Some("room:test:alice:audio".into());
    worker.peer_control = true;
    *worker.shared.remote.lock().unwrap() = Some(crate::frames::VideoFrame {
        width: 1,
        height: 1,
        bgra: vec![0, 0, 0, 255],
    });
    worker
        .remote_clipboard
        .push(ClipboardPacket {
            packet_id: 0,
            total_packets: 2,
            data: b"old host ".to_vec(),
        })
        .unwrap();
    let (finish, pending) = tokio::sync::oneshot::channel::<Vec<u8>>();
    worker.clipboard_tasks.push(tokio::spawn(async move {
        Ok(ClipboardResult::SendPaste(pending.await?))
    }));

    worker
        .internal(Event::RemoteShare {
            identity: "room:test:bob:video".into(),
            active: true,
        })
        .await;

    assert_eq!(worker.remote_host.as_deref(), Some("room:test:bob:audio"));
    assert!(worker.shared.remote.lock().unwrap().is_none());
    assert!(
        !worker.peer_control,
        "a new host needs a fresh control grant"
    );
    assert!(worker.clipboard_tasks.is_empty());
    assert!(
        worker
            .remote_clipboard
            .push(ClipboardPacket {
                packet_id: 1,
                total_packets: 2,
                data: b"clipboard".to_vec(),
            })
            .is_err(),
        "a new host must not finish the old host's clipboard transfer"
    );
    drop(finish);
}

#[tokio::test]
async fn participant_updates_preserve_a_viewers_pending_clipboard_transfer() {
    let mut worker = worker();
    worker.remote_host = Some("room:test:alice:audio".into());
    worker.peer_control = true;
    let (finish, pending) = tokio::sync::oneshot::channel::<Vec<u8>>();
    worker.clipboard_tasks.push(tokio::spawn(async move {
        Ok(ClipboardResult::SendPaste(pending.await?))
    }));

    // Camera/video identities can connect while the same audio participant
    // continues hosting. The viewer has no local input-control grant to revoke.
    worker
        .internal(Event::Participants(vec![Participant {
            identity: "room:test:alice:audio".into(),
            name: "Alice".into(),
            ..Default::default()
        }]))
        .await;

    assert!(worker.peer_control);
    assert_eq!(worker.clipboard_tasks.len(), 1);
    finish.send(b"still in progress".to_vec()).unwrap();
    let completed = worker
        .clipboard_tasks
        .pop()
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(completed, ClipboardResult::SendPaste(bytes) if bytes == b"still in progress")
    );
}

#[tokio::test]
async fn reconnecting_invalidates_both_control_and_pending_clipboard_transfer() {
    let mut worker = worker();
    worker.peer_control = true;
    worker.transport_ready = true;
    let (finish, pending) = tokio::sync::oneshot::channel::<Vec<u8>>();
    worker.clipboard_tasks.push(tokio::spawn(async move {
        Ok(ClipboardResult::SendPaste(pending.await?))
    }));

    worker.internal(Event::TransportReconnecting).await;

    assert!(!worker.peer_control);
    assert!(!worker.transport_ready);
    assert!(worker.clipboard_tasks.is_empty());
    worker.internal(Event::TransportReconnected).await;
    assert!(worker.transport_ready);
    assert!(
        !worker.peer_control,
        "reconnection must not restore an old grant"
    );
    drop(finish);
}

#[tokio::test]
#[ignore = "Requires authorized localhost Hopp stack and HOPP_API_TEST_FIXTURE private file"]
async fn canceled_direct_connection_before_poll_notifies_the_real_peer() -> anyhow::Result<()> {
    use crate::{
        api::ApiClient,
        presence::{self, PresenceEvent},
    };
    use anyhow::{Context, bail};
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    async fn receive(
        receiver: &std::sync::mpsc::Receiver<PresenceEvent>,
        kind: &str,
    ) -> anyhow::Result<serde_json::Value> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                while let Ok(event) = receiver.try_recv() {
                    match event {
                        PresenceEvent::Connected if kind == "connected" => {
                            return Ok(serde_json::Value::Null);
                        }
                        PresenceEvent::Message {
                            kind: received,
                            payload,
                        } if received == kind => return Ok(payload),
                        PresenceEvent::Message { kind: received, .. } if received == "error" => {
                            bail!("Local call negotiation failed")
                        }
                        _ => {}
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("Timed out waiting for real local Hopp peer")?
    }

    let path =
        std::env::var("HOPP_API_TEST_FIXTURE").context("Set private localhost call fixture")?;
    let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let base = fixture["api_base"]
        .as_str()
        .context("Missing local API base")?;
    if !base.starts_with("http://127.0.0.1:") {
        bail!("This test requires a localhost fixture");
    }
    let caller_id = fixture["publisher"]["user_id"]
        .as_str()
        .context("Missing caller ID")?;
    let callee_id = fixture["viewer"]["user_id"]
        .as_str()
        .context("Missing callee ID")?;
    let caller = ApiClient::new(
        base,
        Some(
            fixture["publisher"]["token"]
                .as_str()
                .context("Missing caller token")?
                .into(),
        ),
    )?;
    let callee = ApiClient::new(
        base,
        Some(
            fixture["viewer"]["token"]
                .as_str()
                .context("Missing callee token")?
                .into(),
        ),
    )?;
    let (caller_events, caller_rx) = crate::wake::detached();
    let (callee_events, callee_rx) = crate::wake::detached();
    let (caller_send, caller_commands) = tokio::sync::mpsc::channel(16);
    let (callee_send, callee_commands) = tokio::sync::mpsc::channel(16);
    let (caller_stop, caller_stopped) = tokio::sync::watch::channel(false);
    let (callee_stop, callee_stopped) = tokio::sync::watch::channel(false);
    let caller_task = tokio::spawn(presence::run_presence(
        caller.clone(),
        Arc::new(Mutex::new(None)),
        caller_stopped,
        caller_events,
        caller_commands,
    ));
    let callee_task = tokio::spawn(presence::run_presence(
        callee,
        Arc::new(Mutex::new(None)),
        callee_stopped,
        callee_events,
        callee_commands,
    ));
    let result = async {
        receive(&caller_rx, "connected").await?;
        receive(&callee_rx, "connected").await?;
        caller_send.send(presence::call_request(callee_id)).await?;
        receive(&callee_rx, "incoming_call").await?;
        callee_send.send(presence::call_accept(caller_id)).await?;
        let tokens = receive(&caller_rx, "call_tokens").await?;
        receive(&callee_rx, "call_tokens").await?;

        let mut worker = worker();
        worker.api = Some(caller);
        worker.outgoing = Some(caller_send.clone());
        worker.socket_connected = true;
        worker.call_peer = Some(callee_id.into());
        worker.begin_join(super::JoinTarget::Direct {
            payload: tokens,
            peer: callee_id.into(),
            name: "Local test peer".into(),
        })?;
        // The current-thread runtime has not polled the spawned connect task.
        // Its tokens already changed real backend presence, so cancellation
        // must still send the real call_end despite skipping media setup.
        worker.leave().await;
        if let Some(cleanup) = worker.join_cleanup.take() {
            cleanup.await?;
        }
        let ended = receive(&callee_rx, "call_end").await?;
        if ended["participant_id"].as_str() != Some(caller_id) {
            bail!("Peer hangup had the wrong participant");
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    // Both local fake users are left clean even when the assertion fails.
    let _ = caller_send.send(presence::call_end(callee_id)).await;
    let _ = callee_send.send(presence::call_end(caller_id)).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = caller_stop.send(true);
    let _ = callee_stop.send(true);
    let _ = caller_task.await;
    let _ = callee_task.await;
    result
}

#[test]
fn audio_recovery_retries_quickly_then_backs_off_and_reports_once() {
    let mut recovery = Recovery::default();
    assert!(recovery.wait().is_none());
    recovery.failed();
    assert_eq!(recovery.delay, Recovery::FIRST);
    assert!(recovery.wait().unwrap() <= Recovery::FIRST);
    assert!(recovery.report());
    for _ in 0..10 {
        recovery.failed();
    }
    assert_eq!(recovery.delay, Recovery::MAX);
    assert!(!recovery.report());
}

#[test]
fn audio_recovery_starts_over_after_a_stable_run() {
    let mut recovery = Recovery::default();
    recovery.failed();
    recovery.failed();
    assert!(recovery.delay > Recovery::FIRST);
    recovery.due = None;
    recovery.last_failure = Some(std::time::Instant::now() - Recovery::STABLE);
    recovery.failed();
    assert_eq!(recovery.delay, Recovery::FIRST);
    assert!(recovery.report());
}

#[tokio::test]
async fn a_stale_speaker_failure_without_a_call_is_ignored() {
    let mut worker = worker();
    worker
        .internal(Event::AudioDeviceFailed {
            input: false,
            error: "gone".into(),
        })
        .await;
    assert!(worker.speaker.due.is_none());
}

/// A worker that is signed in and connected, its outgoing socket messages and its events.
fn connected_worker(
    status: crate::api::UserStatus,
) -> (
    Worker,
    tokio::sync::mpsc::Receiver<serde_json::Value>,
    std::sync::mpsc::Receiver<Event>,
) {
    let (events, events_rx) = crate::wake::detached();
    let mut worker = Worker::new(Settings::default(), SharedFrames::default(), events);
    let (outgoing, outgoing_rx) = tokio::sync::mpsc::channel(8);
    worker.outgoing = Some(outgoing);
    worker.socket_connected = true;
    worker.own_status = Some((status, false));
    (worker, outgoing_rx, events_rx)
}

fn missed_events(events: &std::sync::mpsc::Receiver<Event>) -> Vec<(String, bool)> {
    std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|event| match event {
            Event::Missed {
                caller_id, invite, ..
            } => Some((caller_id, invite)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn do_not_disturb_turns_calls_and_invitations_away_as_missed() {
    use crate::{api::UserStatus, presence::PresenceEvent};
    let (mut worker, mut outgoing, events) = connected_worker(UserStatus::Dnd);
    worker
        .presence_event(PresenceEvent::Message {
            kind: "incoming_call".into(),
            payload: serde_json::json!({"caller_id": "mira"}),
        })
        .await
        .unwrap();
    let reply = outgoing.try_recv().unwrap();
    assert_eq!(reply["type"], "call_reject");
    assert_eq!(reply["payload"]["caller_id"], "mira");
    assert_eq!(reply["payload"]["reject_reason"], "do-not-disturb");
    assert!(worker.incoming_call.is_none(), "it must not ring");

    worker
        .presence_event(PresenceEvent::Message {
            kind: "incoming_invite".into(),
            payload: serde_json::json!({"inviter_id": "jo", "invite_id": "i1"}),
        })
        .await
        .unwrap();
    let reply = outgoing.try_recv().unwrap();
    assert_eq!(reply["type"], "invite_reject");
    assert_eq!(reply["payload"]["invite_id"], "i1");
    assert_eq!(reply["payload"]["reject_reason"], "do-not-disturb");
    assert!(worker.incoming_invite.is_none());
    assert_eq!(
        missed_events(&events),
        [("mira".to_owned(), false), ("jo".to_owned(), true)]
    );
}

#[tokio::test]
async fn away_still_rings() {
    use crate::{api::UserStatus, presence::PresenceEvent};
    let (mut worker, mut outgoing, events) = connected_worker(UserStatus::Away);
    worker
        .presence_event(PresenceEvent::Message {
            kind: "incoming_call".into(),
            payload: serde_json::json!({"caller_id": "mira"}),
        })
        .await
        .unwrap();
    assert!(outgoing.try_recv().is_err());
    assert_eq!(worker.incoming_call.as_deref(), Some("mira"));
    assert!(missed_events(&events).is_empty());
}

#[tokio::test]
async fn a_call_turned_away_says_why() {
    use crate::presence::PresenceEvent;
    let (mut worker, _outgoing, events) = connected_worker(crate::api::UserStatus::Available);
    worker.call_peer = Some("mira".into());
    worker
        .presence_event(PresenceEvent::Message {
            kind: "call_reject".into(),
            payload: serde_json::json!({"caller_id": "mira", "reject_reason": "do-not-disturb"}),
        })
        .await
        .unwrap();
    let reasons: Vec<_> = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|event| match event {
            Event::Disconnected(reason) => Some(reason),
            _ => None,
        })
        .collect();
    // Leaving says the call ended; the reason comes last and is what stays shown.
    assert_eq!(
        reasons.last().map(String::as_str),
        Some("Your teammate has Do not disturb on, but they'll see that you tried")
    );
}
