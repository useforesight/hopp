import { isToday, isYesterday } from "date-fns";

export type MissedCall = {
  callerId: string;
  kind: "call" | "invite";
  /** When they last tried, in milliseconds. */
  at: number;
  count: number;
};

const MISSED_CALLS_STORAGE = "hopp-missed-calls";

const isMissedCall = (value: any): value is MissedCall =>
  typeof value?.callerId === "string" &&
  (value.kind === "call" || value.kind === "invite") &&
  typeof value.at === "number" &&
  typeof value.count === "number";

export const loadMissedCalls = (): MissedCall[] => {
  try {
    const saved = JSON.parse(localStorage.getItem(MISSED_CALLS_STORAGE) ?? "[]");
    return Array.isArray(saved) ? saved.filter(isMissedCall) : [];
  } catch {
    return [];
  }
};

export const saveMissedCalls = (missedCalls: MissedCall[]) => {
  try {
    localStorage.setItem(MISSED_CALLS_STORAGE, JSON.stringify(missedCalls));
  } catch (error) {
    console.error("Failed to save missed calls:", error);
  }
};

/** Newest first, with one entry per person. */
export const addMissedCall = (missedCalls: MissedCall[], callerId: string, kind: MissedCall["kind"], at: number) => {
  const previous = missedCalls.find((call) => call.callerId === callerId);
  return [
    { callerId, kind, at: Math.max(at, previous?.at ?? 0), count: (previous?.count ?? 0) + 1 },
    ...missedCalls.filter((call) => call.callerId !== callerId),
  ];
};

const describeTime = (at: number) => {
  const date = new Date(at);
  const time = date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  if (isToday(date)) return time;
  if (isYesterday(date)) return `Yesterday ${time}`;
  return date.toLocaleDateString([], { day: "numeric", month: "short" });
};

export const describeMissedCall = ({ kind, at, count }: MissedCall) => {
  const what =
    kind === "invite" ? "Missed invite"
    : count > 1 ? `Missed ${count} calls`
    : "Missed call";
  return `${what} · ${describeTime(at)}`;
};
