import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { WakeHeard, WakeWord } from "@/core_payloads";
import useStore from "@/store/store";

/** Whose ask of an agent is showing, and their words so far. */
export interface Heard {
  /** Whose tile, as `heardTile` names it. */
  tile: string;
  words: WakeWord[];
  /** How many times their words have changed, so the light can start its hold again. */
  updates: number;
}

// The core takes an ask back a few seconds after its last word (`wake_heard::HOLD`, as
// on Hopp for Linux). This is only for a core that went away before it could.
const GONE_MS = 6_000;

let heard: Heard | null = null;
let gone: ReturnType<typeof setTimeout> | undefined;
const listeners = new Set<() => void>();

const show = (next: Heard | null) => {
  clearTimeout(gone);
  if (next === heard) return;
  heard = next;
  if (next) gone = setTimeout(() => show(null), GONE_MS);
  listeners.forEach((listener) => listener());
};

/**
 * The tile a participant identity belongs to: "local" for yours, otherwise the person's
 * user id. Takes the core's "local", a whole identity (`room:<room>:<user>:audio`) or
 * one without its connection (`room:<room>:<user>`), which is how the capture knows people.
 */
export const heardTile = (identity: string, ownId: string | undefined) => {
  if (identity === "local") return "local";
  const parts = identity.split(":");
  const person = parts[0] === "room" && parts.length >= 3 ? (parts[2] ?? identity) : identity;
  // You, heard from another computer you're also in the call on.
  return person === ownId ? "local" : person;
};

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

/** Who said the wake word and what they're asking, while it shows. */
export const useHeard = () => useSyncExternalStore(subscribe, () => heard);

/**
 * Follows the capture hearing someone say the wake word: a light round their tile in the
 * call window and their words beside it, well before any agent answers. Only this app's
 * own capture says so, so nothing shows unless you're capturing the call.
 */
export const setupHeard = () => {
  const unlisten = listen<WakeHeard | null>("core_wake_heard", (event) => {
    const { callTokens, user } = useStore.getState();
    if (!event.payload || !callTokens) {
      show(null);
      return;
    }
    const tile = heardTile(event.payload.identity, user?.id);
    // More from the same person keeps the light where it is on its way round.
    const updates = heard?.tile === tile ? heard.updates + 1 : 0;
    show({ tile, words: event.payload.words, updates });
  });
  const unsubscribe = useStore.subscribe((state, previous) => {
    if (previous.callTokens && !state.callTokens) show(null);
  });

  return () => {
    show(null);
    unsubscribe();
    unlisten.then((stopListening) => stopListening());
  };
};
