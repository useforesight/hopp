import toast from "react-hot-toast";
import useStore, { ParticipantRole } from "@/store/store";
import { tauriUtils } from "@/windows/window-utils";
import { Constants } from "@/constants";
import { validateAndSetAuthToken } from "./authUtils";
import { requestCallback } from "./incoming-call";
import { isLauncherKey } from "./launcher";
import type { components } from "@/openapi";

/**
 * Processes a hopp:// deep-link URL and handles the appropriate action.
 *
 * Supported deep-link formats:
 * - hopp:///authenticate?token=xxx - Authenticate user with token
 * - hopp:///join-session?sessionId=xxx - Join a Slack pairing session
 * - hopp:///call?userId=xxx&key=xxx - Call a teammate, from a launcher such as Raycast
 *
 * @param url The hopp:// URL to process
 * @returns true if the URL was handled successfully, false otherwise
 */
export const processDeepLinkUrl = async (url: string): Promise<boolean> => {
  try {
    const urlObj = new URL(url);

    if (urlObj.protocol !== "hopp:") {
      console.warn("Not a hopp:// URL:", url);
      return false;
    }

    const pathname = urlObj.pathname;
    const params = new URLSearchParams(urlObj.search);

    // Handle /authenticate deep-link
    if (pathname === "/authenticate") {
      const token = params.get("token");
      if (token) {
        await validateAndSetAuthToken(token);
        await tauriUtils.showWindow("main");
        return true;
      }
      console.warn("Missing token in authenticate deep-link");
      return false;
    }

    // Handle /join-session deep-link
    if (pathname === "/join-session") {
      const sessionId = params.get("sessionId");
      if (sessionId) {
        return await handleJoinSessionDeepLink(sessionId);
      }
      console.warn("Missing sessionId in join-session deep-link");
      return false;
    }

    // Handle /call deep-link
    if (pathname === "/call") {
      const userId = params.get("userId");
      if (userId && isLauncherKey(params.get("key"))) {
        return await handleCallDeepLink(userId);
      }
      console.warn("Missing user or wrong key in call deep-link");
      return false;
    }

    console.warn("Unknown deep-link path:", pathname);
    return false;
  } catch (err) {
    console.error("Failed to parse deep link URL:", err);
    toast.error("Failed to process deep link");
    return false;
  }
};

const handleCallDeepLink = async (userId: string): Promise<boolean> => {
  const { authToken, callTokens, calling, incomingCallCallerId, setTab } = useStore.getState();

  if (!authToken) {
    toast.error("Please log in first to call someone");
    setTab("login");
    await tauriUtils.showWindow("main");
    return false;
  }

  const blockedReason =
    callTokens ? "Leave your current call first"
    : incomingCallCallerId ? "Answer or decline the incoming call first"
    : calling ? "You're already calling someone"
    : null;
  if (blockedReason) {
    toast.error(blockedReason);
    await tauriUtils.showWindow("main");
    return false;
  }

  requestCallback(userId);
  return true;
};

let isJoiningSession = false;

/**
 * Handles joining a Slack pairing session by fetching tokens and starting the call.
 *
 * @param sessionId The session/room ID to join
 * @returns true if successfully joined, false otherwise
 */
export const handleJoinSessionDeepLink = async (sessionId: string): Promise<boolean> => {
  if (isJoiningSession) return false;

  isJoiningSession = true;
  try {
    return await joinSession(sessionId);
  } finally {
    isJoiningSession = false;
  }
};

const joinSession = async (sessionId: string): Promise<boolean> => {
  const { authToken, callTokens, setCallTokens, setTab, user } = useStore.getState();

  if (!authToken) {
    toast.error("Please log in first to join the session");
    setTab("login");
    await tauriUtils.showWindow("main");
    return false;
  }

  if (callTokens) {
    await tauriUtils.showWindow("main");
    if (callTokens.room?.id === sessionId) {
      setTab("call");
      return true;
    }
    toast.error("Leave your current call first");
    return false;
  }

  try {
    toast.loading("Joining session", { id: "join-session" });

    // Fetch tokens for the session
    const response = await fetch(
      `${Constants.backendUrl}/api/auth/slack/session/${encodeURIComponent(sessionId)}/tokens`,
      {
        headers: {
          Authorization: `Bearer ${authToken}`,
        },
      },
    );

    if (!response.ok) {
      toast.dismiss("join-session");
      if (response.status === 404) {
        toast.error("Session has ended or doesn't exist");
      } else if (response.status === 401) {
        toast.error("Please log in again to join the session");
        setTab("login");
      } else if (response.status === 402) {
        const body = await response.json().catch(() => null);
        if (body?.error === "trial-ended") {
          toast.error("Trial has expired, contact us if you want to extend it");
        } else {
          toast.error("Payment required to access this session");
        }
      } else if (response.status === 403) {
        toast.error("You don't have access to this session. It belongs to a different team.");
      } else {
        toast.error("Failed to join session");
      }
      console.error("Failed to fetch session tokens:", response.status);
      return false;
    }

    const data: components["schemas"]["SessionTokensResponse"] = await response.json();
    const { startMic, startCamera } = await tauriUtils.getCallStartPreferences();

    // A call may have started during those requests.
    if (useStore.getState().callTokens) {
      toast.dismiss("join-session");
      toast.error("Leave your current call first");
      await tauriUtils.showWindow("main");
      return false;
    }

    // Set up the call with the tokens
    setCallTokens({
      ...data,
      timeStarted: new Date(),
      hasAudioEnabled: startMic,
      hasCameraEnabled: startCamera,
      role: ParticipantRole.NONE,
      isRemoteControlEnabled: true,
      isRoomCall: true,
      participants: [],
      room: {
        id: sessionId,
        // TODO(@konsalex): Get the room name from the backend
        // Same issue with `SlackJoin.tsx` component
        name: "Slack Session",
        user_id: user?.id || "",
      },
      isInitialisingCall: true,
      micLevel: 0,
    });

    await tauriUtils.showWindow("main");

    try {
      await tauriUtils.callStarted(data.audioToken, data.videoToken);
    } catch (err) {
      console.error("Failed to start session call:", err);
      tauriUtils.endCallCleanup();
      setCallTokens(null);
      toast.dismiss("join-session");
      toast.error("Failed to start the call");
      return false;
    }

    toast.dismiss("join-session");
    return true;
  } catch (err) {
    console.error("Error joining session:", err);
    toast.dismiss("join-session");
    toast.error("Failed to join room");
    return false;
  }
};
