import "../../App.css";
import React, { useCallback, useEffect, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import { HiOutlineCpuChip, HiXMark } from "react-icons/hi2";
import { listen } from "@tauri-apps/api/event";
import { Button } from "@/components/ui/button";
import { typedInvoke, type AgentCard, type AgentCardAnswer } from "@/core_payloads";
import { initTheme } from "@/lib/theme";
import { cn } from "@/lib/utils";

initTheme();

/** Shown at once; the rest wait behind a "+N more" line. */
const SHOWN = 3;
/** A card that's just moved isn't what you were aiming at, and the answer can't be taken back. */
const STEADY_AFTER_MS = 500;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <CardsWindow />
  </React.StrictMode>,
);

function CardsWindow() {
  const [cards, setCards] = useState<AgentCard[]>([]);
  const movedAt = useRef(0);

  useEffect(() => {
    const show = (showing: AgentCard[]) => {
      movedAt.current = Date.now();
      setCards(showing);
    };
    let heard = false;
    const unlisten = listen<AgentCard[]>("cards_changed", (event) => {
      heard = true;
      show(event.payload);
    });
    // Anything heard while this is on its way is newer than its answer.
    unlisten.then(() => typedInvoke("cards_showing").then((showing) => heard || show(showing)));
    return () => {
      unlisten.then((stopListening) => stopListening());
    };
  }, []);

  const settle = useCallback((id: string, state: AgentCardAnswer) => {
    if (Date.now() - movedAt.current < STEADY_AFTER_MS) {
      // The click brought Hopp forward even though it didn't count.
      typedInvoke("cards_return_focus");
      return;
    }
    typedInvoke("cards_settle", { id, state });
  }, []);

  // Once a click has given this window the keyboard, Escape closes the newest card. Return
  // does nothing: someone typing elsewhere who hit it here would have said yes to something.
  useEffect(() => {
    const newest = cards[0];
    if (!newest) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") settle(newest.id, newest.question ? "rejected" : "dismissed");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [cards, settle]);

  useEffect(() => {
    const preventContextMenu = (event: MouseEvent) => event.preventDefault();
    document.addEventListener("contextmenu", preventContextMenu);
    return () => document.removeEventListener("contextmenu", preventContextMenu);
  }, []);

  const more = cards.length - SHOWN;
  return (
    <div className="flex h-full w-full flex-col gap-2">
      {cards.slice(0, SHOWN).map((card) => (
        <CardView key={card.id} card={card} onSettle={settle} />
      ))}
      {more > 0 && (
        <div className="flex h-[22px] shrink-0 items-center justify-center rounded-lg bg-raised text-xs text-muted-foreground ring-1 ring-inset ring-border-strong">
          +{more} more
        </div>
      )}
    </div>
  );
}

const CardView = ({ card, onSettle }: { card: AgentCard; onSettle: (id: string, state: AgentCardAnswer) => void }) => (
  <div
    role={card.question ? "alertdialog" : "status"}
    className={cn(
      "flex h-[136px] shrink-0 flex-col gap-1 overflow-hidden rounded-2xl bg-raised p-3 text-foreground ring-1 ring-inset animate-in fade-in-0 zoom-in-95 duration-200",
      card.question ? "ring-accent" : "ring-border-strong",
    )}
  >
    <div className="flex min-w-0 items-center gap-1.5 text-[11px] leading-4 text-muted-foreground">
      <HiOutlineCpuChip className="size-3 shrink-0" />
      <span className="min-w-0 flex-1 truncate">{card.sender ?? "Agent"}</span>
      <button
        aria-label="Close"
        title="Close"
        onClick={() => onSettle(card.id, "dismissed")}
        className="-m-0.5 shrink-0 rounded p-0.5 transition-colors hover:bg-subtle hover:text-foreground"
      >
        <HiXMark className="size-3" />
      </button>
    </div>
    {/* Nobody should answer a question they could only read part of, so all of it scrolls. */}
    <div className="selectable min-h-0 flex-1 overflow-y-auto pb-2 [mask-image:linear-gradient(to_bottom,black_calc(100%-8px),transparent)]">
      <p className="text-[13px] leading-[18px] font-semibold tracking-[-0.005em] break-words">{card.title}</p>
      {card.body && (
        <p className="mt-0.5 text-xs leading-[17px] whitespace-pre-line break-words text-muted-foreground">
          {card.body}
        </p>
      )}
    </div>
    {card.question && (
      <div className="flex justify-end gap-1.5">
        <Button
          variant="secondary"
          size="sm"
          onClick={() => onSettle(card.id, "rejected")}
          className="h-7 min-w-0 rounded-lg px-3"
        >
          <span className="truncate">{card.question.reject}</span>
        </Button>
        <Button size="sm" onClick={() => onSettle(card.id, "accepted")} className="h-7 min-w-0 rounded-lg px-3">
          <span className="truncate">{card.question.accept}</span>
        </Button>
      </div>
    )}
  </div>
);
