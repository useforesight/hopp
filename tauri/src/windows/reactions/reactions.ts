// What can be sent, shared by the tray and the layer that plays them. The keys are the
// choices on the wire (see `socket_lib::Reaction`), and the sounds are in the order of the
// soundboard's pads.

export const EMOJI = [
  { key: "party", emoji: "🎉", label: "Party" },
  { key: "fire", emoji: "🔥", label: "Fire" },
  { key: "applause", emoji: "👏", label: "Applause" },
  { key: "lol", emoji: "😂", label: "LOL" },
  { key: "rocket", emoji: "🚀", label: "Ship it" },
  { key: "hundred", emoji: "💯", label: "100" },
  { key: "love", emoji: "❤️", label: "Love" },
  { key: "shock", emoji: "😱", label: "Shock" },
] as const;

export const SOUNDS = [
  { key: "airhorn", emoji: "📯", label: "Airhorn" },
  { key: "badumtss", emoji: "🥁", label: "Ba-dum-tss" },
  { key: "sad_trombone", emoji: "🎺", label: "Sad trombone" },
  { key: "crickets", emoji: "🦗", label: "Crickets" },
  { key: "drumroll", emoji: "🥁", label: "Drumroll" },
  { key: "chaching", emoji: "💰", label: "Cha-ching" },
  { key: "wooow", emoji: "😮", label: "Wooow" },
  { key: "applause", emoji: "👏", label: "Applause" },
] as const;

/** Reactions play silently on this Mac when their sounds are off. On by default. */
const SOUNDS_KEY = "hopp-reactions-sound";

export function soundsOn() {
  return localStorage.getItem(SOUNDS_KEY) !== "off";
}

export function setSoundsOn(on: boolean) {
  localStorage.setItem(SOUNDS_KEY, on ? "on" : "off");
}

/** Confetti taps closer together than this build a streak, up to the biggest burst. */
export const CONFETTI_STREAK_MS = 900;
export const CONFETTI_MOST = 4;

/** Fireworks: a click sends a few rockets; holding charges up to ten, as the prototype did. */
const FIREWORKS_CLICK = 3;
const FIREWORKS_HOLD_MS = 250;
export const FIREWORKS_FULL_MS = 3000;

export function rockets(heldMs: number) {
  if (heldMs < FIREWORKS_HOLD_MS) return FIREWORKS_CLICK;
  const charge = Math.min(1, (heldMs - FIREWORKS_HOLD_MS) / (FIREWORKS_FULL_MS - FIREWORKS_HOLD_MS));
  return 1 + Math.round(charge * 9);
}
