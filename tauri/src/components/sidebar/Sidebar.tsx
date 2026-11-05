import React, { useEffect, useMemo } from "react";
import throttle from "lodash/throttle";
import toast from "react-hot-toast";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  HiArrowDownTray,
  HiCheck,
  HiOutlineArrowRightOnRectangle,
  HiOutlineBugAnt,
  HiOutlineComputerDesktop,
  HiOutlineChatBubbleBottomCenterText,
  HiOutlineCog6Tooth,
  HiOutlineLockOpen,
  HiOutlineMinus,
  HiOutlineMoon,
  HiOutlinePower,
  HiOutlineSun,
  HiOutlineUserCircle,
  HiOutlineUserPlus,
  HiOutlineUsers,
} from "react-icons/hi2";
import { CgSpinner } from "react-icons/cg";
import { differenceInDays, parseISO } from "date-fns";
import { clsx } from "clsx";
import { motion } from "framer-motion";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { invoke } from "@tauri-apps/api/core";
import useStore, { Tab } from "@/store/store";
import { components } from "@/openapi";
import { HiOutlineAnnotation, HiOutlineDotsHorizontal, HiOutlineUserGroup } from "react-icons/hi";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { avatarGradient } from "@/components/ui/hopp-avatar";
import { USER_STATUS_DOTS, USER_STATUS_LABELS, USER_STATUSES, UserStatus, useSetUserStatus } from "@/lib/user-status";
import { appVersion, tauriUtils } from "@/windows/window-utils.ts";
import { Constants, OS } from "@/constants";
import { useQueryClient } from "@tanstack/react-query";
import { downloadAndRelaunch, hasPendingUpdate, installAndRelaunch } from "@/update";
import { LuCircleFadingArrowUp } from "react-icons/lu";
import { FiPhoneCall } from "react-icons/fi";
import hotkeys from "hotkeys-js";
import logo from "@/assets/Hopp.png";
import { setThemePreference, ThemePreference, useThemePreference } from "@/lib/theme";

// The search field is usually focused when the window opens, and hotkeys-js ignores text fields by default
hotkeys.filter = (event) => event.metaKey || event.ctrlKey;

const railButtonClass =
  "relative flex size-9 items-center justify-center rounded-[11px] transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

const SidebarButton = ({
  active,
  children,
  label,
  ...rest
}: {
  label: React.ReactNode;
  active?: boolean;
} & React.ButtonHTMLAttributes<HTMLButtonElement>) => {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label={typeof label === "string" ? label : undefined}
          aria-current={active ? "page" : undefined}
          className={clsx(
            railButtonClass,
            active ? "text-foreground" : "text-faint-foreground hover:bg-subtle hover:text-foreground",
          )}
          {...rest}
        >
          {active && (
            <motion.span
              layoutId="sidebar-active-tab"
              className="absolute inset-0 rounded-[11px] bg-subtle-strong ring-1 ring-inset ring-border"
              transition={{ type: "spring", stiffness: 520, damping: 38 }}
            />
          )}
          <span className="relative">{children}</span>
        </button>
      </TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
};

const getAvailableTabs = (
  hasUser: boolean,
): Array<{
  label: string;
  icon: React.ReactNode;
  key: Tab;
}> => {
  const baseTabs =
    !hasUser ?
      [
        {
          label: "Login",
          icon: <HiOutlineLockOpen className="size-[18px]" />,
          key: "login",
        } as const,
      ]
    : [
        {
          label: "People",
          icon: <HiOutlineUsers className="size-[18px]" />,
          key: "user-list",
        } as const,
        {
          label: "Rooms",
          icon: <HiOutlineUserGroup className="size-[18px] stroke-[1.5]" />,
          key: "rooms",
        } as const,
        {
          label: "Invite",
          icon: <HiOutlineUserPlus className="size-[18px]" />,
          key: "invite",
        } as const,
        {
          label: "Broken again?",
          icon: <HiOutlineAnnotation className="size-[18px] stroke-[1.5]" />,
          key: "report-issue",
        } as const,
      ];

  return [
    ...baseTabs,
    // ...[
    //   {
    //     label: "Debug",
    //     icon: <HiOutlineBugAnt className="size-4" />,
    //     key: "debug",
    //   } as const,
    // ],
  ];
};

const DownloadNewVersionButton = () => {
  const { needsUpdate, updateInProgress, setUpdateInProgress } = useStore();

  if (!needsUpdate) {
    return null;
  }

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label="Download and install update"
          className={clsx(railButtonClass, "bg-accent-soft text-accent hover:scale-[1.04] duration-200")}
          onClick={() => {
            setUpdateInProgress(true);
            if (OS === "macos" && hasPendingUpdate()) {
              installAndRelaunch();
              return;
            }
            downloadAndRelaunch();
          }}
          disabled={updateInProgress}
        >
          {updateInProgress ?
            <CgSpinner className="animate-spin size-4" />
          : <LuCircleFadingArrowUp className="size-4" />}
        </button>
      </TooltipTrigger>
      <TooltipContent side="right">Download and install update</TooltipContent>
    </Tooltip>
  );
};

const TrialCountdownAvatarFill = ({ user }: { user: components["schemas"]["PrivateUser"] }) => {
  const handleClick = useMemo(
    () =>
      throttle(
        () => {
          if (user.is_admin) {
            openUrl(new URL("/subscription", Constants.webAppUrl).toString());
          } else {
            toast("Contact your admin to manage your team's subscription.", { duration: 3000 });
          }
        },
        2000,
        { leading: true, trailing: false },
      ),
    [user.is_admin],
  );

  // Only show if user is in trial
  if (!user.is_trial || !user.trial_ends_at) {
    return null;
  }

  // Uncomment and modify value to test visual changes
  // const end = "2025-10-05T17:20:32.677+02:00";
  // const trialEndDate = parseISO(end);
  const trialEndDate = parseISO(user.trial_ends_at);
  const currentDate = new Date();
  const daysRemaining = differenceInDays(trialEndDate, currentDate);
  const isExpired = daysRemaining <= 0;
  const displayDays = Math.max(0, daysRemaining);

  const maxTrialDays = 14;
  const percentage = isExpired ? 100 : Math.min(100, Math.max(5, (daysRemaining / maxTrialDays) * 100));

  // 14-day trial thresholds: amber ≤7, red ≤1.
  const warningAt = 7;
  const urgentAt = 1;

  const getTextColor = (days: number) => {
    if (days <= urgentAt) return "text-destructive";
    if (days <= warningAt) return "text-warning";
    return "text-success";
  };

  const textColor = isExpired ? "text-destructive" : getTextColor(daysRemaining);

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div
          className={clsx(
            "relative flex size-9 items-center justify-center overflow-hidden rounded-[11px] bg-subtle text-[13px] font-semibold ring-1 ring-inset ring-border",
            textColor,
          )}
          onClick={handleClick}
        >
          {/* Background fill from bottom */}
          <div
            className="absolute bottom-0 left-0 right-0 bg-current opacity-20 transition-all duration-300"
            style={{ height: `${percentage}%` }}
          />
          {/* Content */}
          <span className="relative z-10">{displayDays}</span>
        </div>
      </TooltipTrigger>
      <TooltipContent side="right">
        {isExpired ?
          "Trial expired, click to manage subscription"
        : `Trial expires in ${daysRemaining} day${daysRemaining !== 1 ? "s" : ""}, click to manage`}
      </TooltipContent>
    </Tooltip>
  );
};

const CallPageButton = () => {
  const { tab, setTab, callTokens } = useStore();

  // Local shortcut to jump to the call page while in a call. hotkeys-js binds to
  // the webview document, so it only fires when the main window is focused.
  useEffect(() => {
    if (!callTokens) return;
    hotkeys("cmd+o, ctrl+o", (event) => {
      event.preventDefault();
      setTab("call");
    });
    return () => hotkeys.unbind("cmd+o, ctrl+o");
  }, [callTokens, setTab]);

  if (!callTokens) return null;

  const active = tab === "call";
  const shortcutLabel = OS === "macos" ? "⌘O" : "Ctrl+O";

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label="Ongoing call"
          aria-current={active ? "page" : undefined}
          onClick={() => setTab("call")}
          className={clsx(
            railButtonClass,
            "animate-in fade-in-0 zoom-in-90 duration-300",
            active ? "bg-success text-success-foreground" : "bg-success-soft text-success hover:bg-success/25",
          )}
        >
          {!active && (
            <span className="absolute inset-0 animate-ping rounded-[11px] bg-success/20 [animation-duration:2s]" />
          )}
          {active ?
            <motion.span
              layoutId="sidebar-active-tab"
              className="absolute inset-0 rounded-[11px]"
              transition={{ type: "spring", stiffness: 520, damping: 38 }}
            />
          : null}
          <FiPhoneCall className="relative size-4" />
        </button>
      </TooltipTrigger>
      <TooltipContent side="right" className="flex flex-row items-center">
        Ongoing call <kbd className="ml-1 max-w-min">{shortcutLabel}</kbd>
      </TooltipContent>
    </Tooltip>
  );
};

const THEME_OPTIONS: { value: ThemePreference; label: string; icon: React.ReactNode }[] = [
  { value: "system", label: "Auto", icon: <HiOutlineComputerDesktop /> },
  { value: "light", label: "Light", icon: <HiOutlineSun /> },
  { value: "dark", label: "Dark", icon: <HiOutlineMoon /> },
];

const ThemeSwitch = () => {
  const preference = useThemePreference();

  return (
    <div role="group" aria-label="Appearance" className="grid grid-cols-3 gap-0.5 rounded-[10px] bg-subtle p-0.5">
      {THEME_OPTIONS.map((option) => (
        <DropdownMenuItem
          key={option.value}
          aria-checked={preference === option.value}
          // Keeps the menu open, so the change can be seen and undone
          onSelect={(event) => {
            event.preventDefault();
            setThemePreference(option.value);
          }}
          className={clsx(
            "flex-col gap-0.5 rounded-lg px-1 py-1 text-[11px] leading-4",
            preference === option.value ?
              "bg-subtle-strong text-foreground shadow-card focus:bg-subtle-strong"
            : "text-muted-foreground",
          )}
        >
          {option.icon}
          {option.label}
        </DropdownMenuItem>
      ))}
    </div>
  );
};

const StatusPicker = ({ status }: { status: UserStatus }) => {
  const setUserStatus = useSetUserStatus();

  return (
    <div role="group" aria-label="Your status">
      {USER_STATUSES.map((option) => (
        <DropdownMenuItem
          key={option}
          aria-checked={status === option}
          onClick={() => {
            setUserStatus(option).catch(() => toast.error("Couldn't change your status"));
          }}
        >
          <span aria-hidden className={clsx("mx-1 size-2 rounded-full", USER_STATUS_DOTS[option])} />
          {USER_STATUS_LABELS[option]}
          {status === option && <HiCheck className="ml-auto" />}
        </DropdownMenuItem>
      ))}
    </div>
  );
};

export const Sidebar = () => {
  const { tab, setTab, user, reset, missedCalls } = useStore();
  const queryClient = useQueryClient();

  useEffect(() => {
    // If user is not set, show login tab
    if (!user) {
      setTab("login");
    }
  }, [user]);

  // Logged out on the login tab the rail steps aside, leaving only its menu
  const isRailHidden = !user && tab === "login";

  return (
    <TooltipProvider delayDuration={250} skipDelayDuration={100}>
      <div
        className={clsx(
          "flex w-[52px] flex-col items-center gap-3 py-2.5",
          isRailHidden ? "absolute bottom-0 left-0 z-30" : "min-w-[52px] border-r border-border bg-rail",
        )}
      >
        {!isRailHidden && (
          <>
            <img
              src={logo}
              alt=""
              draggable="false"
              className="size-[30px] rounded-[9px] shadow-card ring-1 ring-border-strong"
            />

            <nav className="flex flex-col items-center gap-1">
              {getAvailableTabs(!!user).map((t) => {
                const missedCount = t.key === "user-list" ? missedCalls.length : 0;
                return (
                  <SidebarButton
                    key={t.key}
                    active={t.key === tab}
                    label={missedCount > 0 ? `${t.label} · ${missedCount} missed` : t.label}
                    onClick={() => setTab(t.key)}
                  >
                    {t.icon}
                    {missedCount > 0 && t.key !== tab && (
                      <span className="absolute -right-0.5 -top-0.5 size-2 rounded-full bg-destructive ring-2 ring-rail" />
                    )}
                  </SidebarButton>
                );
              })}
              {OS === "windows" && (
                <SidebarButton label="Minimize" onClick={() => tauriUtils.minimizeMainWindow()}>
                  <HiOutlineMinus className="size-[18px]" />
                </SidebarButton>
              )}
            </nav>
          </>
        )}

        <CallPageButton />

        {/* Bottom user section */}
        <div className="mt-auto flex flex-col items-center gap-2">
          <DownloadNewVersionButton />
          {user && <TrialCountdownAvatarFill user={user} />}
          <DropdownMenu>
            <DropdownMenuTrigger
              aria-label="Account menu"
              className="relative rounded-[11px] transition-transform duration-150 hover:scale-[1.04] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              {!user && (
                <div className="flex size-9 shrink-0 items-center justify-center rounded-[11px] bg-subtle text-muted-foreground ring-1 ring-inset ring-border">
                  <HiOutlineDotsHorizontal />
                </div>
              )}
              {user && (
                <div
                  className="flex size-9 shrink-0 items-center justify-center rounded-[11px] text-[13px] font-semibold text-white shadow-card ring-1 ring-inset ring-white/15"
                  style={{
                    background:
                      user.avatar_url ?
                        `url(${user.avatar_url}) center center/cover no-repeat`
                      : avatarGradient(user.first_name + user.last_name),
                  }}
                >
                  {user.avatar_url ? "" : user.first_name.charAt(0).toUpperCase()}
                </div>
              )}
              {(user?.status === "away" || user?.status === "dnd") && (
                <span
                  aria-label={USER_STATUS_LABELS[user.status]}
                  className={clsx(
                    "absolute -bottom-0.5 -right-0.5 size-[11px] rounded-full border-[2.5px] border-rail",
                    USER_STATUS_DOTS[user.status],
                  )}
                />
              )}
            </DropdownMenuTrigger>
            <DropdownMenuContent className="w-[220px]" side="top" align="start" sideOffset={6}>
              {user && (
                <>
                  <div className="flex flex-col px-2 pb-1.5 pt-1">
                    <span className="truncate text-[13px] leading-[18px] font-medium text-foreground">
                      {user.first_name} {user.last_name}
                    </span>
                    <span className="truncate text-xs leading-4 text-muted-foreground">{user.email}</span>
                  </div>
                  <DropdownMenuSeparator />
                  {user.status && (
                    <>
                      <StatusPicker status={user.status} />
                      <DropdownMenuSeparator />
                    </>
                  )}
                </>
              )}
              <DropdownMenuItem onClick={() => openUrl(Constants.webAppUrl)}>
                <HiOutlineUserCircle />
                Profile
              </DropdownMenuItem>
              {OS === "macos" && (
                <DropdownMenuItem onClick={() => tauriUtils.openCapturesWindow()}>
                  <HiOutlineChatBubbleBottomCenterText />
                  Captures
                </DropdownMenuItem>
              )}
              <DropdownMenuItem onClick={() => tauriUtils.openSettingsWindow()}>
                <HiOutlineCog6Tooth />
                Settings
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => setTab("debug")}>
                <HiOutlineBugAnt />
                Debug
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <ThemeSwitch />
              <DropdownMenuSeparator />
              <DropdownMenuItem
                onClick={async () => {
                  queryClient.clear();
                  reset();
                  await invoke("delete_stored_token");
                }}
              >
                <HiOutlineArrowRightOnRectangle />
                Sign out
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => invoke("quit_app")}>
                <HiOutlinePower />
                Quit
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <div className="px-2 pb-0.5 pt-1 text-[11px] leading-4 text-faint-foreground">Version {appVersion}</div>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
    </TooltipProvider>
  );
};
