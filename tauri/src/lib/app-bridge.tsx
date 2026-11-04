import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useRef } from "react";
import toast from "react-hot-toast";
import { OS } from "@/constants";
import { AppRequest, typedInvoke } from "@/core_payloads";
import { useAPI } from "@/services/query";
import useStore from "@/store/store";
import { tauriUtils } from "@/windows/window-utils";
import { appState, callForApp } from "./app-state";
import { pressCallAlert } from "./call-alert";
import { useJoinRoom } from "./hooks";
import { requestCallback, requestInvite } from "./incoming-call";

const ENABLED = isTauri() && OS === "macos";
const ROOMS_POLL_MS = 30_000;

/**
 * The window's end of the `hopp` command: says what's going on whenever it changes, and does what
 * only the window knows how to do. See `tauri/src-tauri/src/app_socket.rs`.
 */
export const AppBridge = () => {
  const authToken = useStore((state) => state.authToken);
  const user = useStore((state) => state.user);
  const teammates = useStore((state) => state.teammates);
  const callsPresence = useStore((state) => state.callsPresence);
  const calling = useStore((state) => state.calling);
  const inviting = useStore((state) => state.inviting);
  const incomingCaller = useStore((state) => state.incomingCallCallerId);
  const incomingInviter = useStore((state) => state.incomingInviteInviterId);
  // As text, so that this doesn't run for every change of the mic level.
  const callText = useStore((state) => JSON.stringify(callForApp(state.callTokens)));

  const { useQuery } = useAPI();
  const { data: rooms } = useQuery("get", "/api/auth/rooms", undefined, {
    enabled: ENABLED && !!authToken,
    refetchInterval: ROOMS_POLL_MS,
    retry: true,
    queryHash: `rooms-${authToken}`,
    staleTime: ROOMS_POLL_MS,
  });
  const { data: roomsPresence } = useQuery("get", "/api/auth/rooms/presence", undefined, {
    enabled: ENABLED && !!authToken,
    refetchInterval: ROOMS_POLL_MS,
    retry: true,
    queryHash: `rooms-presence-${authToken}`,
    select: (data) => data.rooms || {},
  });

  const state = useMemo(
    () =>
      appState({
        user: authToken ? user : null,
        teammates,
        callsPresence: callsPresence ?? {},
        rooms: rooms ?? [],
        roomsPresence: roomsPresence ?? {},
        call: JSON.parse(callText),
        calling,
        inviting,
        incomingCaller,
        incomingInviter,
      }),
    [
      authToken,
      user,
      teammates,
      callsPresence,
      rooms,
      roomsPresence,
      callText,
      calling,
      inviting,
      incomingCaller,
      incomingInviter,
    ],
  );

  const published = useRef<string | null>(null);
  useEffect(() => {
    if (!ENABLED) return;
    const text = JSON.stringify(state);
    if (text === published.current) return;
    published.current = text;
    typedInvoke("app_publish_state", { state }).catch((error) => console.error("Publishing state:", error));
  }, [state]);

  const { joinRoom } = useJoinRoom();
  const onRequest = useRef<(request: AppRequest) => void>(() => {});
  onRequest.current = (request) => {
    switch (request.type) {
      case "call":
        if (useStore.getState().callTokens) requestInvite(request.value);
        else requestCallback(request.value);
        break;
      case "join_room": {
        const room = rooms?.find((known) => known.id === request.value);
        if (!room) {
          toast.error("That room isn't there any more");
          break;
        }
        tauriUtils.showWindow("main");
        joinRoom(room);
        break;
      }
      case "accept":
        pressCallAlert("answer");
        break;
      case "decline":
        pressCallAlert("decline");
        break;
      case "open":
        tauriUtils.showWindow("main");
        break;
    }
  };

  useEffect(() => {
    if (!ENABLED) return;
    const unlisten = listen<AppRequest>("app_request", (event) => onRequest.current(event.payload));
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  return null;
};
