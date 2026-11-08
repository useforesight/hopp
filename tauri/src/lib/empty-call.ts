import useStore from "@/store/store";
import { abandonCall } from "./call-recovery";

/** How long a call with nobody else in it stays open, with the mic and camera on. */
export const EMPTY_CALL_LIMIT_MS = 5 * 60_000;

type CallTokens = NonNullable<ReturnType<typeof useStore.getState>["callTokens"]>;

/** A call with someone, alone in it. A room is somewhere to wait for people, so it's never empty. */
const isEmptyCall = (callTokens: CallTokens) =>
  !callTokens.room &&
  !callTokens.isRoomCall &&
  !callTokens.isInitialisingCall &&
  !callTokens.isReconnecting &&
  callTokens.participants.every((participant) => participant.identity.includes("local"));

/**
 * Leaves a call nobody else is in any more. One left open by mistake keeps the mic live, and
 * keeps showing you as in a call, so others who join you end up in it alone.
 */
export const setupLeavingEmptyCalls = () => {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let timedCall: string | null = null;

  const stopTiming = () => {
    if (timer) clearTimeout(timer);
    timer = null;
    timedCall = null;
  };

  const check = () => {
    const { callTokens } = useStore.getState();
    if (!callTokens || !isEmptyCall(callTokens)) {
      stopTiming();
      return;
    }
    // Still the same call, already being timed.
    if (timer && timedCall === callTokens.audioToken) return;
    stopTiming();
    timedCall = callTokens.audioToken;
    timer = setTimeout(() => {
      timer = null;
      timedCall = null;
      const { callTokens } = useStore.getState();
      if (!callTokens || !isEmptyCall(callTokens)) return;
      abandonCall(
        callTokens.participant,
        "Hopp left an empty call",
        "Nobody else was in it for 5 minutes, so your mic and camera are off.",
      );
    }, EMPTY_CALL_LIMIT_MS);
  };

  const unsubscribe = useStore.subscribe(check);
  check();
  return () => {
    unsubscribe();
    stopTiming();
  };
};
