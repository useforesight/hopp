import { HiMiniPhone, HiXMark } from "react-icons/hi2";
import { Button } from "./button";
import { MissedCallText } from "./call-alert-card";
import { HoppAvatar } from "./hopp-avatar";
import { SectionLabel } from "./page-header";
import { clearMissedCall, requestCallback } from "@/lib/incoming-call";
import { MissedCall } from "@/lib/missed-calls";
import { components } from "@/openapi";
import useStore from "@/store/store";

type Teammate = components["schemas"]["BaseUser"];

const MissedCallRow = ({ missedCall, teammate }: { missedCall: MissedCall; teammate?: Teammate }) => {
  const inACall = useStore((state) => state.callTokens !== null);
  const name = teammate ? `${teammate.first_name} ${teammate.last_name}`.trim() : "Someone";
  const { callerId } = missedCall;

  return (
    <div className="group grid w-full grid-cols-[max-content_minmax(0,1fr)_max-content] items-center gap-2.5 rounded-xl px-2 py-[7px] transition-colors duration-150 hover:bg-subtle">
      <HoppAvatar
        src={teammate?.avatar_url || undefined}
        firstName={teammate?.first_name ?? ""}
        lastName={teammate?.last_name ?? ""}
        status={teammate?.is_active ? "online" : "offline"}
      />
      <div className="flex flex-col justify-center overflow-hidden">
        <div className="truncate text-[13px] leading-[18px] font-medium text-foreground">{name}</div>
        <div className="truncate text-xs leading-4 text-destructive">
          <MissedCallText missedCall={missedCall} />
        </div>
      </div>
      <div className="flex flex-row items-center gap-1">
        {teammate?.is_active && !inACall && (
          <Button
            variant="ghost"
            aria-label={`Call ${name} back`}
            onClick={() => {
              requestCallback(callerId);
              clearMissedCall(callerId);
            }}
            className="h-7 gap-1 rounded-lg bg-success-soft px-2.5 text-xs font-semibold text-success transition-all duration-150 hover:bg-success hover:text-success-foreground"
          >
            <HiMiniPhone className="size-3" />
            Call
          </Button>
        )}
        <Button
          variant="ghost"
          aria-label={`Clear missed call from ${name}`}
          onClick={() => clearMissedCall(callerId)}
          className="size-7 rounded-lg p-0 text-faint-foreground hover:bg-subtle-strong hover:text-foreground"
        >
          <HiXMark className="size-3.5" />
        </Button>
      </div>
    </div>
  );
};

export const MissedCallsSection = ({ teammates }: { teammates: Teammate[] }) => {
  const missedCalls = useStore((state) => state.missedCalls);
  if (missedCalls.length === 0) return null;

  return (
    <section className="mb-1.5">
      <SectionLabel>Missed · {missedCalls.length}</SectionLabel>
      <div className="flex flex-col gap-0.5">
        {missedCalls.map((missedCall) => (
          <MissedCallRow
            key={missedCall.callerId}
            missedCall={missedCall}
            teammate={teammates.find((teammate) => teammate.id === missedCall.callerId)}
          />
        ))}
      </div>
    </section>
  );
};
