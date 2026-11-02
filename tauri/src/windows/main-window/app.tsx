import { useEffect, useRef } from "react";
import { onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { publishLauncherDirectory } from "@/lib/launcher";
import useStore from "../../store/store";
import { tauriUtils } from "@/windows/window-utils.ts";
import { Sidebar } from "@/components/sidebar/Sidebar";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Debug } from "./tabs/Debug";
import { Login } from "./login";
import { Report } from "./report";
import { useAPI } from "@/services/query";
import { isTauri } from "@tauri-apps/api/core";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { HiOutlineExclamationCircle } from "react-icons/hi2";
import toast from "react-hot-toast";
import { socketService } from "@/services/socket";
import { TWebSocketMessage, TPresenceAckMessage } from "@/payloads";
import { Participants } from "@/components/ui/participants";
import { CallCenter } from "@/components/ui/call-center";
import { listen } from "@tauri-apps/api/event";
import Invite from "./invite";
import { useDisableNativeContextMenu, useJoinCall } from "@/lib/hooks";
import { processDeepLinkUrl } from "@/lib/deepLinkUtils";
import { Rooms } from "./tabs/Rooms";
import { pollUpdates } from "@/lib/auto-update";
import { setupCallAlertBridge } from "@/lib/call-alert";
import { setupCallRecoveryListeners } from "@/lib/call-recovery";
import { setJoinCallHandler, setupIncomingCallHandling } from "@/lib/incoming-call";

function App() {
  const {
    tab,
    authToken,
    callTokens,
    teammates,
    updateInProgress,
    setCallTokens,
    setNeedsUpdate,
    setUser,
    setTab,
    setTeammates,
    setAuthToken,
    setLivekitUrl,
    setIncomingCallCallerId,
    setIncomingInviteInviterId,
    setCallsPresence,
  } = useStore();

  useDisableNativeContextMenu();

  const { useQuery } = useAPI();

  const sentryMetadataRef = useRef<boolean>(false);

  const { error: userError } = useQuery("get", "/api/auth/user", undefined, {
    enabled: !!authToken,
    refetchInterval: 30_000,
    retry: true,
    queryHash: `user-${authToken}`,
    select: (data) => {
      setUser(data);
      if (!sentryMetadataRef.current) {
        tauriUtils.setSentryMetadata(data.id);
        sentryMetadataRef.current = true;
      }
      return data;
    },
  });

  // Get current user's teammates
  const { error: teammatesError, refetch: refetchTeammates } = useQuery("get", "/api/auth/teammates", undefined, {
    enabled: !!authToken,
    refetchInterval: 10_000,
    refetchIntervalInBackground: true,
    retry: true,
    queryHash: `teammates-${authToken}`,
    select: (data) => {
      setTeammates(data);
      return data;
    },
  });

  // Poll call presence every 10 seconds
  const { refetch: refetchCallsPresence } = useQuery("get", "/api/auth/calls/presence", undefined, {
    enabled: !!authToken,
    refetchInterval: 10_000,
    refetchIntervalInBackground: true,
    retry: true,
    queryHash: `calls-presence-${authToken}`,
    select: (data) => {
      setCallsPresence(data.presence);
      return data.presence;
    },
  });

  // Get LiveKit server URL and send to Tauri backend
  const { data: livekitUrlData } = useQuery("get", "/api/auth/livekit/server-url", undefined, {
    enabled: !!authToken,
    retry: true,
    queryHash: `livekit-url-${authToken}`,
  });

  // Send LiveKit URL to Tauri backend and store when fetched
  useEffect(() => {
    const sendLivekitUrlToBackend = async () => {
      if (livekitUrlData?.url) {
        console.log("livekitUrlData", livekitUrlData);
        try {
          await tauriUtils.setLivekitUrl(livekitUrlData.url);
          setLivekitUrl(livekitUrlData.url);
          console.debug("LiveKit URL sent to Tauri backend:", livekitUrlData.url);
        } catch (err) {
          console.error("Failed to send LiveKit URL to Tauri backend:", err);
        }
      }
    };

    sendLivekitUrlToBackend();
  }, [livekitUrlData, setLivekitUrl]);

  // Load stored token and custom server URL on app start
  useEffect(() => {
    (async () => {
      if (!isTauri()) return;
      // Load custom server URL first (before auth token, since auth uses the URL)
      const customUrl = await tauriUtils.loadCustomServerUrl();
      if (customUrl) {
        useStore.getState().setCustomServerUrl(customUrl);
      }
      // Then load auth token
      const token = await tauriUtils.getStoredToken();
      if (token) {
        setAuthToken(token);
      }
    })();
  }, []);

  // Deep link handling
  useEffect(() => {
    if (!isTauri()) return;
    // Focus to open the window is its closed
    const setupDeepLinkListener = async () => {
      const unlistenFn = await onOpenUrl(async (urls: string[]) => {
        console.log("Received deep link request:", urls);
        const url = urls[0];
        if (url) {
          // Use the centralized deep-link handler
          await processDeepLinkUrl(url);
        }
      });

      return unlistenFn;
    };

    let unlisten: (() => void) | undefined;
    setupDeepLinkListener().then((fn) => {
      unlisten = fn;
    });

    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  // Lets launchers such as Raycast see who can be called
  useEffect(() => {
    if (!isTauri()) return;
    if (!authToken) publishLauncherDirectory([]);
    else if (teammates) publishLauncherDirectory(teammates);
  }, [authToken, teammates]);

  // Check for updates (and auto-install when idle on macOS)
  useEffect(() => {
    if (!isTauri()) return;

    pollUpdates(setNeedsUpdate);
    const interval = setInterval(() => pollUpdates(setNeedsUpdate), 15 * 60 * 1000);

    return () => clearInterval(interval);
  }, [setNeedsUpdate]);

  // Auto-navigate to the call page on call join, and away from it on call end.
  const prevInCallRef = useRef(false);
  useEffect(() => {
    const inCall = !!callTokens;
    if (inCall && !prevInCallRef.current) {
      setTab("call");
    } else if (!inCall && prevInCallRef.current && tab === "call") {
      setTab("user-list");
    }
    prevInCallRef.current = inCall;
  }, [callTokens, tab, setTab]);

  const joinOngoingCall = useJoinCall();
  useEffect(() => setJoinCallHandler(joinOngoingCall), [joinOngoingCall]);

  // Incoming calls and invites, the card that shows them, and recovery during calls
  useEffect(() => {
    if (!isTauri()) return;
    const stopCallAlertBridge = setupCallAlertBridge();
    const stopIncomingCallHandling = setupIncomingCallHandling();
    const stopCallRecoveryListeners = setupCallRecoveryListeners();
    return () => {
      stopCallAlertBridge();
      stopIncomingCallHandling();
      stopCallRecoveryListeners();
    };
  }, []);

  // Generic socket event listeners
  // Need to remove from here and add them to a shared file
  // to be cleaner and easier to manage
  useEffect(() => {
    socketService.on("teammate_online", (data: TWebSocketMessage) => {
      if (data.type === "teammate_online") {
        const { teammates } = useStore.getState();
        for (const teammate of teammates || []) {
          if (teammate.id === data.payload.teammate_id && !teammate.is_active) {
            refetchTeammates();
          }
        }
      }
    });

    socketService.on("presence_changed", (data: TWebSocketMessage) => {
      if (data.type === "presence_changed") {
        refetchCallsPresence();
      }
    });

    socketService.on("presence_check", (data: TWebSocketMessage) => {
      if (data.type !== "presence_check") {
        return;
      }
      const room = data.payload.room;
      const { callTokens } = useStore.getState();
      // in_call answers the validation ping. For named room calls we additionally
      // verify the room matches; ad-hoc 1:1 rooms aren't stored client-side, so
      // the boolean (callTokens !== null) covers them.
      let inCall = callTokens !== null;
      if (inCall && callTokens?.room?.id) {
        inCall = callTokens.room.id === room;
      }
      socketService.send({
        type: "presence_ack",
        payload: { room, in_call: inCall },
      } as TPresenceAckMessage);
    });
  }, []);

  useEffect(() => {
    if (!isTauri()) return;
    // Update auth token when it changes in the backend
    const setupChangeTokenListener = async () => {
      const unlistenFn = await listen("token_changed", (event) => {
        const token = event.payload as string;
        if (token) {
          setAuthToken(token);
        } else {
          // Token was deleted (e.g., when changing server URL)
          // Clear auth-related state but preserve customServerUrl
          setAuthToken(null);
          setUser(null);
          setTeammates(null);
          setCallTokens(null);
        }
      });

      return unlistenFn;
    };

    // Update custom server URL when it changes in the backend
    const setupServerUrlChangedListener = async () => {
      const unlistenFn = await listen<string | null>("hopp_server_url_changed", (event) => {
        useStore.getState().setCustomServerUrl(event.payload);
      });

      return unlistenFn;
    };

    let unlistenChangeToken: (() => void) | undefined;
    setupChangeTokenListener().then((fn) => {
      unlistenChangeToken = fn;
    });

    let unlistenServerUrl: (() => void) | undefined;
    setupServerUrlChangedListener().then((fn) => {
      unlistenServerUrl = fn;
    });

    return () => {
      if (unlistenChangeToken) unlistenChangeToken();
      if (unlistenServerUrl) unlistenServerUrl();
    };
  }, []);

  /*
   * This is a hack for keeping the frontend alive and
   * continue to receive web socket messages, if we don't do that
   * the frontend goes to sleep and stops receiving web sockets messages
   * which means that an incoming call might be missed.
   * Worst case scenario the ring won't be heard for the first 30 seconds.
   */
  useEffect(() => {
    if (!isTauri()) return;
    const setupCoreProcessCrashedListener = async () => {
      const unlistenFn = await listen("ping", () => {});

      return unlistenFn;
    };

    let unlisten: (() => void) | undefined;
    setupCoreProcessCrashedListener().then((fn) => {
      unlisten = fn;
    });

    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  // Avoid showing login tab if user is already logged in
  if (authToken && tab === "login") {
    setTab("user-list");
  }

  return (
    <div className="container flex flex-row" id="app-body">
      {/* Action Sidebar */}
      <Sidebar />
      <div className="relative h-full min-w-0 flex-1">
        <ScrollArea key={tab} type="scroll" className="h-full w-full [&>[data-radix-scroll-area-viewport]>div]:!block">
          {(userError || teammatesError) && (
            <div className="flex flex-col gap-2 px-3 pt-3">
              {userError && (
                <Alert variant="destructive" className="py-2">
                  <HiOutlineExclamationCircle className="h-4 w-4" />
                  <AlertTitle>Issue</AlertTitle>
                  <AlertDescription>{userError?.message}</AlertDescription>
                </Alert>
              )}
              {teammatesError && (
                <Alert variant="destructive" className="py-2">
                  <HiOutlineExclamationCircle className="h-4 w-4" />
                  <AlertTitle>Issue</AlertTitle>
                  <AlertDescription>{teammatesError?.message}</AlertDescription>
                </Alert>
              )}
            </div>
          )}
          {callTokens && (
            <div className={tab === "call" ? "" : "hidden"}>
              <CallCenter />
            </div>
          )}
          {tab === "debug" && <Debug />}
          {tab === "invite" && <Invite />}
          {tab === "login" && <Login />}
          {tab === "rooms" && <Rooms />}
          {tab === "user-list" && <Participants teammates={teammates || []} />}
          {tab === "report-issue" && <Report />}
        </ScrollArea>
        <div aria-hidden className="aurora pointer-events-none absolute inset-0 z-20" />
      </div>
    </div>
  );
}

export default App;
