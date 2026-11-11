import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import useStore from "@/store/store";

/** The last seconds, each of which beeps. The same on Hopp for Linux. */
export const OFF_RECORD_COUNTDOWN_S = 5;

// A timer wakes a little late: the tick is for the second that just began.
const LATE_MS = 20;

/** Seconds left with `ends` and `now` on the same clock. A second that has started counts. */
export const secondsLeft = (ends: number, now: number) => Math.max(0, Math.ceil((ends - now) / 1000));

/** Seconds as m:ss. */
export const offRecordClock = (seconds: number) =>
  `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;

// When the call goes back on the record, on performance.now()'s clock. The core says
// how long is left once and the countdown runs here, so a computer's wrong time or a
// change of it can't move it.
let ends: number | null = null;
let seconds: number | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;
const listeners = new Set<() => void>();

const show = (next: number | null) => {
  if (next === seconds) return;
  seconds = next;
  listeners.forEach((listener) => listener());
};

let audio: AudioContext | undefined;

const audioContext = () => {
  audio ??= new AudioContext();
  // WKWebView may hold it back until the window has been clicked or typed in.
  if (audio.state === "suspended") audio.resume().catch(() => {});
  return audio;
};

const tone = (hz: number, ms: number) => {
  try {
    const context = audioContext();
    const oscillator = context.createOscillator();
    const gain = context.createGain();
    oscillator.frequency.value = hz;
    const start = context.currentTime;
    const end = start + ms / 1000;
    // A short fade either side, or it clicks.
    gain.gain.setValueAtTime(0, start);
    gain.gain.linearRampToValueAtTime(0.15, start + 0.01);
    gain.gain.setValueAtTime(0.15, Math.max(start + 0.01, end - 0.03));
    gain.gain.linearRampToValueAtTime(0, end);
    oscillator.connect(gain).connect(context.destination);
    oscillator.start(start);
    oscillator.stop(end + 0.02);
  } catch (error) {
    console.error("Off the record: couldn't play a tone:", error);
  }
};

const beep = () => tone(880, 90);
const resumed = () => tone(1320, 380);

// Redraws the clock on each second and beeps through the last five. The core says when
// the pause is over.
const tick = () => {
  clearTimeout(timer);
  if (ends === null) return;
  const left = ends - performance.now();
  if (left <= 0) return;
  const untilNextSecond = left % 1000 || 1000;
  timer = setTimeout(() => {
    if (ends === null) return;
    const now = secondsLeft(ends, performance.now() + LATE_MS);
    if (now >= 1 && now <= OFF_RECORD_COUNTDOWN_S) beep();
    show(now);
    tick();
  }, untilNextSecond);
};

/** Off the record with this long left, replacing what was left before. Null: back on it. */
const setLeft = (leftMs: number | null, quietly = false) => {
  const was = ends !== null;
  clearTimeout(timer);
  if (leftMs === null) {
    ends = null;
    show(null);
    if (was && !quietly) resumed();
    return;
  }
  const now = performance.now();
  ends = now + leftMs;
  show(secondsLeft(ends, now));
  tick();
};

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

/** Seconds left off the record, or null on the record. Changes once a second. */
export const useOffRecordSeconds = () => useSyncExternalStore(subscribe, () => seconds);

/**
 * Follows the call going off the record and back, from anyone in it: the clock for the
 * pill in the call window, a beep for each of the last five seconds and a higher tone
 * when the call is back on the record. Runs whether or not this app is capturing.
 */
export const setupOffRecord = () => {
  const unlisten = listen<number | null>("core_off_record", (event) => {
    // Leaving a call ends its pause without a sound.
    setLeft(event.payload, !useStore.getState().callTokens);
  });
  const unsubscribe = useStore.subscribe((state, previous) => {
    if (previous.callTokens && !state.callTokens) setLeft(null, true);
  });
  // So the first beep isn't held back for want of a click.
  const unlock = () => void audioContext();
  document.addEventListener("pointerdown", unlock, { once: true });
  document.addEventListener("keydown", unlock, { once: true });

  return () => {
    setLeft(null, true);
    unsubscribe();
    unlisten.then((stopListening) => stopListening());
    document.removeEventListener("pointerdown", unlock);
    document.removeEventListener("keydown", unlock);
  };
};
