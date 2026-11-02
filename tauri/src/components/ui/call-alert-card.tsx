import { HiMiniPhone, HiMiniPhoneArrowDownLeft, HiMiniPhoneXMark } from "react-icons/hi2";
import { Button } from "./button";
import { HoppAvatar } from "./hopp-avatar";
import { CallAlert, CallAlertAction } from "@/lib/call-alert-types";

const CARD_CLASS_NAME =
  "flex h-full w-full flex-col justify-between gap-2 overflow-hidden rounded-xl border border-black/10 bg-white p-3 text-black dark:border-white/10 dark:bg-zinc-900 dark:text-white";

const PERSON_SUBTITLES = {
  ringing: "Calling you...",
  connecting: "Connecting...",
  missed: "Missed call",
  invite: "Invited you to join a call",
  "missed-invite": "Missed invite to a call",
};

export const CallAlertCard = ({
  alert,
  onAction,
}: {
  alert: CallAlert;
  onAction: (action: CallAlertAction) => void;
}) => {
  if (alert.kind === "notice") {
    return (
      <div className={CARD_CLASS_NAME}>
        <div className="flex min-w-0 flex-col">
          <span className="truncate text-sm font-medium">{alert.title}</span>
          <span className="line-clamp-2 text-xs text-muted-foreground">{alert.body}</span>
        </div>
        <div className="flex flex-row justify-end gap-1">
          <Button variant="ghost" size="sm" onClick={() => onAction("dismiss")}>
            Dismiss
          </Button>
          {alert.actionLabel && (
            <Button variant="outline" size="sm" onClick={() => onAction("notice-action")}>
              {alert.actionLabel}
            </Button>
          )}
        </div>
      </div>
    );
  }

  const { caller } = alert;
  const callerName = `${caller.firstName} ${caller.lastName}`.trim() || "Someone";

  return (
    <div className={CARD_CLASS_NAME}>
      <div className="flex min-w-0 flex-row items-center gap-2">
        <HoppAvatar src={caller.avatarUrl ?? undefined} firstName={caller.firstName} lastName={caller.lastName} />
        <div className="flex min-w-0 flex-col">
          <span className="truncate text-sm font-medium">{callerName}</span>
          <span className="truncate text-xs text-muted-foreground">{PERSON_SUBTITLES[alert.kind]}</span>
        </div>
      </div>
      {alert.kind !== "connecting" && (
        <div className="flex flex-row justify-end gap-1">
          {(alert.kind === "ringing" || alert.kind === "invite") && (
            <>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => onAction("decline")}
                className="flex flex-row gap-2 hover:bg-red-100 dark:hover:bg-red-950"
              >
                <HiMiniPhoneXMark className="size-4" /> {alert.kind === "ringing" ? "Decline" : "Not now"}
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => onAction("answer")}
                className="flex flex-row gap-2 px-4 hover:text-green-700"
              >
                <HiMiniPhoneArrowDownLeft className="size-4 min-h-4 min-w-4" />
                {alert.kind === "ringing" ? "Answer" : "Join"}
              </Button>
            </>
          )}
          {alert.kind === "missed" && (
            <>
              <Button variant="ghost" size="sm" onClick={() => onAction("dismiss")}>
                Dismiss
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => onAction("call-back")}
                className="flex flex-row gap-2 px-4 hover:text-green-700"
              >
                <HiMiniPhone className="size-4 min-h-4 min-w-4" /> Call back
              </Button>
            </>
          )}
          {alert.kind === "missed-invite" && (
            <>
              <Button variant="ghost" size="sm" onClick={() => onAction("dismiss")}>
                Dismiss
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => onAction("answer")}
                className="flex flex-row gap-2 px-4 hover:text-green-700"
              >
                <HiMiniPhoneArrowDownLeft className="size-4 min-h-4 min-w-4" /> Join
              </Button>
            </>
          )}
        </div>
      )}
    </div>
  );
};
