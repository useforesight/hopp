export const CALL_ALERT_WINDOW_LABEL = "callAlert";
export const CALL_ALERT_STATE_EVENT = "call-alert-state";
export const CALL_ALERT_READY_EVENT = "call-alert-ready";
export const CALL_ALERT_ACTION_EVENT = "call-alert-action";

export type CallAlertPerson = {
  id: string;
  firstName: string;
  lastName: string;
  avatarUrl: string | null;
};

export type CallAlert =
  | { kind: "ringing" | "connecting" | "missed" | "invite" | "missed-invite"; caller: CallAlertPerson }
  | { kind: "notice"; title: string; body: string; actionLabel?: string };

export type CallAlertAction = "answer" | "decline" | "call-back" | "dismiss" | "notice-action";

export type CallAlertEnvelope = { id: number; alert: CallAlert };
export type CallAlertActionEvent = { id: number; action: CallAlertAction };
