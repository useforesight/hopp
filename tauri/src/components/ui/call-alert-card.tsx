import clsx from "clsx";
import { useEffect, useState } from "react";
import { HiMiniPhone, HiMiniPhoneArrowDownLeft, HiMiniPhoneXMark } from "react-icons/hi2";
import { Button } from "./button";
import { HoppAvatar } from "./hopp-avatar";
import { CallAlert, CallAlertAction } from "@/lib/call-alert-types";
import { describeMissedCall, MissedCall } from "@/lib/missed-calls";
import { cn } from "@/lib/utils";

const CARD_CLASS_NAME =
  "flex h-full w-full flex-col justify-between gap-2.5 overflow-hidden rounded-2xl bg-raised p-3 text-foreground ring-1 ring-inset ring-border-strong animate-in fade-in-0 zoom-in-95 duration-200";

const SECONDARY_ACTION_CLASS_NAME = "h-8 flex-1 gap-1.5 rounded-[10px]";
const ANSWER_ACTION_CLASS_NAME =
  "h-8 flex-1 gap-1.5 rounded-[10px] bg-success text-success-foreground shadow-card hover:bg-success/90";

const PERSON_SUBTITLES = {
  ringing: "Calling you…",
  connecting: "Connecting…",
  invite: "Invited you to join a call",
};

/** Stays up to date while it's on screen, as a missed call can sit there past midnight. */
export const MissedCallText = ({ missedCall }: { missedCall: MissedCall }) => {
  const [, refresh] = useState(0);
  useEffect(() => {
    const interval = setInterval(() => refresh((count) => count + 1), 60_000);
    return () => clearInterval(interval);
  }, []);
  return <>{describeMissedCall(missedCall)}</>;
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
      <div className={cn(CARD_CLASS_NAME, "gap-1.5")}>
        <div className="flex min-w-0 flex-col">
          <span className="truncate text-sm leading-5 font-semibold tracking-[-0.005em]">{alert.title}</span>
          <span className="line-clamp-2 text-xs leading-[17px] text-muted-foreground">{alert.body}</span>
        </div>
        <div className="flex flex-row gap-1.5">
          <Button
            variant="secondary"
            size="sm"
            onClick={() => onAction("dismiss")}
            className={SECONDARY_ACTION_CLASS_NAME}
          >
            Dismiss
          </Button>
          {alert.actionLabel && (
            <Button size="sm" onClick={() => onAction("notice-action")} className={SECONDARY_ACTION_CLASS_NAME}>
              {alert.actionLabel}
            </Button>
          )}
        </div>
      </div>
    );
  }

  const { caller } = alert;
  const callerName = `${caller.firstName} ${caller.lastName}`.trim() || "Someone";
  const isAskingToJoin = alert.kind === "ringing" || alert.kind === "invite";
  const isMissed = alert.kind === "missed" || alert.kind === "missed-invite";

  return (
    <div className={cn(CARD_CLASS_NAME, alert.kind === "connecting" && "justify-center")}>
      <div className="flex min-w-0 flex-row items-center gap-2.5 pl-0.5">
        <div className="relative shrink-0">
          {isAskingToJoin && (
            <span className="absolute inset-0 animate-ping rounded-[11px] bg-success/50 [animation-duration:1.6s]" />
          )}
          <div className="relative">
            <HoppAvatar src={caller.avatarUrl ?? undefined} firstName={caller.firstName} lastName={caller.lastName} />
          </div>
        </div>
        <div className="flex min-w-0 flex-col">
          <span className="truncate text-sm leading-5 font-semibold tracking-[-0.005em]">{callerName}</span>
          <span className={clsx("truncate text-xs leading-4", isMissed ? "text-destructive" : "text-muted-foreground")}>
            {isMissed ?
              <MissedCallText missedCall={alert.missedCall} />
            : PERSON_SUBTITLES[alert.kind]}
          </span>
        </div>
      </div>
      {alert.kind !== "connecting" && (
        <div className="flex flex-row gap-1.5">
          {(alert.kind === "ringing" || alert.kind === "invite") && (
            <>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => onAction("decline")}
                className={clsx(
                  SECONDARY_ACTION_CLASS_NAME,
                  "bg-destructive-soft text-destructive hover:bg-destructive hover:text-destructive-foreground",
                )}
              >
                <HiMiniPhoneXMark className="size-4" /> {alert.kind === "ringing" ? "Decline" : "Not now"}
              </Button>
              <Button size="sm" onClick={() => onAction("answer")} className={ANSWER_ACTION_CLASS_NAME}>
                <HiMiniPhoneArrowDownLeft className="size-4 min-h-4 min-w-4" />
                {alert.kind === "ringing" ? "Answer" : "Join"}
              </Button>
            </>
          )}
          {alert.kind === "missed" && (
            <>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => onAction("dismiss")}
                className={SECONDARY_ACTION_CLASS_NAME}
              >
                Dismiss
              </Button>
              {alert.canCallBack && (
                <Button size="sm" onClick={() => onAction("call-back")} className={ANSWER_ACTION_CLASS_NAME}>
                  <HiMiniPhone className="size-4 min-h-4 min-w-4" /> Call back
                </Button>
              )}
            </>
          )}
          {alert.kind === "missed-invite" && (
            <>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => onAction("dismiss")}
                className={SECONDARY_ACTION_CLASS_NAME}
              >
                Dismiss
              </Button>
              <Button size="sm" onClick={() => onAction("answer")} className={ANSWER_ACTION_CLASS_NAME}>
                <HiMiniPhoneArrowDownLeft className="size-4 min-h-4 min-w-4" /> Join
              </Button>
            </>
          )}
        </div>
      )}
    </div>
  );
};
