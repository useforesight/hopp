import { useEffect, useState } from "react";
import { typedInvoke } from "@/core_payloads";

/** Decoded frames kept at once (each up to ~20 MB full size, ~3.5 MB for cards). */
const HELD = 36;

type Held = { url: string; used: number };

const held = new Map<string, Held>();
const loading = new Map<string, Promise<string>>();
let tick = 0;

const keyOf = (callId: string, number: number, full: boolean) => `${callId}/${number}/${full ? "full" : "small"}`;

const remember = (key: string, url: string) => {
  held.set(key, { url, used: ++tick });
  while (held.size > HELD) {
    let oldest: string | undefined;
    let oldestUsed = Infinity;
    for (const [other, { used }] of held) {
      if (used < oldestUsed) {
        oldest = other;
        oldestUsed = used;
      }
    }
    if (oldest === undefined) break;
    URL.revokeObjectURL(held.get(oldest)!.url);
    held.delete(oldest);
  }
};

const load = (callId: string, number: number, full: boolean): Promise<string> => {
  const key = keyOf(callId, number, full);
  const running = loading.get(key);
  if (running) return running;
  const job = typedInvoke("captures_frame", { callId, number, full })
    .then((bytes) => {
      // The type doesn't matter here: the browser reads it from the bytes.
      const url = URL.createObjectURL(new Blob([bytes]));
      remember(key, url);
      return url;
    })
    .finally(() => loading.delete(key));
  loading.set(key, job);
  return job;
};

/** A frame's picture as a URL, once it's loaded. */
export const useFrame = (callId: string, number: number, full = false): { url?: string; failed: boolean } => {
  const key = keyOf(callId, number, full);
  const [state, setState] = useState<{ key: string; url?: string; failed: boolean }>(() => ({
    key,
    url: held.get(key)?.url,
    failed: false,
  }));

  useEffect(() => {
    const known = held.get(key);
    if (known) {
      known.used = ++tick;
      setState({ key, url: known.url, failed: false });
      return;
    }
    let current = true;
    load(callId, number, full).then(
      (url) => current && setState({ key, url, failed: false }),
      () => current && setState({ key, failed: true }),
    );
    return () => {
      current = false;
    };
  }, [callId, number, full, key]);

  return state.key === key ? state : { failed: false };
};
