import { emitTo, listen } from "@tauri-apps/api/event";
import toast from "react-hot-toast";
import { CallAlertCard } from "@/components/ui/call-alert-card";
import { typedInvoke } from "@/core_payloads";
import { tauriUtils } from "@/windows/window-utils";
import {
  CALL_ALERT_ACTION_EVENT,
  CALL_ALERT_READY_EVENT,
  CALL_ALERT_STATE_EVENT,
  CALL_ALERT_WINDOW_LABEL,
  CallAlert,
  CallAlertAction,
  CallAlertActionEvent,
  CallAlertEnvelope,
} from "./call-alert-types";

const FALLBACK_TOAST_ID = "call-alert-fallback";

type CallAlertActionHandler = (action: CallAlertAction) => void;
type ActiveCallAlert = {
  id: number;
  alert: CallAlert;
  onAction: CallAlertActionHandler;
  noticeKey?: string;
};
type CallNotice = {
  key: string;
  title: string;
  body: string;
  actionLabel?: string;
  onAction?: () => void;
  autoDismissMs?: number;
};

let activeCallAlert: ActiveCallAlert | null = null;
let nextCallAlertId = 1;
let autoDismissTimeout: ReturnType<typeof setTimeout> | undefined;
let usingFallbackToast = false;
let windowCommands: Promise<unknown> = Promise.resolve();
let showNextCallAlert: () => void = () => {};

/** Called whenever the card closes, in case something else was waiting for the space. */
export const setNextCallAlertHandler = (handler: () => void) => {
  showNextCallAlert = handler;
};

// Each command runs on its own thread in Tauri, so a hide and the show right
// after it could otherwise swap places and leave the card hidden.
const queueWindowCommand = (command: () => Promise<unknown>) => {
  windowCommands = windowCommands.then(command, command);
  return windowCommands;
};

const renderFallbackToast = () => {
  if (!activeCallAlert) {
    toast.dismiss(FALLBACK_TOAST_ID);
    return;
  }
  const { alert, onAction } = activeCallAlert;
  toast(
    () => (
      <div className="h-[116px] w-[320px]">
        <CallAlertCard alert={alert} onAction={onAction} />
      </div>
    ),
    { id: FALLBACK_TOAST_ID, duration: Infinity, position: "bottom-center", style: { padding: 0 } },
  );
};

const publishActiveCallAlert = () => {
  const envelope: CallAlertEnvelope | null =
    activeCallAlert ? { id: activeCallAlert.id, alert: activeCallAlert.alert } : null;
  emitTo(CALL_ALERT_WINDOW_LABEL, CALL_ALERT_STATE_EVENT, envelope).catch((error) => {
    console.error("Failed to update call alert window:", error);
  });
  if (usingFallbackToast) renderFallbackToast();
};

const setActiveCallAlert = (alert: CallAlert, onAction: CallAlertActionHandler, noticeKey?: string) => {
  clearTimeout(autoDismissTimeout);
  activeCallAlert = { id: nextCallAlertId++, alert, onAction, noticeKey };
  publishActiveCallAlert();
};

export const getActiveCallAlertKind = () => activeCallAlert?.alert.kind ?? null;

/** Answer or decline from outside the card, if what it shows is someone calling. */
export const pressCallAlert = (action: "answer" | "decline") => {
  const kind = activeCallAlert?.alert.kind;
  if (kind !== "ringing" && kind !== "invite") return false;
  activeCallAlert?.onAction(action);
  return true;
};

export const getShownMissedCallerId = () => {
  const alert = activeCallAlert?.alert;
  return alert?.kind === "missed" || alert?.kind === "missed-invite" ? alert.caller.id : null;
};

export const showCallAlert = (alert: CallAlert, onAction: CallAlertActionHandler, noticeKey?: string) => {
  setActiveCallAlert(alert, onAction, noticeKey);
  queueWindowCommand(() => typedInvoke("show_call_alert_window")).catch((error) => {
    // A call must never go unnoticed because this window failed to open.
    console.error("Failed to show call alert window:", error);
    usingFallbackToast = true;
    renderFallbackToast();
    tauriUtils.showWindow("main");
  });
};

/** Changes what the card says without moving it. */
export const updateCallAlert = (alert: CallAlert, onAction: CallAlertActionHandler) => {
  if (!activeCallAlert) {
    showCallAlert(alert, onAction);
    return;
  }
  setActiveCallAlert(alert, onAction);
};

export const closeCallAlert = () => {
  clearTimeout(autoDismissTimeout);
  activeCallAlert = null;
  publishActiveCallAlert();
  usingFallbackToast = false;
  toast.dismiss(FALLBACK_TOAST_ID);
  queueWindowCommand(() => typedInvoke("close_call_alert_window")).catch((error) => {
    console.error("Failed to close call alert window:", error);
  });
  showNextCallAlert();
};

export const showCallNotice = ({ key, title, body, actionLabel, onAction, autoDismissMs }: CallNotice) => {
  // Someone waiting for an answer matters more than a notice. A missed call doesn't need to
  // hold its place, as it's kept in the list and comes back when the notice closes.
  const activeKind = getActiveCallAlertKind();
  if (activeKind === "ringing" || activeKind === "invite" || activeKind === "connecting") {
    console.warn(`Not showing "${title}" over an incoming call`);
    return;
  }

  showCallAlert(
    { kind: "notice", title, body, actionLabel },
    (action) => {
      closeCallAlert();
      if (action === "notice-action") onAction?.();
    },
    key,
  );

  if (autoDismissMs !== undefined) {
    autoDismissTimeout = setTimeout(closeCallAlert, autoDismissMs);
  }
};

export const closeCallNotice = (key: string) => {
  if (activeCallAlert?.noticeKey === key) closeCallAlert();
};

export const setupCallAlertBridge = () => {
  const unlisteners = [
    listen(CALL_ALERT_READY_EVENT, publishActiveCallAlert),
    listen<CallAlertActionEvent>(CALL_ALERT_ACTION_EVENT, (event) => {
      // The card can be a step behind, and an old click must not answer a newer alert.
      if (activeCallAlert?.id === event.payload.id) activeCallAlert.onAction(event.payload.action);
    }),
  ];
  return () => unlisteners.forEach((unlisten) => unlisten.then((stopListening) => stopListening()));
};
