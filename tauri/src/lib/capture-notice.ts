import { listen } from "@tauri-apps/api/event";
import { CoreParticipantState } from "@/core_payloads";
import useStore from "@/store/store";
import { closeCallNotice, isCallNoticeShowing, showCallNotice } from "./call-alert";

const NOTICE_KEY = "call-being-captured";
const RETRY_MS = 5_000;

const capturersIn = (participants: CoreParticipantState[]) => {
  const { teammates, user } = useStore.getState();
  const names = participants
    .filter((participant) => participant.connected && participant.capturing)
    .map((participant) => ({ id: participant.identity.split(":")[2], name: participant.name }))
    .filter(({ id }) => id !== undefined && id !== user?.id)
    .map(({ id, name }) => teammates?.find((teammate) => teammate.id === id)?.first_name ?? (name || "Someone"));
  return [...new Set(names)];
};

const listOf = (names: string[]) =>
  names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;

/**
 * Says so when someone else transcribes your call. The call window's notice and the camera
 * window's badges only help if those are open, and on an audio call neither need be.
 */
export const setupCaptureNotice = () => {
  // Whom the card has named. Once dismissed, it stays away until someone new starts.
  let told = new Set<string>();
  let retry: ReturnType<typeof setTimeout> | undefined;

  const tell = (participants: CoreParticipantState[]) => {
    clearTimeout(retry);
    const names = capturersIn(participants);
    if (names.length === 0) {
      told = new Set();
      closeCallNotice(NOTICE_KEY);
      return;
    }
    const someoneNew = names.some((name) => !told.has(name));
    const cardIsOutOfDate = isCallNoticeShowing(NOTICE_KEY) && names.length !== told.size;
    if (someoneNew || cardIsOutOfDate) {
      const one = names.length === 1;
      const shown = showCallNotice({
        key: NOTICE_KEY,
        title: `${listOf(names)} ${one ? "is" : "are"} transcribing this call`,
        body: `What's said and shown is being saved on ${one ? "their computer" : "their computers"}.`,
      });
      if (!shown) {
        retry = setTimeout(() => tell(participants), RETRY_MS);
        return;
      }
    }
    told = new Set(names);
  };

  const unlisten = listen<CoreParticipantState[]>("core_participants_snapshot", (event) => {
    if (useStore.getState().callTokens) tell(event.payload);
  });
  const unsubscribe = useStore.subscribe((state, previous) => {
    if (previous.callTokens && !state.callTokens) tell([]);
  });

  return () => {
    clearTimeout(retry);
    unsubscribe();
    unlisten.then((stopListening) => stopListening());
  };
};
