import { listen } from "@tauri-apps/api/event";
import { relaunch } from "@tauri-apps/plugin-process";
import { typedInvoke } from "@/core_payloads";
import { socketService } from "@/services/socket";
import useStore, { ParticipantRole } from "@/store/store";
import { tauriUtils } from "@/windows/window-utils";
import { closeCallNotice, showCallNotice } from "./call-alert";

const RECONNECTED_NOTICE_MS = 4_000;
// A rejoin also fails when the core dies again under it, and that core's replacement rejoins again.
const NEXT_RESTART_GRACE_MS = 10_000;
const MIC_SILENT_NOTICE_KEY = "microphone-silent";

const showCallControls = () => {
  useStore.getState().setTab("call");
  tauriUtils.showWindow("main");
};

/** Backs out of a call that couldn't start, so nobody is left in it alone. */
export const abandonCall = (peerId: string | undefined, title: string, body: string) => {
  if (peerId) {
    socketService.send({ type: "call_end", payload: { participant_id: peerId } });
  }
  tauriUtils.endCallCleanup();
  useStore.getState().setCallTokens(null);
  showCallNotice({ key: "call-failed", title, body });
};

let coreRestartCount = 0;

const rejoinAfterCoreRestart = async () => {
  const thisRestart = ++coreRestartCount;
  const { callTokens, updateCallTokens } = useStore.getState();
  if (!callTokens) {
    console.log("Core process restarted outside of a call");
    return;
  }

  const wasSharingScreen = callTokens.role === ParticipantRole.SHARER;
  showCallNotice({
    key: "call-reconnecting",
    title: "Reconnecting your call",
    body: "Hopp's audio and video process restarted. Rejoining now.",
  });
  updateCallTokens({ isReconnecting: true, isInitialisingCall: true, participants: [], role: ParticipantRole.NONE });

  try {
    await tauriUtils.callStarted(callTokens.audioToken, callTokens.videoToken, {
      micOn: callTokens.hasAudioEnabled,
      cameraOn: callTokens.hasCameraEnabled,
    });
  } catch (error) {
    console.error("Failed to rejoin call after core restart:", error);
    await new Promise((resolve) => setTimeout(resolve, NEXT_RESTART_GRACE_MS));
    if (thisRestart !== coreRestartCount) return;
    if (!useStore.getState().callTokens) {
      closeCallNotice("call-reconnecting");
      return;
    }
    abandonCall(callTokens.participant, "Couldn't reconnect your call", "Call them back to carry on.");
    return;
  }

  if (thisRestart !== coreRestartCount) return;

  // The user may have hung up while this was reconnecting.
  if (!useStore.getState().callTokens) {
    tauriUtils.endCallCleanup();
    closeCallNotice("call-reconnecting");
    return;
  }

  updateCallTokens({ isReconnecting: false });
  showCallNotice({
    key: "call-reconnected",
    title: "You're back in the call",
    body: wasSharingScreen ? "Start sharing your screen again when you're ready." : "Your audio and video are back.",
    autoDismissMs: RECONNECTED_NOTICE_MS,
  });
};

export const setupCallRecoveryListeners = () => {
  const unlisteners = [
    listen("core_process_restarted", rejoinAfterCoreRestart),
    listen<string>("core_process_crashed", (event) => {
      console.error("Core process could not be restarted:", event.payload);
      const { callTokens, setCallTokens } = useStore.getState();
      if (callTokens) {
        socketService.send({ type: "call_end", payload: { participant_id: callTokens.participant } });
        setCallTokens(null);
        // No core is left to acknowledge the end of the call, which is what normally
        // lets go of the call shortcuts, the sleep prevention and the Dock icon.
        typedInvoke("end_call").catch((error) => console.error("Failed to clean up call:", error));
      }
      showCallNotice({
        key: "core-crashed",
        title: "Hopp's audio and video stopped",
        body: "Restarting them automatically didn't work. Restart Hopp to get them back.",
        actionLabel: "Restart Hopp",
        onAction: () => relaunch(),
      });
    }),
    listen<string>("core_camera_failed", (event) => {
      console.error("Camera failed:", event.payload);
      const { callTokens, updateCallTokens } = useStore.getState();
      if (!callTokens) return;
      updateCallTokens({ hasCameraEnabled: false });
      // The core gave up on capturing but still has the track published, and the
      // button below only offers Start once it's off here. Line the two up.
      typedInvoke("stop_camera").catch((error) => console.error("Failed to stop camera:", error));
      showCallNotice({
        key: "camera-failed",
        title: "Your camera stopped working",
        body: "Hopp tried restarting it. Turn it on again or pick another camera.",
        actionLabel: "Open controls",
        onAction: showCallControls,
      });
    }),
    listen<string>("core_microphone_silent", (event) => {
      const { callTokens } = useStore.getState();
      // A muted user wouldn't be heard anyway.
      if (!callTokens?.hasAudioEnabled) return;
      showCallNotice({
        key: MIC_SILENT_NOTICE_KEY,
        title: `No sound from ${event.payload || "your microphone"}`,
        body: "Hopp tried restarting it. Check it isn't muted or pick another one.",
        actionLabel: "Choose mic",
        onAction: showCallControls,
      });
    }),
    listen("core_microphone_recovered", () => closeCallNotice(MIC_SILENT_NOTICE_KEY)),
  ];
  return () => unlisteners.forEach((unlisten) => unlisten.then((stopListening) => stopListening()));
};
