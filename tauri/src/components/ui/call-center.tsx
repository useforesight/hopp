import { showCallNotice } from "@/lib/call-alert";
import {
  LuAudioLines,
  LuMicOff,
  LuPhoneOff,
  LuVideo,
  LuVideoOff,
  LuScreenShare,
  LuScreenShareOff,
} from "react-icons/lu";
import { PiScribbleLoopBold, PiCursorBold, PiMapPinBold } from "react-icons/pi";
import useStore, { CallState, ParticipantRole } from "@/store/store";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  DropdownMenuCheckboxItem,
} from "@/components/ui/dropdown-menu";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Select, SelectContent, SelectItem, SelectTrigger } from "./select";
import { SelectPortal } from "@radix-ui/react-select";
import { Button } from "./button";
import { tauriUtils } from "@/windows/window-utils";
import { HiOutlineCursorClick, HiOutlineEye } from "react-icons/hi";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { CustomIcons } from "@/components/ui/icons";
import { avatarGradient, HoppAvatar } from "@/components/ui/hopp-avatar";
import clsx from "clsx";
import { ChevronDownIcon } from "@radix-ui/react-icons";
import { MoreHorizontal } from "lucide-react";
import { HiLockClosed, HiMiniLink } from "react-icons/hi2";
import { CgSpinner } from "react-icons/cg";
import toast from "react-hot-toast";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Constants } from "@/constants";
import { useEndCall } from "@/lib/hooks";
import { CoreParticipantState, typedInvoke } from "@/core_payloads";
import { useQuery } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { getAllWindows } from "@tauri-apps/api/window";
import { sounds } from "@/constants/sounds";

const NO_PARTICIPANTS: CoreParticipantState[] = [];

type DockTone = "neutral" | "danger" | "success" | "accent" | "end";

const dockToneClass: Record<DockTone, string> = {
  neutral: "bg-subtle text-foreground ring-border hover:bg-subtle-strong",
  danger: "bg-destructive-soft text-destructive ring-destructive/25 hover:bg-destructive/20",
  success: "bg-success-soft text-success ring-success/25 hover:bg-success/20",
  accent: "bg-accent-soft text-accent ring-accent/25 hover:bg-accent/25",
  end: "bg-destructive text-destructive-foreground ring-transparent shadow-card hover:bg-destructive/90",
};

const DockButton = React.forwardRef<
  HTMLButtonElement,
  { tone: DockTone; icon: React.ReactNode; label: string } & React.ComponentPropsWithoutRef<"button">
>(({ tone, icon, label, className, ...props }, ref) => (
  <button
    ref={ref}
    type="button"
    {...props}
    className={clsx(
      "flex h-[58px] w-full flex-col items-center justify-center gap-1 rounded-[14px] text-[11px] leading-4 font-medium ring-1 ring-inset transition-[background-color,color,transform] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring active:scale-[0.97] disabled:opacity-50",
      dockToneClass[tone],
      className,
    )}
  >
    {icon}
    {label}
  </button>
));
DockButton.displayName = "DockButton";

const deviceMenuTriggerClass =
  "flex size-5 items-center justify-center rounded-md border-0 bg-transparent p-0 text-current opacity-70 hover:bg-subtle-strong hover:opacity-100 focus:ring-0 focus-visible:ring-2 focus-visible:ring-ring";

const sharingToolClass =
  "relative flex size-8 items-center justify-center rounded-[10px] transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50";

const ShortcutTooltip = ({ shortcut }: { shortcut?: string }) =>
  shortcut ?
    <TooltipContent sideOffset={4} side="top" className="flex flex-row items-center px-1.5">
      <kbd>{shortcut}</kbd>
    </TooltipContent>
  : null;

export function CallCenter() {
  const { callTokens } = useStore();
  const [controllerCursorState, setControllerCursorState] = useState(true);
  const [accessibilityPermission, setAccessibilityPermission] = useState(true);

  const { data: userSettings } = useQuery({
    queryKey: ["user-settings"],
    queryFn: () => typedInvoke("get_user_settings"),
    refetchOnWindowFocus: true,
  });

  const handleEndCall = useEndCall();
  const handleEndCallRef = useRef(handleEndCall);
  handleEndCallRef.current = handleEndCall;

  const handleCopyRoomLink = async () => {
    if (!callTokens?.room) return;
    const roomLink = `${Constants.webAppUrl}/room/${callTokens.room.id}`;
    await writeText(roomLink);
    toast.success("Room link copied to clipboard");
  };

  useEffect(() => {
    const fetchAccessibilityPermission = async () => {
      const permission = await tauriUtils.getControlPermission();
      setAccessibilityPermission(permission);
      const enabled = permission && (userSettings?.remote_control_enabled ?? true);
      setControllerCursorState(enabled);

      if (callTokens?.role === ParticipantRole.SHARER && userSettings) {
        tauriUtils.setControllerCursor(enabled);
      }
    };

    fetchAccessibilityPermission();
  }, [callTokens?.role, userSettings]);

  useEffect(() => {
    const unlisten = listen("core_call_ended", () => {
      console.log("core_call_ended event received");
      const { callTokens } = useStore.getState();
      if (!callTokens) return;
      handleEndCallRef.current();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const unlisten = listen("core_room_connection_failed", () => {
      console.log("core_call_ended event received");
      const { callTokens } = useStore.getState();
      if (!callTokens) return;
      handleEndCallRef.current();
      toast.error("Failed to establish connection");
      showCallNotice({
        key: "call-failed",
        title: "Couldn't connect the call",
        body: "Hopp couldn't reach the call server. Check your connection and try again.",
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const unlisten = listen<string>("core_screenshare_failed", async () => {
      const windows = await getAllWindows();
      const main = windows.find((w) => w.label === "main");
      if (main) {
        const focused = await main.isFocused();
        if (!focused) {
          await main.show();
          await main.setFocus();
        }
      }
      toast.error("Failed to start screen sharing", { duration: 2500 });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const unlisten = listen<string>("core_app_veil_failed", async (event) => {
      const windows = await getAllWindows();
      const main = windows.find((w) => w.label === "main");
      if (main) {
        const focused = await main.isFocused();
        if (!focused) {
          await main.show();
          await main.setFocus();
        }
      }
      toast.error(event.payload || "App Veil is unavailable", { duration: 6000 });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  if (!callTokens) return null;

  const toggleControllerCursor = () => {
    let controllerCursorTmp = !controllerCursorState;
    tauriUtils.setControllerCursor(controllerCursorTmp);
    setControllerCursorState(controllerCursorTmp);
  };

  return (
    <div className="flex h-[var(--app-height)] w-full flex-col gap-2.5 px-3 pb-3 pt-3.5">
      <CallHeader onCopyRoomLink={handleCopyRoomLink} />
      <CaptureNotice />
      <CallParticipants />

      {callTokens.role === ParticipantRole.CONTROLLER && (
        <Button
          className="w-full gap-2 rounded-[14px] bg-accent-soft text-accent hover:bg-accent/25 hover:text-accent"
          variant="ghost"
          onClick={() => {
            typedInvoke("open_screenshare_viewer");
          }}
        >
          <HiOutlineEye className="size-4" />
          Open shared window
        </Button>
      )}

      {callTokens.role === ParticipantRole.SHARER && (
        <div className="flex items-center gap-1 rounded-[14px] bg-accent-soft py-1 pl-3 pr-1 ring-1 ring-inset ring-accent/20 animate-in fade-in-0 slide-in-from-bottom-1 duration-200">
          <span className="min-w-0 flex-1 truncate text-xs font-medium text-accent">Sharing your screen</span>
          <TooltipProvider>
            <Tooltip delayDuration={100}>
              <TooltipTrigger asChild>
                <span className="inline-flex">
                  <button
                    type="button"
                    aria-label={`${controllerCursorState ? "Disable" : "Enable"} remote control`}
                    aria-pressed={controllerCursorState}
                    onClick={toggleControllerCursor}
                    disabled={!accessibilityPermission}
                    className={clsx(
                      sharingToolClass,
                      controllerCursorState ?
                        "bg-accent text-accent-foreground"
                      : "text-muted-foreground hover:bg-subtle-strong",
                    )}
                  >
                    <HiOutlineCursorClick className="size-4" />
                    {!controllerCursorState && (
                      <HiLockClosed className="absolute bottom-1 right-1 size-2.5 text-destructive" />
                    )}
                  </button>
                </span>
              </TooltipTrigger>
              <TooltipContent side="top">
                {!accessibilityPermission ?
                  "Remote control needs Accessibility permission"
                : `${controllerCursorState ? "Disable" : "Enable"} remote control`}
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
          <DrawingEnableButton />
        </div>
      )}

      <MediaDevicesSettings
        micShortcut={userSettings?.shortcut_toggle_mic}
        cameraShortcut={userSettings?.shortcut_toggle_camera}
        screenShareShortcut={userSettings?.shortcut_toggle_screenshare}
        endCallShortcut={userSettings?.shortcut_end_call}
        onEndCall={handleEndCall}
      />
    </div>
  );
}

// Ticks on its own, so the rest of the call screen doesn't re-render every second
function CallTimer({ startedAt }: { startedAt: Date }) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const interval = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(interval);
  }, []);

  const totalSeconds = Math.max(0, Math.floor((now - new Date(startedAt).getTime()) / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = String(Math.floor((totalSeconds % 3600) / 60)).padStart(hours > 0 ? 2 : 1, "0");
  const seconds = String(totalSeconds % 60).padStart(2, "0");

  return <span className="tabular-nums">{hours > 0 ? `${hours}:${minutes}:${seconds}` : `${minutes}:${seconds}`}</span>;
}

function useCallTiles() {
  const coreParticipants = useStore((state) => state.callTokens?.participants ?? NO_PARTICIPANTS);
  const hasAudioEnabled = useStore((state) => state.callTokens?.hasAudioEnabled ?? true);
  const hasCameraEnabled = useStore((state) => state.callTokens?.hasCameraEnabled ?? false);
  const isSharing = useStore((state) => state.callTokens?.role === ParticipantRole.SHARER);
  const teammates = useStore((state) => state.teammates);
  const user = useStore((state) => state.user);

  return useMemo(() => {
    const extractUserId = (identity: string): string => {
      const parts = identity.split(":");
      return parts.length >= 4 ? (parts[2] ?? identity) : identity;
    };

    const findUser = (participantId: string) => {
      if (user && user.id === participantId) return user;
      return teammates?.find((t) => t.id === participantId) ?? null;
    };

    const localEntry =
      user ?
        {
          id: "local",
          user,
          name: `${user.first_name} ${user.last_name}`,
          isLocal: true,
          isMicrophoneEnabled: hasAudioEnabled,
          hasCamera: hasCameraEnabled,
          isScreensharing: isSharing,
          isCapturing: false,
        }
      : null;

    const remoteEntries = coreParticipants
      .filter((p) => p.connected)
      .filter((p) => {
        if (p.identity === "local") return false;
        const pid = extractUserId(p.identity);
        return pid !== user?.id;
      })
      .map((p) => {
        const teammate = findUser(extractUserId(p.identity));
        return {
          id: p.identity,
          user: teammate,
          name: teammate ? `${teammate.first_name} ${teammate.last_name}` : p.name || "Unknown user",
          isLocal: false,
          isMicrophoneEnabled: !p.muted,
          hasCamera: p.has_camera,
          isScreensharing: p.is_screensharing,
          isCapturing: !!p.capturing,
        };
      });

    return localEntry ? [localEntry, ...remoteEntries] : remoteEntries;
  }, [coreParticipants, teammates, user, hasAudioEnabled, hasCameraEnabled, isSharing]);
}

function useCallPeerName() {
  const peerId = useStore((state) => state.callTokens?.participant);
  const teammates = useStore((state) => state.teammates);
  return teammates?.find((teammate) => teammate.id === peerId)?.first_name;
}

function CallHeader({ onCopyRoomLink }: { onCopyRoomLink: () => void }) {
  const isReconnecting = useStore((state) => !!state.callTokens?.isReconnecting);
  const isInitialising = useStore((state) => !!state.callTokens?.isInitialisingCall);
  const timeStarted = useStore((state) => state.callTokens?.timeStarted);
  const room = useStore((state) => state.callTokens?.room);
  const peerName = useCallPeerName();
  const remoteTiles = useCallTiles().filter((tile) => !tile.isLocal);

  const remoteFirstNames = remoteTiles.map((tile) => tile.user?.first_name ?? tile.name);
  const title =
    room?.name ??
    (remoteFirstNames.length === 0 ?
      peerName ? `Pairing with ${peerName}`
      : "Pairing"
    : remoteFirstNames.length === 1 ? `Pairing with ${remoteFirstNames[0]}`
    : remoteFirstNames.length === 2 ? `Pairing with ${remoteFirstNames[0]} & ${remoteFirstNames[1]}`
    : `Pairing with ${remoteFirstNames.length} people`);

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex min-h-6 items-center justify-between gap-2">
        {isReconnecting ?
          <span className="inline-flex items-center gap-1.5 rounded-full bg-warning-soft px-2 py-0.5 text-[11px] leading-4 font-semibold text-warning">
            <CgSpinner className="size-3 animate-spin" />
            Reconnecting…
          </span>
        : isInitialising ?
          <span className="inline-flex items-center gap-1.5 rounded-full bg-subtle px-2 py-0.5 text-[11px] leading-4 font-semibold text-muted-foreground">
            <span className="size-1.5 animate-pulse rounded-full bg-current" />
            Connecting…
          </span>
        : <span className="inline-flex items-center gap-1.5 rounded-full bg-success-soft px-2 py-0.5 text-[11px] leading-4 font-semibold text-success">
            <span className="size-1.5 rounded-full bg-current" />
            Live
            {timeStarted && <CallTimer startedAt={timeStarted} />}
          </span>
        }
        {room && (
          <TooltipProvider delayDuration={100}>
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  onClick={onCopyRoomLink}
                  className="flex size-6 items-center justify-center rounded-lg text-muted-foreground transition-colors duration-150 hover:bg-subtle hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  aria-label="Copy room link"
                >
                  <HiMiniLink className="size-3.5" />
                </button>
              </TooltipTrigger>
              <TooltipContent side="bottom" className="flex flex-col items-center gap-0">
                <span>Copy room link for web</span>
                <span className="text-xs font-normal text-muted-foreground">Share with teammates</span>
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        )}
      </div>
      <h1 className="truncate text-[19px] leading-6 font-semibold tracking-[-0.015em] text-foreground">{title}</h1>
    </div>
  );
}

// Grows with the local mic level. Raw RMS for speech is ~0.001–0.03, so scale until peak speech fills it.
function MicLevelRing() {
  const micLevel = useStore((state) => state.callTokens?.micLevel ?? 0);
  const level = Math.min(1, Math.max(0, micLevel * 40));

  return (
    <div
      aria-hidden
      className="absolute inset-0 rounded-[17px] transition-[box-shadow] duration-100 ease-out"
      style={{
        boxShadow: `0 0 0 ${1 + level * 3.5}px color-mix(in srgb, var(--success) ${40 + level * 40}%, transparent)`,
      }}
    />
  );
}

/** Who is transcribing the call, so nobody is captured without knowing. */
function CaptureNotice() {
  const names = useCallTiles()
    .filter((tile) => tile.isCapturing)
    .map((tile) => tile.user?.first_name ?? tile.name);
  if (names.length === 0) return null;
  const who = names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
  return (
    <div
      role="status"
      className="flex items-center gap-2 rounded-[12px] bg-destructive/10 px-3 py-1.5 text-xs text-destructive animate-in fade-in-0 duration-300"
    >
      <LuAudioLines aria-hidden className="size-3.5 shrink-0" />
      <span className="truncate">
        {who} {names.length === 1 ? "is" : "are"} transcribing this call
      </span>
    </div>
  );
}

function CallParticipants() {
  const coreParticipantCount = useStore((state) => state.callTokens?.participants.length ?? 0);
  const isInitialising = useStore((state) => !!state.callTokens?.isInitialisingCall);
  const isRoomCall = useStore((state) => !!(state.callTokens?.isRoomCall || state.callTokens?.room));
  const setTab = useStore((state) => state.setTab);
  const peerName = useCallPeerName();
  const tiles = useCallTiles();
  const prevCountRef = useRef(coreParticipantCount);

  // Play sound when a new participant connects (count increases)
  useEffect(() => {
    if (coreParticipantCount > prevCountRef.current && prevCountRef.current !== 0) {
      sounds.callAccepted.play();
    }
    prevCountRef.current = coreParticipantCount;
  }, [coreParticipantCount]);

  const isAlone = tiles.every((tile) => tile.isLocal);
  const tileCount = tiles.length + (isAlone ? 1 : 0);

  return (
    <div
      className={clsx(
        "grid min-h-0 flex-1 gap-2 overflow-y-auto",
        tileCount <= 2 ? "grid-cols-1 auto-rows-fr"
        : tileCount <= 4 ? "grid-cols-2 auto-rows-fr"
        : "grid-cols-2 auto-rows-[112px]",
      )}
    >
      {tiles.map((tile) => (
        <div
          key={tile.id}
          className="relative flex min-h-0 flex-col items-center justify-center gap-2 overflow-hidden rounded-2xl bg-subtle px-2 py-2 ring-1 ring-inset ring-border animate-in fade-in-0 zoom-in-95 duration-300"
        >
          {/* A wash of the person's own colour, so tiles are easy to tell apart at a glance */}
          <div
            aria-hidden
            className="absolute inset-0 opacity-25 [mask-image:radial-gradient(90%_90%_at_50%_0%,black,transparent)]"
            style={{ background: avatarGradient((tile.user?.first_name ?? "") + (tile.user?.last_name ?? "")) }}
          />
          <div className="absolute right-2 top-2 flex items-center gap-1 text-muted-foreground">
            {tile.hasCamera && <LuVideo aria-label="Camera on" className="size-3.5 text-success" />}
            {tile.isScreensharing && <LuScreenShare aria-label="Sharing screen" className="size-3.5 text-accent" />}
            {tile.isCapturing && (
              <LuAudioLines aria-label="Transcribing this call" className="size-3.5 text-destructive" />
            )}
          </div>
          <div className="relative">
            {tile.isLocal && tile.isMicrophoneEnabled && <MicLevelRing />}
            {tile.user ?
              <HoppAvatar
                src={tile.user.avatar_url || undefined}
                firstName={tile.user.first_name}
                lastName={tile.user.last_name}
                className="size-[52px] rounded-[17px] [&_span]:text-[16px]"
              />
            : <div className="flex size-[52px] items-center justify-center rounded-[17px] bg-subtle-strong text-base font-semibold text-muted-foreground">
                ?
              </div>
            }
            {!tile.isMicrophoneEnabled && (
              <span
                aria-label="Muted"
                className="absolute -bottom-1 -right-1 flex size-[22px] items-center justify-center rounded-full bg-destructive text-destructive-foreground ring-[3px] ring-background"
              >
                <LuMicOff className="size-3" />
              </span>
            )}
          </div>
          <span className="relative max-w-full truncate text-[13px] leading-[18px] font-medium text-foreground">
            {tile.name}
            {tile.isLocal && <span className="font-normal text-muted-foreground"> (You)</span>}
          </span>
        </div>
      ))}

      {isAlone && (
        <div className="flex min-h-0 flex-col items-center justify-center gap-1.5 rounded-2xl border border-dashed border-border-strong px-3 py-2 text-center animate-in fade-in-0 duration-300">
          {isInitialising ?
            <>
              <CgSpinner className="size-5 animate-spin text-faint-foreground" />
              <span className="text-xs text-muted-foreground">Connecting your call…</span>
            </>
          : <>
              <span className="text-xs text-muted-foreground">
                {isRoomCall ?
                  "No one else is here yet"
                : peerName ?
                  `Waiting for ${peerName}…`
                : "Waiting for your teammate…"}
              </span>
              {isRoomCall && (
                <Button size="sm" variant="ghost" className="h-7" onClick={() => setTab("user-list")}>
                  Invite teammates
                </Button>
              )}
            </>
          }
        </div>
      )}
    </div>
  );
}

function DrawingEnableButton() {
  const [drawingPermanent, setDrawingPermanent] = useState(false);
  // Drawing and stamps turn each other off.
  const [activeTool, setActiveTool] = useState<"draw" | "stamp" | null>(null);
  const drawingEnabled = activeTool === "draw";
  const stampEnabled = activeTool === "stamp";
  const [dropdownOpen, setDropdownOpen] = useState(false);

  useEffect(() => {
    const loadPreferences = async () => {
      try {
        setDrawingPermanent(await tauriUtils.getSharerDrawPersist());
      } catch (error) {
        console.error("Failed to load drawing preferences:", error);
      }
    };
    loadPreferences();
  }, []);

  useEffect(() => {
    const unlisten = listen("core_drawing_disabled", () => {
      setActiveTool(null);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const handlePermanentToggle = async (checked: boolean) => {
    setDrawingPermanent(checked);
    try {
      await tauriUtils.setSharerDrawPersist(checked);
    } catch (error) {
      console.error("Failed to save drawing permanent preference:", error);
    }
  };

  const handleToggleTool = async (tool: "draw" | "stamp") => {
    const newEnabled = activeTool !== tool;
    const label = tool === "draw" ? "drawing" : "stamps";
    try {
      await tauriUtils.setDrawingEnabled(newEnabled, drawingPermanent, tool === "stamp");
      setActiveTool(newEnabled ? tool : null);
    } catch (error) {
      console.error(`Failed to toggle ${label}:`, error);
      toast.error(`Failed to toggle ${label}`, { duration: 2500 });
    }
  };

  return (
    <div className="inline-flex">
      <TooltipProvider>
        <Tooltip delayDuration={100}>
          <TooltipTrigger asChild>
            <button
              type="button"
              aria-label={drawingEnabled ? "Disable drawing" : "Enable drawing"}
              aria-pressed={drawingEnabled}
              onClick={() => handleToggleTool("draw")}
              className={clsx(
                sharingToolClass,
                drawingEnabled ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:bg-subtle-strong",
              )}
            >
              {drawingEnabled ?
                <PiCursorBold className="size-4" />
              : <PiScribbleLoopBold className="size-4" />}
            </button>
          </TooltipTrigger>
          <TooltipContent side="top">{drawingEnabled ? "Disable drawing" : "Enable drawing"}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
      <DropdownMenu open={dropdownOpen} onOpenChange={setDropdownOpen}>
        <TooltipProvider>
          <Tooltip delayDuration={100}>
            <TooltipTrigger asChild>
              <DropdownMenuTrigger asChild>
                <button
                  type="button"
                  className={clsx(sharingToolClass, "w-6 text-muted-foreground hover:bg-subtle-strong")}
                  aria-label="Drawing options"
                >
                  <MoreHorizontal className="size-4" />
                </button>
              </DropdownMenuTrigger>
            </TooltipTrigger>
            <TooltipContent side="top">Drawing options</TooltipContent>
          </Tooltip>
        </TooltipProvider>
        <DropdownMenuContent
          onCloseAutoFocus={(e) => e.preventDefault()}
          align="end"
          side="top"
          className="w-auto min-w-[200px]"
        >
          <DropdownMenuCheckboxItem checked={drawingPermanent} onCheckedChange={handlePermanentToggle}>
            <span>Persist until right click</span>
          </DropdownMenuCheckboxItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <TooltipProvider>
        <Tooltip delayDuration={100}>
          <TooltipTrigger asChild>
            <button
              type="button"
              aria-label={stampEnabled ? "Stop stamping" : "Place stamps"}
              aria-pressed={stampEnabled}
              onClick={() => handleToggleTool("stamp")}
              className={clsx(
                sharingToolClass,
                stampEnabled ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:bg-subtle-strong",
              )}
            >
              {stampEnabled ?
                <PiCursorBold className="size-4" />
              : <PiMapPinBold className="size-4" />}
            </button>
          </TooltipTrigger>
          <TooltipContent side="top">{stampEnabled ? "Stop stamping" : "Place stamps"}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    </div>
  );
}

/**
 * MicrophoneIcon — uses typedInvoke to list/select microphones and toggle mute via core.
 */
function MicrophoneIcon({ shortcut }: { shortcut?: string }) {
  const { updateCallTokens, callTokens } = useStore();
  const hasAudioEnabled = callTokens?.hasAudioEnabled || false;

  const { data: microphoneDevices = [], refetch: refetchMics } = useQuery({
    queryKey: ["list_microphones"],
    enabled: !callTokens?.isInitialisingCall,
    queryFn: async () => typedInvoke("list_microphones"),
    select: (data) => data.sort((a, b) => a.name.localeCompare(b.name)),
  });

  const [activeMicId, setActiveMicId] = useState<string>("");
  const [tooltipOpen, setTooltipOpen] = useState(false);
  const [selectOpen, setSelectOpen] = useState(false);
  const suppressTooltipRef = useRef(false);

  const handleTooltipOpenChange = useCallback(
    (open: boolean) => {
      if (!open) {
        setTooltipOpen(false);
        return;
      }

      if (selectOpen || suppressTooltipRef.current) return;
      setTooltipOpen(true);
    },
    [selectOpen],
  );

  useEffect(() => {
    if (!microphoneDevices.length) return;
    if (activeMicId) return;
    let cancelled = false;
    typedInvoke("get_preferred_microphone")
      .then((name) => {
        if (!cancelled) setActiveMicId((current) => current || name);
      })
      .catch((error) => console.error("Failed to resolve microphone:", error));
    return () => {
      cancelled = true;
    };
  }, [microphoneDevices, activeMicId]);

  useEffect(() => {
    const unlisten = listen<string>("core_active_mic_changed", (event) => {
      setActiveMicId(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const handleMicToggle = useCallback(() => {
    const newState = !hasAudioEnabled;
    updateCallTokens({ hasAudioEnabled: newState });
    if (newState) {
      typedInvoke("unmute_mic");
    } else {
      typedInvoke("mute_mic");
    }
  }, [hasAudioEnabled, updateCallTokens]);

  const handleMicrophoneChange = useCallback((value: string) => {
    setActiveMicId(value);
    typedInvoke("select_microphone", { deviceName: value });
    tauriUtils.setLastUsedMic(value);
  }, []);

  const handleDropdownOpenChange = useCallback(
    (open: boolean) => {
      setSelectOpen(open);
      if (open) {
        setTooltipOpen(false);
        suppressTooltipRef.current = false;
        refetchMics();
      } else {
        suppressTooltipRef.current = true;
        setTooltipOpen(false);
      }
    },
    [refetchMics],
  );

  return (
    <div className="relative min-w-0">
      <TooltipProvider>
        <Tooltip delayDuration={100} open={tooltipOpen} onOpenChange={handleTooltipOpenChange}>
          <TooltipTrigger asChild>
            <DockButton
              onPointerEnter={() => {
                if (!selectOpen) {
                  suppressTooltipRef.current = false;
                }
              }}
              onClick={handleMicToggle}
              tone={hasAudioEnabled ? "neutral" : "danger"}
              aria-pressed={!hasAudioEnabled}
              icon={
                hasAudioEnabled ?
                  <CustomIcons.MicWithLevel level={callTokens?.micLevel ?? 0} className="size-[18px]" />
                : <LuMicOff className="size-[18px]" />
              }
              label={hasAudioEnabled ? "Mute" : "Unmute"}
            />
          </TooltipTrigger>
          <ShortcutTooltip shortcut={shortcut} />
        </Tooltip>
      </TooltipProvider>
      <span
        className={clsx("absolute right-0.5 top-0.5", hasAudioEnabled ? "text-muted-foreground" : "text-destructive")}
      >
        <Select value={activeMicId} onValueChange={handleMicrophoneChange} onOpenChange={handleDropdownOpenChange}>
          <SelectTrigger aria-label="Choose microphone" iconClassName="size-3.5" className={deviceMenuTriggerClass} />
          <SelectPortal container={document.getElementsByClassName("container")[0]}>
            <SelectContent align="center">
              {microphoneDevices.map(
                (device) =>
                  device.name !== "" && (
                    <SelectItem key={device.name} value={device.name}>
                      <span className="text-xs truncate">{device.name}</span>
                    </SelectItem>
                  ),
              )}
            </SelectContent>
          </SelectPortal>
        </Select>
      </span>
    </div>
  );
}

function ScreenShareIcon({ callTokens, shortcut }: { callTokens: CallState | null; shortcut?: string }) {
  const { setCallTokens } = useStore();

  const toggleScreenShare = useCallback(async () => {
    if (!callTokens) return;

    if (callTokens.role === ParticipantRole.NONE || callTokens.role === ParticipantRole.CONTROLLER) {
      tauriUtils.getAvailableContent();
    } else if (callTokens.role === ParticipantRole.SHARER) {
      setCallTokens({
        ...callTokens,
        role: ParticipantRole.NONE,
        isRemoteControlEnabled: true,
      });
      tauriUtils.stopSharing();
    }
  }, [callTokens, setCallTokens]);

  const changeScreenShare = useCallback(() => {
    if (!callTokens) return;
    tauriUtils.getAvailableContent();
  }, [callTokens]);

  const isSharing = callTokens?.role === ParticipantRole.SHARER;

  return (
    <div className="relative min-w-0">
      <TooltipProvider>
        <Tooltip delayDuration={100}>
          <TooltipTrigger asChild>
            <DockButton
              onClick={toggleScreenShare}
              tone={isSharing ? "accent" : "neutral"}
              aria-pressed={isSharing}
              icon={
                isSharing ? <LuScreenShareOff className="size-[18px]" /> : <LuScreenShare className="size-[18px]" />
              }
              label={isSharing ? "Stop" : "Share"}
            />
          </TooltipTrigger>
          <ShortcutTooltip shortcut={shortcut} />
        </Tooltip>
      </TooltipProvider>
      {isSharing && (
        <span className="absolute right-0.5 top-0.5 text-accent">
          <button
            type="button"
            aria-label="Change what you're sharing"
            onClick={changeScreenShare}
            className={deviceMenuTriggerClass}
          >
            <ChevronDownIcon className="size-3.5" />
          </button>
        </span>
      )}
    </div>
  );
}

function CameraIcon({ shortcut }: { shortcut?: string }) {
  const { updateCallTokens, callTokens } = useStore();
  const cameraEnabled = callTokens?.hasCameraEnabled || false;

  const { data: cameraDevices = [], refetch: refetchCameras } = useQuery({
    queryKey: ["list_webcams"],
    queryFn: () => typedInvoke("list_webcams"),
    select: (data) => data.sort((a, b) => a.name.localeCompare(b.name)),
  });

  const [activeCamera, setActiveCamera] = useState<string>("");
  const [tooltipOpen, setTooltipOpen] = useState(false);
  const [selectOpen, setSelectOpen] = useState(false);
  const suppressTooltipRef = useRef(false);

  const handleTooltipOpenChange = useCallback(
    (open: boolean) => {
      if (!open) {
        setTooltipOpen(false);
        return;
      }

      if (selectOpen || suppressTooltipRef.current) return;
      setTooltipOpen(true);
    },
    [selectOpen],
  );

  useEffect(() => {
    if (!cameraDevices.length) return;
    if (activeCamera) return;
    const resolve = async () => {
      const lastUsedCamera = await tauriUtils.getLastUsedCamera();
      if (lastUsedCamera && cameraDevices.find((d) => d.name === lastUsedCamera)) {
        setActiveCamera(lastUsedCamera);
        return;
      }
      const defaultDevice = cameraDevices.find((d) => d.default) ?? cameraDevices[0];
      if (defaultDevice) setActiveCamera(defaultDevice.name);
    };
    resolve();
  }, [cameraDevices]);

  useEffect(() => {
    const unlisten = listen<string>("core_active_camera_changed", (event) => {
      setActiveCamera(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const handleCameraToggle = useCallback(async () => {
    const newEnabled = !cameraEnabled;
    updateCallTokens({ hasCameraEnabled: newEnabled });

    if (newEnabled) {
      try {
        await typedInvoke("start_camera", { deviceName: activeCamera || undefined });
      } catch (error) {
        console.error("Failed to start camera:", error);
        toast.error("Failed to initialize camera", { duration: 2500 });
        updateCallTokens({ hasCameraEnabled: false });
      }
    } else {
      // TODO(@konsalex): This is an optimistic update, not sure how I feel about this.
      // We may need to make this sync somehow.
      typedInvoke("stop_camera");
    }
  }, [cameraEnabled, updateCallTokens, activeCamera]);

  const handleCameraChange = useCallback(
    async (deviceName: string) => {
      setActiveCamera(deviceName);
      tauriUtils.setLastUsedCamera(deviceName);
      if (cameraEnabled) {
        try {
          await typedInvoke("start_camera", { deviceName });
        } catch (error) {
          console.error("Failed to switch camera:", error);
        }
      }
    },
    [cameraEnabled],
  );

  const handleDropdownOpenChange = useCallback(
    (open: boolean) => {
      setSelectOpen(open);
      if (open) {
        setTooltipOpen(false);
        suppressTooltipRef.current = false;
        refetchCameras();
      } else {
        suppressTooltipRef.current = true;
        setTooltipOpen(false);
      }
    },
    [refetchCameras],
  );

  return (
    <div className="relative min-w-0">
      <TooltipProvider>
        <Tooltip delayDuration={100} open={tooltipOpen} onOpenChange={handleTooltipOpenChange}>
          <TooltipTrigger asChild>
            <DockButton
              onPointerEnter={() => {
                if (!selectOpen) {
                  suppressTooltipRef.current = false;
                }
              }}
              onClick={handleCameraToggle}
              tone={cameraEnabled ? "success" : "neutral"}
              aria-pressed={cameraEnabled}
              icon={cameraEnabled ? <LuVideo className="size-[18px]" /> : <LuVideoOff className="size-[18px]" />}
              label={cameraEnabled ? "Stop cam" : "Start cam"}
            />
          </TooltipTrigger>
          <ShortcutTooltip shortcut={shortcut} />
        </Tooltip>
      </TooltipProvider>
      <span className={clsx("absolute right-0.5 top-0.5", cameraEnabled ? "text-success" : "text-muted-foreground")}>
        <Select value={activeCamera} onValueChange={handleCameraChange} onOpenChange={handleDropdownOpenChange}>
          <SelectTrigger aria-label="Choose camera" iconClassName="size-3.5" className={deviceMenuTriggerClass} />
          <SelectPortal container={document.getElementsByClassName("container")[0]}>
            <SelectContent align="center">
              {cameraDevices.map(
                (device) =>
                  device.name !== "" && (
                    <SelectItem key={device.name} value={device.name}>
                      <span className="text-xs truncate">{device.name}</span>
                    </SelectItem>
                  ),
              )}
            </SelectContent>
          </SelectPortal>
        </Select>
      </span>
    </div>
  );
}

function MediaDevicesSettings({
  micShortcut,
  cameraShortcut,
  screenShareShortcut,
  endCallShortcut,
  onEndCall,
}: {
  micShortcut?: string;
  cameraShortcut?: string;
  screenShareShortcut?: string;
  endCallShortcut?: string;
  onEndCall: () => void;
}) {
  const { callTokens } = useStore();

  return (
    <div className="grid w-full shrink-0 grid-cols-4 gap-1.5">
      <MicrophoneIcon shortcut={micShortcut} />
      <CameraIcon shortcut={cameraShortcut} />
      <ScreenShareIcon callTokens={callTokens} shortcut={screenShareShortcut} />
      <TooltipProvider>
        <Tooltip delayDuration={100}>
          <TooltipTrigger asChild>
            <DockButton tone="end" onClick={onEndCall} icon={<LuPhoneOff className="size-[18px]" />} label="End" />
          </TooltipTrigger>
          <ShortcutTooltip shortcut={endCallShortcut} />
        </Tooltip>
      </TooltipProvider>
    </div>
  );
}
