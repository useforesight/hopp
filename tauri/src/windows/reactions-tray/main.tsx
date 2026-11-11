import "../../App.css";
import React, { useCallback, useEffect, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Switch } from "@/components/ui/switch";
import { typedInvoke, type Reaction, type ReactionPerson } from "@/core_payloads";
import { cn } from "@/lib/utils";
import {
  CONFETTI_MOST,
  CONFETTI_STREAK_MS,
  EMOJI,
  FIREWORKS_FULL_MS,
  SOUNDS,
  rockets,
  setSoundsOn,
  soundsOn,
} from "../reactions/reactions";

/** A disco lasts about this long; until then the button ends it early. */
const DISCO_MS = 7000;
/** Longest an achievement's title or a ship-it name can be, as on the wire. */
const TEXT_MAX = 60;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ReactionsTray />
  </React.StrictMode>,
);

function send(reaction: Reaction) {
  typedInvoke("reactions_send", { reaction }).catch((e) => console.error("reactions: send", e));
}

function close(handBack: boolean) {
  typedInvoke("reactions_close_tray", { handBack }).catch((e) => console.error("reactions: close", e));
}

type Form = "achievement" | "ship_it" | null;

function ReactionsTray() {
  const [form, setForm] = useState<Form>(null);
  const [people, setPeople] = useState<ReactionPerson[]>([]);
  const [sounds, setSounds] = useState(soundsOn);
  const [discoUntil, setDiscoUntil] = useState(0);

  // Fresh each time it opens: who's in the call, and back to the start.
  useEffect(() => {
    const refresh = () => {
      setForm(null);
      setSounds(soundsOn());
      typedInvoke("reactions_people")
        .then(setPeople)
        .catch((e) => console.error("reactions: people", e));
    };
    refresh();
    const unlisten = listen("reactions_tray_opened", refresh);
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Clicking anywhere else closes it.
  useEffect(() => {
    const unlisten = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (!focused) close(false);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        if (form) setForm(null);
        else close(true);
        return;
      }
      if (form || event.metaKey || event.ctrlKey || event.altKey || event.repeat) return;
      const sound = SOUNDS[Number(event.key) - 1];
      if (sound && /^[1-9]$/.test(event.key)) {
        event.preventDefault();
        send({ kind: "sound", choice: sound.key });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [form]);

  // The disco button ends your disco while it's on.
  useEffect(() => {
    if (!discoUntil) return;
    const timer = setTimeout(() => setDiscoUntil(0), Math.max(0, discoUntil - Date.now()));
    return () => clearTimeout(timer);
  }, [discoUntil]);

  const toggleSounds = (on: boolean) => {
    setSoundsOn(on);
    setSounds(on);
  };

  return (
    <div className="flex h-screen w-screen p-1.5 select-none">
      <div className="flex flex-1 flex-col overflow-hidden rounded-[18px] border border-white/10 bg-[#17131f]/95 text-white shadow-[0_18px_50px_rgba(0,0,0,0.45)] backdrop-blur-xl">
        <div className="flex items-center justify-between px-4 pt-3 pb-2">
          <span className="text-[13px] font-semibold tracking-tight">Reactions</span>
          <label className="flex items-center gap-2 text-[11px] text-white/60">
            Sounds
            <Switch checked={sounds} onCheckedChange={toggleSounds} aria-label="Reaction sounds" />
          </label>
        </div>
        {form ?
          <TextForm form={form} people={people} onDone={() => setForm(null)} />
        : <div className="flex flex-col gap-3 px-3 pb-3">
            <div className="grid grid-cols-8 gap-1">
              {EMOJI.map((item) => (
                <button
                  key={item.key}
                  title={item.label}
                  onClick={() => send({ kind: "emoji", choice: item.key })}
                  className="grid aspect-square place-items-center rounded-xl text-[22px] transition-transform duration-150 ease-[cubic-bezier(0.2,1.6,0.4,1)] hover:scale-[1.18] hover:bg-white/10 active:scale-90"
                >
                  {item.emoji}
                </button>
              ))}
            </div>
            <Section title="Effects">
              <div className="grid grid-cols-4 gap-1.5">
                <ConfettiTile />
                <Tile emoji="🔔" label="Gong" onClick={() => send({ kind: "gong" })} />
                <Tile emoji="┻━┻" label="Flip table" small onClick={() => send({ kind: "table_flip" })} />
                <Tile emoji="👏" label="Slow clap" onClick={() => send({ kind: "clap" })} />
                <FireworksTile />
                <Tile emoji="🏆" label="Achievement" onClick={() => setForm("achievement")} />
                <Tile
                  emoji="🪩"
                  label={discoUntil ? "Stop disco" : "Disco"}
                  on={!!discoUntil}
                  onClick={() => {
                    if (discoUntil) {
                      send({ kind: "disco_end" });
                      setDiscoUntil(0);
                    } else {
                      send({ kind: "disco" });
                      setDiscoUntil(Date.now() + DISCO_MS);
                    }
                  }}
                />
                <Tile emoji="🚀" label="Ship it" onClick={() => setForm("ship_it")} />
              </div>
            </Section>
            <Section title="Soundboard">
              <div className="grid grid-cols-4 gap-1.5">
                {SOUNDS.map((sound, i) => (
                  <button
                    key={sound.key}
                    onClick={() => send({ kind: "sound", choice: sound.key })}
                    className="relative flex h-[46px] flex-col items-center justify-center rounded-xl bg-white/[0.06] transition-[transform,background-color] duration-150 hover:bg-white/[0.12] active:scale-95"
                  >
                    <span className="text-[17px] leading-none">{sound.emoji}</span>
                    <span className="mt-1 max-w-full truncate px-0.5 text-[9.5px] leading-tight text-white/65">
                      {sound.label}
                    </span>
                    {/* Not a <kbd>: App.css styles those as keycaps, over these classes. */}
                    <span className="absolute top-1 right-1.5 text-[9px] leading-none text-white/35">{i + 1}</span>
                  </button>
                ))}
              </div>
            </Section>
          </div>
        }
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <span className="px-1 text-[10.5px] font-semibold tracking-wide text-white/45 uppercase">{title}</span>
      {children}
    </div>
  );
}

type TileProps = {
  emoji: string;
  label: string;
  small?: boolean;
  on?: boolean;
  onClick?: () => void;
} & Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, "onClick">;

function Tile({ emoji, label, small, on, onClick, className, children, ...rest }: TileProps) {
  return (
    <button
      {...rest}
      onClick={onClick}
      className={cn(
        "relative flex h-[58px] flex-col items-center justify-center overflow-hidden rounded-xl bg-white/[0.06] transition-[transform,background-color] duration-150 hover:bg-white/[0.12] active:scale-95",
        on && "bg-fuchsia-500/30 hover:bg-fuchsia-500/40",
        className,
      )}
    >
      {children}
      <span className={cn("relative leading-none", small ? "text-[13px] font-bold" : "text-[22px]")}>{emoji}</span>
      <span className="relative mt-1.5 text-[10px] leading-tight text-white/70">{label}</span>
    </button>
  );
}

/** Quick taps build a streak, for a bigger burst each time. */
function ConfettiTile() {
  const last = useRef(0);
  const streak = useRef(0);
  return (
    <Tile
      emoji="🎉"
      label="Confetti"
      onClick={() => {
        const now = performance.now();
        streak.current = now - last.current < CONFETTI_STREAK_MS ? Math.min(streak.current + 1, CONFETTI_MOST) : 0;
        last.current = now;
        send({ kind: "confetti", level: streak.current });
      }}
    />
  );
}

/** Click for a few rockets, or hold to charge up to ten. */
function FireworksTile() {
  const [charge, setCharge] = useState(0);
  const pressed = useRef<number | null>(null);
  const frame = useRef(0);

  const tick = useCallback(() => {
    if (pressed.current === null) return;
    setCharge(Math.min(1, (performance.now() - pressed.current) / FIREWORKS_FULL_MS));
    frame.current = requestAnimationFrame(tick);
  }, []);

  const start = () => {
    if (pressed.current !== null) return;
    pressed.current = performance.now();
    frame.current = requestAnimationFrame(tick);
  };
  const release = (fire: boolean) => {
    if (pressed.current === null) return;
    const held = performance.now() - pressed.current;
    pressed.current = null;
    cancelAnimationFrame(frame.current);
    setCharge(0);
    if (fire) send({ kind: "fireworks", level: rockets(held) });
  };

  useEffect(() => () => cancelAnimationFrame(frame.current), []);

  return (
    <Tile
      emoji="🎆"
      label={charge > 0.05 ? `${rockets(charge * FIREWORKS_FULL_MS)} rockets` : "Fireworks"}
      title="Click for fireworks, or hold for more"
      onPointerDown={(event) => {
        event.currentTarget.setPointerCapture(event.pointerId);
        start();
      }}
      onPointerUp={() => release(true)}
      onPointerCancel={() => release(false)}
      onKeyDown={(event) => {
        if ((event.key === " " || event.key === "Enter") && !event.repeat) {
          event.preventDefault();
          start();
        }
      }}
      onKeyUp={(event) => {
        if (event.key === " " || event.key === "Enter") {
          event.preventDefault();
          release(true);
        }
      }}
    >
      <span
        className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-amber-400/45 to-fuchsia-500/25"
        style={{ height: `${charge * 100}%` }}
      />
    </Tile>
  );
}

/** An achievement's title and who it's for, or what shipped. */
function TextForm({
  form,
  people,
  onDone,
}: {
  form: "achievement" | "ship_it";
  people: ReactionPerson[];
  onDone: () => void;
}) {
  const [text, setText] = useState("");
  const [to, setTo] = useState("");
  const achievement = form === "achievement";

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    const trimmed = text.trim();
    send({
      kind: form,
      ...(trimmed && { text: trimmed }),
      ...(achievement && to && { to }),
    });
    onDone();
  };

  return (
    <form onSubmit={submit} className="flex flex-1 flex-col gap-3 px-4 pb-4">
      <div className="flex items-center gap-3 rounded-xl bg-white/[0.06] p-3">
        <span className="text-[30px] leading-none">{achievement ? "🏆" : "🚀"}</span>
        <div className="text-[12px] leading-snug text-white/70">
          {achievement ?
            "Award an achievement. Everyone sees it unlock."
          : "Launch a release. Everyone sees the countdown and lift-off."}
        </div>
      </div>
      <label className="flex flex-col gap-1.5">
        <span className="px-1 text-[10.5px] font-semibold tracking-wide text-white/45 uppercase">
          {achievement ? "Achievement" : "What shipped"}
        </span>
        <input
          autoFocus
          value={text}
          maxLength={TEXT_MAX}
          onChange={(event) => setText(event.target.value)}
          placeholder={achievement ? "Nailed it" : "v2.0"}
          className="h-9 rounded-lg border border-white/10 bg-white/[0.06] px-3 text-[13px] text-white outline-none placeholder:text-white/30 focus:border-fuchsia-400/60"
        />
      </label>
      {achievement && (
        <div className="flex flex-col gap-1.5">
          <span className="px-1 text-[10.5px] font-semibold tracking-wide text-white/45 uppercase">For</span>
          <div className="flex flex-wrap gap-1.5">
            {[{ identity: "", name: "Everyone" }, ...people].map((person) => (
              <button
                type="button"
                key={person.identity}
                onClick={() => setTo(person.identity)}
                className={cn(
                  "h-7 rounded-full border px-3 text-[12px] transition-colors",
                  to === person.identity ?
                    "border-fuchsia-400/70 bg-fuchsia-500/25 text-white"
                  : "border-white/10 bg-white/[0.04] text-white/70 hover:bg-white/10",
                )}
              >
                {person.name}
              </button>
            ))}
          </div>
        </div>
      )}
      <div className="mt-auto flex gap-2">
        <button
          type="button"
          onClick={onDone}
          className="h-9 flex-1 rounded-lg bg-white/[0.06] text-[13px] text-white/75 hover:bg-white/[0.12]"
        >
          Back
        </button>
        <button
          type="submit"
          className="h-9 flex-[2] rounded-lg bg-gradient-to-r from-fuchsia-500 to-amber-400 text-[13px] font-semibold text-white shadow-[0_6px_20px_rgba(217,70,239,0.35)] hover:brightness-110 active:scale-[0.98]"
        >
          {achievement ? "Unlock it" : "Launch"}
        </button>
      </div>
    </form>
  );
}
