import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { typedInvoke } from "@/core_payloads";
import { components } from "@/openapi";
import { useAPI } from "@/services/query";
import useStore from "@/store/store";

export type UserStatus = NonNullable<components["schemas"]["BaseUser"]["status"]>;

export const USER_STATUSES: UserStatus[] = ["available", "away", "dnd"];

export const USER_STATUS_LABELS: Record<UserStatus, string> = {
  available: "Available",
  away: "Away",
  dnd: "Do not disturb",
};

export const USER_STATUS_DOTS: Record<UserStatus, string> = {
  available: "bg-success",
  away: "bg-warning",
  dnd: "bg-destructive",
};

export const useSetUserStatus = () => {
  const { useMutation } = useAPI();
  const { mutateAsync } = useMutation("put", "/api/auth/status", { retry: 3 });

  return useCallback(
    async (status: UserStatus, auto = false) => {
      const current = await mutateAsync({ body: { status, auto } });
      const { user, setUser } = useStore.getState();
      if (user) setUser({ ...user, ...current });
    },
    [mutateAsync],
  );
};

/**
 * Away while the screen's locked or nothing's being touched, and Available again on coming back. Being on a call
 * counts as being here, however still you sit. A status you chose yourself stays until you change it.
 */
export const useAutoAwayStatus = () => {
  const setUserStatus = useSetUserStatus();
  // Servers older than statuses leave this out, and then none of this does anything
  const status = useStore((state) => state.user?.status);
  const isAutomatic = useStore((state) => state.user?.status_auto);
  const inCall = useStore((state) => !!state.callTokens);
  const socketConnected = useStore((state) => state.socketConnected);
  const [steppedAway, setSteppedAway] = useState(false);

  useEffect(() => {
    typedInvoke("get_stepped_away").then(setSteppedAway);
    const unlisten = listen<boolean>("stepped_away_changed", (event) => setSteppedAway(event.payload));
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  useEffect(() => {
    // Teammates see you offline anyway, and coming back online tries again
    if (!socketConnected) return;

    const shouldBeAway = steppedAway && !inCall;
    const change =
      shouldBeAway && status === "available" ? "away"
      : !shouldBeAway && status === "away" && isAutomatic ? "available"
      : null;
    if (change) {
      setUserStatus(change, true).catch((error) => console.error("Failed to update status:", error));
    }
  }, [steppedAway, inCall, status, isAutomatic, socketConnected, setUserStatus]);
};
