// The reactions layer: plays reactions from the call over the whole screen, with sound (see
// `src-tauri/src/reactions.rs`). The effects in `effects/` are the approved prototypes; this
// tells them who's in the call, where the call window is, and what to play.
import { listen } from "@tauri-apps/api/event";
import { typedInvoke, type ReactionPlay } from "@/core_payloads";
import { SOUNDS, soundsOn } from "./reactions";
import "./effects/lib";
import "./effects/p1";
import "./effects/p2";
import "./effects/p3";
import "./effects/p4";
import "./effects/p5";
import "./effects/p6";
import "./effects/p7";
import "./effects/p8";
import "./effects/p9";
import "./effects/p10";

type Person = { key: string };
type Effect = {
  play: (H: Effects, who: Person, choice?: unknown) => void;
  clap: (H: Effects, who: Person) => void;
  stop: (H: Effects, who: Person) => void;
};
type Effects = {
  protos: Record<number, Effect>;
  people: Record<string, Person>;
  me: Person | null;
  person: (key: string, full?: string, colour?: [number, number, number]) => Person;
  setMuted: (muted: boolean) => void;
  wake: () => void;
  start: () => void;
  particleCount: () => number;
  clearParticles: () => void;
  ac: () => AudioContext;
};

const H = (window as unknown as { H: Effects }).H;
H.start();

/** Nothing has played for this long and nothing's left on screen: the layer can go. */
const IDLE_AFTER_MS = 12_000;
/** Whatever's still on screen by now, it goes. */
const GONE_AFTER_MS = 45_000;
/** Where the slow clap's meter goes: above the call window, or below it at the top of the screen. */
const METER_GAP = 6;
const METER_HEIGHT = 32;

const callWindow = document.getElementById("callwin")!;
const meterHost = document.getElementById("meter-host")!;
const fx = document.getElementById("fx")!;

let lastSeq = 0;
let lastPlayAt = 0;
let reportedIdle = true;

/** Put the stand-in where the call window is, or bottom centre when there's none showing. */
function placeCallWindow(frame: ReactionPlay["call_window"]) {
  const [x, y, width, height] = frame ?? [innerWidth / 2 - 160, innerHeight - 280, 320, 240];
  Object.assign(callWindow.style, { left: `${x}px`, top: `${y}px`, width: `${width}px`, height: `${height}px` });
  const above = y - METER_HEIGHT - METER_GAP;
  Object.assign(meterHost.style, {
    left: `${x}px`,
    width: `${width}px`,
    top: `${above >= 0 ? above : y + height + METER_GAP}px`,
  });
}

/** Who's in the call this time: the cached people stay, so their looks don't change. */
function meetPeople(play: ReactionPlay["play"]) {
  const present: Record<string, Person> = {};
  for (const [identity, name] of play.people) present[identity] = H.person(identity, name);
  const me = (present[play.me] = H.person(play.me, play.mine ? play.name : ""));
  const who = (present[play.from] = H.person(play.from, play.name, play.colour));
  H.people = present;
  H.me = me;
  return who;
}

function run({ seq, play, call_window }: ReactionPlay) {
  lastSeq = Math.max(lastSeq, seq);
  lastPlayAt = performance.now();
  reportedIdle = false;
  placeCallWindow(call_window);
  const who = meetPeople(play);
  H.setMuted(!soundsOn());
  H.wake();

  const { kind, choice, level = 0, text, to } = play.reaction;
  const effect = (n: number) => {
    const found = H.protos[n];
    if (!found) throw new Error(`no effect ${n}`);
    return found;
  };
  switch (kind) {
    case "confetti":
      return effect(1).play(H, who, level);
    case "gong":
      return effect(2).play(H, who);
    case "table_flip":
      return effect(3).play(H, who);
    case "emoji":
      return effect(4).play(H, who, choice);
    case "sound": {
      const pad = SOUNDS.findIndex((sound) => sound.key === choice);
      return pad >= 0 && effect(5).play(H, who, pad);
    }
    case "clap":
      return effect(6).clap(H, who);
    case "fireworks":
      return effect(7).play(H, who, Math.max(1, level));
    case "achievement":
      return effect(8).play(H, who, { text: text || undefined, to: to && H.people[to] ? to : undefined });
    case "disco":
      return effect(9).play(H, who);
    case "disco_end":
      return effect(9).stop(H, who);
    case "ship_it":
      return effect(10).play(H, who, text || "");
  }
}

function play(reaction: ReactionPlay) {
  try {
    run(reaction);
  } catch (e) {
    console.error("reactions: failed to play", reaction.play.reaction.kind, e);
  }
}

/** Whether anything is still showing. */
function busy() {
  return H.particleCount() > 0 || fx.childElementCount > 0 || document.querySelector(".h-banner") !== null;
}

setInterval(() => {
  if (reportedIdle) return;
  const quiet = performance.now() - lastPlayAt;
  if (quiet < IDLE_AFTER_MS || (busy() && quiet < GONE_AFTER_MS)) return;
  reportedIdle = true;
  typedInvoke("reactions_idle", { seq: lastSeq }).catch((e) => console.error("reactions: idle", e));
}, 2000);

listen<ReactionPlay>("reaction_play", (event) => play(event.payload))
  .then(() => typedInvoke("reactions_ready"))
  .then((waiting) => waiting.forEach(play))
  .catch((e) => console.error("reactions: failed to start", e));

// The call is over: everything stops at once.
listen("reactions_stop", () => {
  H.clearParticles();
  fx.replaceChildren();
  document.querySelectorAll(".h-banner").forEach((banner) => banner.remove());
  H.ac()
    .suspend()
    .catch(() => {});
  reportedIdle = true;
}).catch((e) => console.error("reactions: failed to listen for the call ending", e));
