import type { CaptureCall, CaptureMark } from "@/core_payloads";

const pad = (value: number) => String(value).padStart(2, "0");

/** `mm:ss` or `h:mm:ss` since the capture started. */
export const clock = (ms: number): string => {
  const seconds = Math.floor(Math.max(ms, 0) / 1000);
  const minutes = pad(Math.floor(seconds / 60) % 60);
  if (seconds >= 3600) return `${Math.floor(seconds / 3600)}:${minutes}:${pad(seconds % 60)}`;
  return `${pad(Math.floor(seconds / 60))}:${pad(seconds % 60)}`;
};

export const duration = (ms: number): string => {
  const minutes = Math.floor((Math.max(ms, 0) + 30_000) / 60_000);
  return minutes >= 60 ? `${Math.floor(minutes / 60)} h ${minutes % 60} min` : `${minutes} min`;
};

const time = (date: Date) => `${pad(date.getHours())}:${pad(date.getMinutes())}`;
const weekday = (date: Date) => date.toLocaleDateString(undefined, { weekday: "short" });
const dayMonth = (date: Date) => date.toLocaleDateString(undefined, { day: "numeric", month: "short" });

/** "Today 18:27", "Mon 16:15" or "3 Nov 16:15". */
export const when = (startedMs: number, nowMs = Date.now()): string => {
  const date = new Date(startedMs);
  if (date.toDateString() === new Date(nowMs).toDateString()) return `Today ${time(date)}`;
  if (nowMs - startedMs < 6 * 24 * 3_600_000) return `${weekday(date)} ${time(date)}`;
  return `${dayMonth(date)} ${time(date)}`;
};

/** "Tue 3 Nov, 16:15". */
export const longDate = (startedMs: number): string => {
  const date = new Date(startedMs);
  return `${weekday(date)} ${dayMonth(date)}, ${time(date)}`;
};

export const firstName = (name: string): string => name.trim().split(/\s+/)[0] || name;

export const initials = (name: string): string => {
  const letters = name
    .trim()
    .split(/\s+/)
    .map((word) => [...word][0] ?? "")
    .slice(0, 2)
    .join("");
  return letters ? letters.toUpperCase() : "?";
};

export const plural = (count: number, noun: string): string => `${count} ${noun}${count === 1 ? "" : "s"}`;

/** Call::heading in crates/hopp-capture/src/store.rs. */
export const heading = (call: CaptureCall): string => {
  if (call.title) return call.title;
  if (call.room) return `Call in ${call.room}`;
  const [first, second, ...rest] = call.participants;
  if (!first) return "Call";
  if (!second) return `Call with ${first}`;
  if (rest.length === 0) return `Call with ${first} and ${second}`;
  if (rest.length === 1) return `Call with ${first}, ${second} and ${rest[0]}`;
  return `Call with ${first}, ${second} and ${rest.length} others`;
};

/** Ticks every minute for short calls, then wider. */
export const tickStep = (durationMs: number): number => {
  if (durationMs < 60_000) return 15_000;
  const minutes = Math.floor(durationMs / 60_000);
  if (minutes <= 6) return 60_000;
  if (minutes <= 15) return 3 * 60_000;
  if (minutes <= 40) return 5 * 60_000;
  if (minutes <= 90) return 10 * 60_000;
  return 20 * 60_000;
};

/** A range of the call, or nothing for a moment. */
export const span = (from: number, to: number): string => (to - from >= 1_000 ? `${clock(from)}–${clock(to)}` : "");

export const everyoneWhoDrew = (marks: CaptureMark[]): string[] => [...new Set(marks.map((mark) => mark.by))];

/** "Alex", "Alex and Sam" or "Alex, Sam and Lee". */
export const joinNames = (names: string[]): string => {
  if (names.length === 0) return "Someone";
  if (names.length === 1) return names[0]!;
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
};

/** One label per person and kind of mark, with the marks it stands for. */
export const markGroups = (marks: CaptureMark[]): { by: string; label: string; indexes: number[] }[] => {
  const groups = new Map<string, { by: string; shape: string; indexes: number[] }>();
  marks.forEach((mark, index) => {
    const key = `${mark.by}\u0000${mark.shape}`;
    const group = groups.get(key) ?? { by: mark.by, shape: mark.shape, indexes: [] };
    group.indexes.push(index);
    groups.set(key, group);
  });
  return [...groups.values()].map(({ by, shape, indexes }) => {
    const who = firstName(by);
    const only = indexes.length === 1 ? marks[indexes[0]!] : undefined;
    let label: string;
    if (shape === "pin") {
      const pins = indexes.map((index) => marks[index]!.pin).filter((pin) => pin !== undefined);
      label = `${who} · ${pins.length === 1 ? "pin" : "pins"} ${pins.join(", ")}`;
      if (only?.text) label += ` “${only.text}”`;
    } else {
      label = `${who} · ${plural(indexes.length, shape === "stroke" ? "drawing" : shape)}`;
    }
    return { by, label, indexes };
  });
};

export const agentPrompt = (call: CaptureCall): string =>
  call.ended_ms === undefined ?
    `I'm in a Hopp call that is being transcribed on this computer (capture ${call.id}). ` +
    `Read what's been said with \`hopp capture show ${call.id}\`. To keep listening, run ` +
    `\`hopp capture follow ${call.id} --on-wake jeepers\`: it returns once someone has said ` +
    `"Jeepers" to ask you something and finished talking, with their words and what to read ` +
    `first. ` +
    `Answer, then run it again.`
  : `Read the transcript of a Hopp call with \`hopp capture show ${call.id}\`. Write a short ` +
    `title and a two- or three-line summary with any decisions and follow-ups, then ` +
    `save them with \`hopp capture annotate ${call.id} --title "…" --summary "…"\`.`;
