import { differenceInSeconds, fromUnixTime } from "date-fns";
import { sounds } from "@/constants/sounds";
import { TIncomingCallMessage, TIncomingInviteMessage, TRejectCallMessage, TWebSocketMessage } from "@/payloads";
import { socketService } from "@/services/socket";
import useStore, { CallState, ParticipantRole } from "@/store/store";
import { tauriUtils } from "@/windows/window-utils";
import {
  closeCallAlert,
  getActiveCallAlertKind,
  getShownMissedCallerId,
  setNextCallAlertHandler,
  showCallAlert,
  showCallNotice,
  updateCallAlert,
} from "./call-alert";
import { CallAlertPerson } from "./call-alert-types";
import { abandonCall } from "./call-recovery";
import { addMissedCall, loadMissedCalls, MissedCall } from "./missed-calls";

const MAX_RING_AGE_S = 60;
// Callers give up after 65 seconds and say so. This covers that message getting lost.
const RING_TIMEOUT_MS = 70_000;
const CALL_TOKENS_TIMEOUT_MS = 10_000;
const CALLBACK_HANDOFF_MS = 3_000;
const CALL_TOKENS_HANDLER_ID = "call_tokens_callback";

type JoinCall = (userId: string) => Promise<boolean>;

let ringTimeout: ReturnType<typeof setTimeout> | undefined;
let cancelPendingAnswer: (() => void) | undefined;
let activeInviteId: string | null = null;
let joinCall: JoinCall = async () => false;
// People who rang or invited us during this call. The server sends the same
// call_end for "gave up ringing" and for "last other person left the room", and a
// room can be down to someone we never knew was in it, so only these are ignored.
const ringersDuringCall = new Set<string>();

/** The join flow is a React hook, so the main window hands it over. */
export const setJoinCallHandler = (handler: JoinCall) => {
  joinCall = handler;
};

const personFromUserId = (userId: string): CallAlertPerson => {
  const teammate = useStore.getState().teammates?.find((user) => user.id === userId);
  return {
    id: userId,
    firstName: teammate?.first_name ?? "",
    lastName: teammate?.last_name ?? "",
    avatarUrl: teammate?.avatar_url ?? null,
  };
};

const isTooOld = (initiatedAt: number | null | undefined) => {
  if (initiatedAt == null) return false;
  return differenceInSeconds(new Date(), fromUnixTime(initiatedAt)) > MAX_RING_AGE_S;
};

const isCallPeer = (callTokens: CallState, userId: string) =>
  callTokens.participant === userId || callTokens.participants.some((peer) => peer.identity.includes(userId));

const stopRinging = () => {
  sounds.incomingCall.stop();
  clearTimeout(ringTimeout);
  ringTimeout = undefined;
};

const rejectCall = (callerId: string, rejectReason: "rejected" | "in-call") => {
  socketService.send({
    type: "call_reject",
    payload: { caller_id: callerId, reject_reason: rejectReason },
  } as TRejectCallMessage);
};

const rejectInvite = (inviterId: string, inviteId: string) => {
  socketService.send({ type: "invite_reject", payload: { inviter_id: inviterId, invite_id: inviteId } });
};

export const requestCallback = (userId: string) => {
  const { setTab, setCallbackUserId } = useStore.getState();
  setTab("user-list");
  tauriUtils.showWindow("main");
  setCallbackUserId(userId);
  // Teammates who aren't in the list can't pick this up, and it mustn't
  // start a surprise call whenever their row appears later on.
  setTimeout(() => {
    if (useStore.getState().callbackUserId === userId) setCallbackUserId(null);
  }, CALLBACK_HANDOFF_MS);
};

export const clearMissedCall = (callerId: string) => {
  const { missedCalls, setMissedCalls } = useStore.getState();
  setMissedCalls(missedCalls.filter((call) => call.callerId !== callerId));
  if (getShownMissedCallerId() === callerId) closeCallAlert();
};

/**
 * Puts the latest missed call on the card. Someone ringing or a call being placed comes first. So does a call
 * in progress, unless this miss has just happened, as the card is then the only way to find out in time.
 */
const showLatestMissedCall = (isNewMiss = false) => {
  const { missedCalls, teammates, callTokens, calling, callbackUserId, incomingCallCallerId, incomingInviteInviterId } =
    useStore.getState();
  const latest = missedCalls[0];
  if (!latest || incomingCallCallerId || incomingInviteInviterId || calling || callbackUserId) return;

  const activeKind = getActiveCallAlertKind();
  // Connecting with tokens means the call is starting. Without them, the caller has just hung up.
  if (activeKind === "notice" || (activeKind === "connecting" && callTokens)) return;
  if (!isNewMiss && (!teammates || (callTokens && !getShownMissedCallerId()))) return;

  const { callerId } = latest;
  const caller = personFromUserId(callerId);
  if (latest.kind === "invite") {
    updateCallAlert({ kind: "missed-invite", caller, missedCall: latest }, (action) => {
      clearMissedCall(callerId);
      if (action === "answer") joinInvitedCall(callerId, null);
    });
  } else {
    updateCallAlert({ kind: "missed", caller, missedCall: latest, canCallBack: !callTokens }, (action) => {
      if (action === "call-back") requestCallback(callerId);
      clearMissedCall(callerId);
    });
  }
};

/** Everything that should have rung but couldn't comes through here, so no attempt goes unnoticed. */
const recordMissedCall = (callerId: string, kind: MissedCall["kind"], at = Date.now()) => {
  const { missedCalls, setMissedCalls } = useStore.getState();
  setMissedCalls(addMissedCall(missedCalls, callerId, kind, at));
  showLatestMissedCall(true);
};

/** The caller gave up. It stays on the card, so the call isn't lost if nobody was at the screen. */
const markCallMissed = (callerId: string) => {
  const { incomingCallCallerId, setIncomingCallCallerId } = useStore.getState();
  if (incomingCallCallerId !== callerId) return;

  stopRinging();
  cancelPendingAnswer?.();
  setIncomingCallCallerId(null);
  recordMissedCall(callerId, "call");
};

const startCallFromTokens = async (
  callerId: string,
  tokens: Extract<TWebSocketMessage, { type: "call_tokens" }>["payload"],
  isCancelled: () => boolean,
) => {
  const { startMic, startCamera } = await tauriUtils.getCallStartPreferences();
  if (isCancelled()) return;

  sounds.callAccepted.play();
  cancelPendingAnswer = undefined;
  const { setCallTokens, setIncomingCallCallerId } = useStore.getState();
  setCallTokens({
    ...tokens,
    timeStarted: new Date(),
    hasAudioEnabled: startMic,
    hasCameraEnabled: startCamera,
    role: ParticipantRole.NONE,
    isRemoteControlEnabled: true,
    participants: [],
    isInitialisingCall: true,
    micLevel: 0,
  });
  setIncomingCallCallerId(null);

  try {
    await tauriUtils.callStarted(tokens.audioToken, tokens.videoToken);
  } catch (error) {
    console.error("Failed to start call:", error);
    closeCallAlert();
    abandonCall(callerId, "Couldn't start the call", "Hopp couldn't start your audio and video. Try calling back.");
    return;
  }

  closeCallAlert();
  tauriUtils.showWindow("main");
};

const answerCall = (callerId: string, caller: CallAlertPerson) => {
  stopRinging();
  updateCallAlert({ kind: "connecting", caller }, () => {});

  let cancelled = false;
  const stopWaitingForTokens = () => {
    clearTimeout(tokensTimeout);
    socketService.removeHandler(CALL_TOKENS_HANDLER_ID);
  };
  const tokensTimeout = setTimeout(() => {
    cancelled = true;
    stopWaitingForTokens();
    cancelPendingAnswer = undefined;
    rejectCall(callerId, "rejected");
    useStore.getState().setIncomingCallCallerId(null);
    closeCallAlert();
    showCallNotice({
      key: "call-failed",
      title: "Couldn't connect the call",
      body: "The server didn't respond in time. Check your connection and try calling back.",
    });
  }, CALL_TOKENS_TIMEOUT_MS);

  cancelPendingAnswer = () => {
    cancelled = true;
    stopWaitingForTokens();
    cancelPendingAnswer = undefined;
  };

  socketService.on(CALL_TOKENS_HANDLER_ID, (data: TWebSocketMessage) => {
    if (data.type !== "call_tokens") return;
    stopWaitingForTokens();
    startCallFromTokens(callerId, data.payload, () => cancelled);
  });

  socketService.send({ type: "call_accept", payload: { caller_id: callerId } } as TWebSocketMessage);
};

const endActiveInvite = () => {
  stopRinging();
  activeInviteId = null;
  useStore.getState().setIncomingInviteInviterId(null);
};

const joinInvitedCall = async (inviterId: string, inviteId: string | null) => {
  closeCallAlert();
  const joined = await joinCall(inviterId);
  if (inviteId) {
    socketService.send({
      type: joined ? "invite_accept" : "invite_reject",
      payload: { inviter_id: inviterId, invite_id: inviteId },
    });
  }
  if (!joined) {
    showCallNotice({
      key: "call-failed",
      title: "Couldn't join the call",
      body: "The call may have ended already.",
    });
  }
};

const markInviteMissed = (inviterId: string) => {
  endActiveInvite();
  recordMissedCall(inviterId, "invite");
};

const ringForInvite = (inviterId: string, inviteId: string) => {
  useStore.getState().setIncomingInviteInviterId(inviterId);
  activeInviteId = inviteId;
  sounds.incomingCall.play();
  clearTimeout(ringTimeout);
  ringTimeout = setTimeout(() => markInviteMissed(inviterId), RING_TIMEOUT_MS);

  const caller = personFromUserId(inviterId);
  showCallAlert({ kind: "invite", caller }, (action) => {
    endActiveInvite();
    if (action === "answer") {
      joinInvitedCall(inviterId, inviteId);
    } else {
      rejectInvite(inviterId, inviteId);
      closeCallAlert();
    }
  });
};

const ringForCall = (callerId: string) => {
  const { incomingCallCallerId, incomingInviteInviterId, setIncomingCallCallerId } = useStore.getState();

  if (incomingCallCallerId === callerId) return;
  // Set first, so whoever this displaces is only recorded and doesn't flash up on the card
  setIncomingCallCallerId(callerId);
  if (incomingCallCallerId) {
    cancelPendingAnswer?.();
    rejectCall(incomingCallCallerId, "in-call");
    recordMissedCall(incomingCallCallerId, "call");
  }
  if (incomingInviteInviterId && activeInviteId) {
    rejectInvite(incomingInviteInviterId, activeInviteId);
    endActiveInvite();
    recordMissedCall(incomingInviteInviterId, "invite");
  }

  sounds.incomingCall.play();
  clearTimeout(ringTimeout);
  ringTimeout = setTimeout(() => markCallMissed(callerId), RING_TIMEOUT_MS);

  const caller = personFromUserId(callerId);
  showCallAlert({ kind: "ringing", caller }, (action) => {
    if (action === "answer") {
      answerCall(callerId, caller);
    } else {
      stopRinging();
      rejectCall(callerId, "rejected");
      useStore.getState().setIncomingCallCallerId(null);
      closeCallAlert();
    }
  });
};

const handleIncomingCall = (data: TWebSocketMessage) => {
  if (data.type !== "incoming_call") return;

  const { caller_id: callerId, initiated_at: initiatedAt } = (data as TIncomingCallMessage).payload;
  const { callTokens, updateInProgress, setCallTokens } = useStore.getState();
  if (callTokens) ringersDuringCall.add(callerId);

  if (isTooOld(initiatedAt)) {
    console.warn("Not ringing for a stale incoming call");
    recordMissedCall(callerId, "call", initiatedAt! * 1000);
    return;
  }

  if (updateInProgress) {
    rejectCall(callerId, "rejected");
    recordMissedCall(callerId, "call");
    return;
  }

  if (callTokens) {
    // The participant we're in a call with is calling again, so they must have
    // lost the call without us getting a termination signal.
    if (callTokens.participant !== callerId) {
      rejectCall(callerId, "in-call");
      if (!isCallPeer(callTokens, callerId)) recordMissedCall(callerId, "call");
      return;
    }
    console.log("Received call from current participant - closing current call and accepting new one");
    setCallTokens(null);
    tauriUtils.endCallCleanup();
  }

  ringForCall(callerId);
};

const handleCallEnd = (data: TWebSocketMessage) => {
  if (data.type !== "call_end") return;

  const senderId = data.payload.participant_id;
  const { incomingCallCallerId, incomingInviteInviterId, callTokens, user, setCallTokens } = useStore.getState();

  if (incomingCallCallerId && (!senderId || incomingCallCallerId === senderId)) {
    markCallMissed(incomingCallCallerId);
  }
  if (incomingInviteInviterId && (!senderId || incomingInviteInviterId === senderId)) {
    markInviteMissed(incomingInviteInviterId);
  }

  if (!callTokens) return;
  // Someone giving up on ringing us mustn't end the call we're in with someone else.
  const wasRingingUs = senderId === incomingCallCallerId || senderId === incomingInviteInviterId;
  if (senderId && !isCallPeer(callTokens, senderId) && (wasRingingUs || ringersDuringCall.has(senderId))) {
    console.log(`Ignoring call_end from ${senderId}, who was only ringing us`);
    return;
  }

  const participantId = callTokens.participant || "";
  const roomId = callTokens.room?.id || data.payload.call_id || "";
  const teamId = user?.team_id?.toString() || "";

  setCallTokens(null);
  // Close screen share window
  tauriUtils.endCallCleanup();

  // Show feedback window if not disabled
  if (participantId && teamId) {
    tauriUtils.showFeedbackWindowIfEnabled(teamId, roomId, participantId);
  }
};

const handleIncomingInvite = (data: TWebSocketMessage) => {
  if (data.type !== "incoming_invite") return;

  const {
    inviter_id: inviterId,
    invite_id: inviteId,
    initiated_at: initiatedAt,
  } = (data as TIncomingInviteMessage).payload;
  if (useStore.getState().callTokens) ringersDuringCall.add(inviterId);

  if (isTooOld(initiatedAt)) {
    console.warn("Not ringing for a stale incoming invite");
    recordMissedCall(inviterId, "invite", initiatedAt! * 1000);
    return;
  }

  const { incomingCallCallerId, incomingInviteInviterId, updateInProgress } = useStore.getState();
  if (incomingCallCallerId || updateInProgress) {
    rejectInvite(inviterId, inviteId);
    recordMissedCall(inviterId, "invite");
    return;
  }
  if (incomingInviteInviterId && activeInviteId && activeInviteId !== inviteId) {
    rejectInvite(incomingInviteInviterId, activeInviteId);
    if (incomingInviteInviterId !== inviterId) recordMissedCall(incomingInviteInviterId, "invite");
  }

  ringForInvite(inviterId, inviteId);
};

const handleInviteCancel = (data: TWebSocketMessage) => {
  if (data.type !== "invite_cancel" || data.payload.invite_id !== activeInviteId) return;
  const { incomingInviteInviterId } = useStore.getState();
  if (incomingInviteInviterId) markInviteMissed(incomingInviteInviterId);
};

export const setupIncomingCallHandling = () => {
  socketService.on("incoming_call", handleIncomingCall);
  socketService.on("call_end", handleCallEnd);
  socketService.on("incoming_invite", handleIncomingInvite);
  socketService.on("invite_cancel", handleInviteCancel);
  setNextCallAlertHandler(showLatestMissedCall);

  const unsubscribeFromStore = useStore.subscribe((state, previousState) => {
    if (previousState.authToken && !state.authToken) {
      state.setMissedCalls([]);
      if (getShownMissedCallerId()) closeCallAlert();
    }
    // This is what gives missed calls a name, so saved ones wait for it
    if (state.teammates && !previousState.teammates) showLatestMissedCall();

    if (!state.callTokens) {
      ringersDuringCall.clear();
      if (previousState.callTokens) showLatestMissedCall();
      return;
    }
    // Once they've joined, their call_end can be the room emptying out.
    for (const ringer of ringersDuringCall) {
      if (isCallPeer(state.callTokens, ringer)) ringersDuringCall.delete(ringer);
    }
    for (const { callerId } of state.missedCalls) {
      if (isCallPeer(state.callTokens, callerId)) clearMissedCall(callerId);
    }

    // The card makes way for the call and comes back after it. The missed call itself stays listed.
    if (!previousState.callTokens && getShownMissedCallerId()) closeCallAlert();
  });
  useStore.getState().setMissedCalls(loadMissedCalls());
  showLatestMissedCall();

  return () => {
    socketService.removeHandler("incoming_call");
    socketService.removeHandler("call_end");
    socketService.removeHandler("incoming_invite");
    socketService.removeHandler("invite_cancel");
    setNextCallAlertHandler(() => {});
    unsubscribeFromStore();
  };
};
