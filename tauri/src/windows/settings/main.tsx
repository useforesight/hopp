import "../../App.css";
import React, { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import ReactDOM from "react-dom/client";
import clsx from "clsx";
import {
  HiCheck,
  HiMagnifyingGlass,
  HiOutlineAdjustmentsHorizontal,
  HiOutlineChatBubbleBottomCenterText,
  HiOutlineEyeSlash,
  HiOutlinePhone,
  HiOutlineSwatch,
  HiXMark,
} from "react-icons/hi2";
import { LuCommand } from "react-icons/lu";
import { useDisableNativeContextMenu } from "@/lib/hooks";
import { initTheme, setThemePreference, useThemePreference, type ThemePreference } from "@/lib/theme";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  typedInvoke,
  type AppVeilApplication,
  type CaptureChange,
  type CaptureModel,
  type InstalledApplication,
  type ScreenSharePickerMode,
  type ScreenShareResolution,
} from "@/core_payloads";
import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { tauriUtils } from "@/windows/window-utils";
import { OS, URLS } from "@/constants";
import posthog from "posthog-js";
import useStore from "@/store/store";

const queryClient = new QueryClient();

initTheme();

const screenShareResolutionItems = [
  { id: "P1080", title: "1080p", description: "Lowest latency" },
  { id: "P1440", title: "1440p", description: "Balance latency and resolution" },
  { id: "P4K", title: "4K", description: "Maximum resolution" },
] satisfies Array<{ id: ScreenShareResolution; title: string; description: string }>;

const screenSharePickerModeItems = [
  { id: "Screen", title: "Screen", description: "Choose an entire display" },
  { id: "Window", title: "Window", description: "Choose a single app window" },
] satisfies Array<{ id: ScreenSharePickerMode; title: string; description: string }>;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <SettingsWindow />
    </QueryClientProvider>
  </React.StrictMode>,
);

const settingTitleClass = "text-[13px] leading-[18px] font-medium text-foreground";
const settingDescriptionClass = "text-xs leading-[18px] text-muted-foreground";

function SettingsGroup({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2">
      {title && (
        <h2 className="border-none pb-0 pl-0.5 text-xs leading-4 font-medium tracking-normal text-muted-foreground">
          {title}
        </h2>
      )}
      <div className="flex flex-col divide-y divide-border rounded-xl bg-surface shadow-card ring-1 ring-inset ring-border">
        {children}
      </div>
    </section>
  );
}

function SwitchRow({
  title,
  description,
  checked,
  disabled,
  onCheckedChange,
}: {
  title: string;
  description: string;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex min-h-[56px] items-center justify-between gap-6 px-3.5 py-2.5">
      <div className="flex min-w-0 flex-col">
        <span className={settingTitleClass}>{title}</span>
        <span className={settingDescriptionClass}>{description}</span>
      </div>
      <Switch checked={checked} disabled={disabled} onCheckedChange={onCheckedChange} />
    </label>
  );
}

function RadioCardsRow<Value extends string>({
  title,
  description,
  name,
  items,
  value,
  onValueChange,
}: {
  title: string;
  description: string;
  name: string;
  items: Array<{ id: Value; title: string; description: string }>;
  value: Value;
  onValueChange: (value: Value) => void;
}) {
  return (
    <div className="flex flex-col gap-2.5 px-3.5 py-3">
      <div className="flex flex-col">
        <span className={settingTitleClass}>{title}</span>
        <span className={settingDescriptionClass}>{description}</span>
      </div>
      <div className="grid auto-cols-fr grid-flow-col gap-2">
        {items.map((item) => (
          <label
            key={item.id}
            className={clsx(
              "relative flex flex-col gap-0.5 rounded-lg p-2.5 ring-1 ring-inset transition-[background-color,box-shadow] duration-150 has-[:focus-visible]:ring-2",
              value === item.id ? "bg-accent-soft ring-accent" : "bg-subtle ring-border hover:ring-border-strong",
            )}
          >
            <input
              type="radio"
              name={name}
              className="sr-only"
              checked={value === item.id}
              onChange={(event) => {
                if (event.target.checked && value !== item.id) {
                  onValueChange(item.id);
                }
              }}
            />
            <span className={settingTitleClass}>{item.title}</span>
            <span className="text-[11px] leading-4 text-muted-foreground">{item.description}</span>
            {value === item.id && (
              <span className="absolute right-2 top-2 flex size-4 items-center justify-center rounded-full bg-accent text-accent-foreground">
                <HiCheck className="size-2.5" strokeWidth={1.5} />
              </span>
            )}
          </label>
        ))}
      </div>
    </div>
  );
}

const THEME_OPTIONS: { id: ThemePreference; title: string }[] = [
  { id: "system", title: "Auto" },
  { id: "light", title: "Light" },
  { id: "dark", title: "Dark" },
];

const THEME_PREVIEW_COLORS = {
  light: { background: "#fbfbfc", rail: "#f0f0f3", line: "rgb(16 17 24 / 0.14)", edge: "rgb(16 17 24 / 0.09)" },
  dark: { background: "#0f1013", rail: "#0b0c0e", line: "rgb(255 255 255 / 0.16)", edge: "rgb(255 255 255 / 0.08)" },
};

const PREVIEW_AVATAR_COLORS = ["#22d3ee", "#f472b6", "#34d399"];

// A tiny Hopp window, painted with fixed colours so it shows that theme whichever one is active
function ThemePreview({ appearance }: { appearance: "light" | "dark" }) {
  const colors = THEME_PREVIEW_COLORS[appearance];
  return (
    <div className="absolute inset-0 flex" style={{ background: colors.background }}>
      <div className="h-full w-[18%] border-r" style={{ background: colors.rail, borderColor: colors.edge }} />
      <div className="flex flex-1 flex-col gap-[7px] p-2.5">
        <div className="mb-0.5 h-1.5 w-2/5 rounded-full" style={{ background: colors.line }} />
        {PREVIEW_AVATAR_COLORS.map((avatarColor) => (
          <div key={avatarColor} className="flex items-center gap-1.5">
            <div className="size-3.5 shrink-0 rounded-[4px]" style={{ background: avatarColor }} />
            <div className="h-1 w-3/5 rounded-full" style={{ background: colors.line }} />
          </div>
        ))}
      </div>
    </div>
  );
}

function AppearanceSettings() {
  const preference = useThemePreference();

  return (
    <div className="flex flex-col gap-3">
      <div className="grid grid-cols-3 gap-3" role="radiogroup" aria-label="Appearance">
        {THEME_OPTIONS.map((option) => {
          const selected = preference === option.id;
          return (
            <button
              key={option.id}
              type="button"
              role="radio"
              aria-checked={selected}
              onClick={() => setThemePreference(option.id)}
              className="group flex flex-col items-center gap-2 rounded-xl outline-none"
            >
              <span
                className={clsx(
                  "relative block aspect-[4/3] w-full overflow-hidden rounded-xl shadow-card ring-1 transition-[box-shadow,transform] duration-200 group-active:scale-[0.98] group-focus-visible:ring-2 group-focus-visible:ring-ring",
                  selected ?
                    "ring-2 ring-accent ring-offset-2 ring-offset-background"
                  : "ring-border-strong group-hover:ring-faint-foreground",
                )}
              >
                {option.id === "system" ?
                  <>
                    <ThemePreview appearance="light" />
                    <span className="absolute inset-0 block [clip-path:polygon(100%_0,100%_100%,0_100%)]">
                      <ThemePreview appearance="dark" />
                    </span>
                  </>
                : <ThemePreview appearance={option.id} />}
              </span>
              <span
                className={clsx(
                  "text-[13px] leading-[18px]",
                  selected ? "font-medium text-foreground" : "text-muted-foreground",
                )}
              >
                {option.title}
              </span>
            </button>
          );
        })}
      </div>
      <p className="text-xs leading-[18px] text-muted-foreground">
        Auto follows your system appearance. Every Hopp window switches together.
      </p>
    </div>
  );
}

const MAC_KEY_SYMBOLS: Record<string, string> = {
  CommandOrControl: "⌘",
  CmdOrCtrl: "⌘",
  Command: "⌘",
  Cmd: "⌘",
  Control: "⌃",
  Ctrl: "⌃",
  Option: "⌥",
  Alt: "⌥",
  Shift: "⇧",
};

function formatAccel(accel: string): string[] {
  const keys = accel.split("+");
  if (OS === "macos") return keys.map((key) => MAC_KEY_SYMBOLS[key] ?? key);
  return keys.map((key) => (key === "CommandOrControl" || key === "CmdOrCtrl" ? "Ctrl" : key));
}

function acceleratorKey(event: KeyboardEvent): string {
  // With Option held, event.key is the composed character (◊ for V), which isn't a valid accelerator.
  if (event.code.startsWith("Key")) return event.code.slice("Key".length);
  if (event.code.startsWith("Digit")) return event.code.slice("Digit".length);
  return event.key.toUpperCase();
}

function ShortcutRow({
  title,
  description,
  value,
  onCommit,
}: {
  title: string;
  description: string;
  value: string;
  onCommit: (accel: string) => void;
}) {
  const [recording, setRecording] = useState(false);
  const listenerRef = useRef<((e: KeyboardEvent) => void) | null>(null);

  const startRecording = () => {
    setRecording(true);
    const handler = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();

      if (e.key === "Escape" && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
        stopRecording();
        return;
      }

      if (["Meta", "Control", "Alt", "Shift"].includes(e.key)) return;

      const hasModifier = e.metaKey || e.ctrlKey || e.altKey;
      if (!hasModifier) return;

      const parts: string[] = [];
      if (OS === "macos") {
        if (e.metaKey) parts.push("Cmd");
        if (e.ctrlKey) parts.push("Ctrl");
        if (e.altKey) parts.push("Alt");
        if (e.shiftKey) parts.push("Shift");
      } else {
        if (e.ctrlKey) parts.push("Ctrl");
        if (e.altKey) parts.push("Alt");
        if (e.shiftKey) parts.push("Shift");
      }
      parts.push(acceleratorKey(e));
      const accel = parts.join("+");

      stopRecording();
      onCommit(accel);
    };

    listenerRef.current = handler;
    window.addEventListener("keydown", handler, true);
  };

  const stopRecording = () => {
    setRecording(false);
    if (listenerRef.current) {
      window.removeEventListener("keydown", listenerRef.current, true);
      listenerRef.current = null;
    }
  };

  return (
    <div className="flex min-h-[56px] items-center justify-between gap-4 px-3.5 py-2.5">
      <div className="flex min-w-0 flex-col">
        <span className={settingTitleClass}>{title}</span>
        <span className={settingDescriptionClass}>{description}</span>
      </div>
      <div className="flex shrink-0 items-center gap-1">
        {value && !recording && (
          <Button variant="ghost" size="sm" aria-label={`Clear ${title} shortcut`} onClick={() => onCommit("")}>
            Clear
          </Button>
        )}
        <button
          type="button"
          aria-label={`Record ${title} shortcut`}
          onClick={() => (recording ? stopRecording() : startRecording())}
          className={clsx(
            "flex h-8 w-[136px] items-center justify-center gap-0.5 rounded-lg px-2 text-xs ring-1 ring-inset transition-[background-color,box-shadow] duration-150 outline-none focus-visible:ring-2 focus-visible:ring-ring",
            recording ?
              "bg-accent-soft font-medium text-accent ring-accent"
            : "bg-subtle text-faint-foreground ring-border hover:ring-border-strong",
          )}
        >
          {recording ?
            "Press keys…"
          : value ?
            formatAccel(value).map((key) => <kbd key={key}>{key}</kbd>)
          : "Not set"}
        </button>
      </div>
    </div>
  );
}

function applicationIcon(application: InstalledApplication): string | undefined {
  if (!application.icon_png) return undefined;
  let binary = "";
  for (const byte of application.icon_png) binary += String.fromCharCode(byte);
  return `data:image/png;base64,${btoa(binary)}`;
}

function AppVeilSettings({
  rows,
  installedApplications,
  onChange,
}: {
  rows: AppVeilApplication[];
  installedApplications: InstalledApplication[];
  onChange: (rows: AppVeilApplication[]) => Promise<void>;
}) {
  const [query, setQuery] = useState("");
  const [highlighted, setHighlighted] = useState(0);
  const [saving, setSaving] = useState(false);
  const addedBundleIds = new Set(rows.map((row) => row.bundle_id));
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const results =
    normalizedQuery ?
      installedApplications
        .filter((application) => application.name.toLocaleLowerCase().includes(normalizedQuery))
        .slice(0, 8)
    : [];
  const nameCounts = installedApplications.reduce<Record<string, number>>((counts, application) => {
    counts[application.name] = (counts[application.name] ?? 0) + 1;
    return counts;
  }, {});
  const commit = async (nextRows: AppVeilApplication[]) => {
    if (saving) return;
    setSaving(true);
    try {
      await onChange(nextRows);
    } catch (error) {
      console.error("Failed to update App Veil settings", error);
    } finally {
      setSaving(false);
    }
  };

  const add = async (application: InstalledApplication) => {
    if (addedBundleIds.has(application.bundle_id)) return;
    await commit([...rows, { bundle_id: application.bundle_id, enabled: true }]);
    setQuery("");
    setHighlighted(0);
  };

  return (
    <div className="flex flex-col gap-3">
      <p className="text-[13px] leading-5 text-muted-foreground">
        Hide selected applications from viewers when sharing a screen. You will still see and use them normally.
      </p>

      <div className="relative">
        <HiMagnifyingGlass className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-faint-foreground" />
        <Input
          aria-label="Add application to App Veil"
          role="combobox"
          aria-expanded={results.length > 0}
          aria-controls="app-veil-results"
          aria-activedescendant={results[highlighted] ? `app-veil-result-${highlighted}` : undefined}
          placeholder="Add application…"
          className="pl-8"
          value={query}
          disabled={saving}
          onChange={(event) => {
            setQuery(event.target.value);
            setHighlighted(0);
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              setQuery("");
              setHighlighted(0);
            } else if (event.key === "ArrowDown" && results.length) {
              event.preventDefault();
              setHighlighted((index) => (index + 1) % results.length);
            } else if (event.key === "ArrowUp" && results.length) {
              event.preventDefault();
              setHighlighted((index) => (index + results.length - 1) % results.length);
            } else if (event.key === "Enter" && results[highlighted]) {
              event.preventDefault();
              const selected = results[highlighted];
              if (selected) void add(selected);
            }
          }}
        />
        {normalizedQuery && (
          <div
            id="app-veil-results"
            role="listbox"
            className="absolute z-20 mt-1.5 max-h-56 w-full overflow-y-auto rounded-xl border border-border-strong bg-raised p-1 shadow-pop"
          >
            {results.length === 0 ?
              <div className="px-2 py-3 text-[13px] text-muted-foreground">No installed applications found.</div>
            : results.map((application, index) => {
                const added = addedBundleIds.has(application.bundle_id);
                const icon = applicationIcon(application);
                return (
                  <button
                    id={`app-veil-result-${index}`}
                    role="option"
                    aria-selected={index === highlighted}
                    key={application.bundle_id}
                    disabled={added || saving}
                    className={clsx(
                      "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[13px] disabled:opacity-60",
                      index === highlighted && "bg-subtle",
                    )}
                    onMouseEnter={() => setHighlighted(index)}
                    onClick={() => void add(application)}
                  >
                    {icon && <img src={icon} alt="" className="size-7 shrink-0" />}
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-foreground">{application.name}</span>
                      {(nameCounts[application.name] ?? 0) > 1 && (
                        <span className="block truncate text-xs text-muted-foreground">{application.bundle_id}</span>
                      )}
                    </span>
                    {added && <span className="text-xs text-muted-foreground">Added</span>}
                  </button>
                );
              })
            }
          </div>
        )}
      </div>

      <SettingsGroup>
        {rows.length === 0 ?
          <div className="flex flex-col items-center gap-1 px-4 py-9 text-center">
            <HiOutlineEyeSlash className="mb-1 size-5 text-faint-foreground" />
            <span className={settingTitleClass}>Nothing is hidden yet</span>
            <span className={settingDescriptionClass}>Search above to add an app, like your password manager.</span>
          </div>
        : rows.map((row) => {
            const application = installedApplications.find((candidate) => candidate.bundle_id === row.bundle_id);
            const name = application?.name ?? "Application not found";
            const icon = application && applicationIcon(application);
            return (
              <div key={row.bundle_id} className="group flex min-h-12 items-center gap-2.5 px-3.5 py-2">
                {icon ?
                  <img src={icon} alt="" className="size-7 shrink-0" />
                : <div aria-hidden className="size-7 shrink-0 rounded-lg bg-subtle" />}
                <span className="min-w-0 flex-1">
                  <span className={clsx("block truncate", settingTitleClass)}>{name}</span>
                  {!application && (
                    <span className="block truncate text-xs text-muted-foreground">{row.bundle_id}</span>
                  )}
                </span>
                <button
                  type="button"
                  aria-label={`Remove ${application?.name ?? row.bundle_id} from App Veil`}
                  disabled={saving}
                  className="flex size-7 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-opacity outline-none group-hover:opacity-100 hover:bg-subtle focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() => void commit(rows.filter((candidate) => candidate.bundle_id !== row.bundle_id))}
                >
                  <HiXMark className="size-4" />
                </button>
                <Switch
                  aria-label={`Hide ${application?.name ?? row.bundle_id} from viewers`}
                  checked={row.enabled}
                  disabled={!application || saving}
                  onCheckedChange={(enabled) =>
                    void commit(
                      rows.map((candidate) =>
                        candidate.bundle_id === row.bundle_id ? { ...candidate, enabled } : candidate,
                      ),
                    )
                  }
                />
              </div>
            );
          })
        }
      </SettingsGroup>
    </div>
  );
}

const formatBytes = (bytes: number) =>
  bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`;

function CaptureModelRow({
  model,
  chosen,
  onChoose,
  onDownload,
  onDelete,
}: {
  model: CaptureModel;
  chosen: boolean;
  onChoose: () => void;
  onDownload: () => void;
  onDelete: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const size = formatBytes(model.bytes);
  return (
    <div
      className={clsx(
        "flex items-center gap-3 px-3.5 py-2 first:rounded-t-xl last:rounded-b-xl",
        chosen && "bg-accent-soft",
      )}
    >
      <label className="flex min-w-0 flex-1 items-center gap-3">
        <input
          type="radio"
          name="capture-model"
          className="size-3.5 shrink-0 accent-[var(--color-accent)]"
          checked={chosen}
          onChange={(event) => {
            if (event.target.checked) onChoose();
          }}
        />
        <span className="flex min-w-0 flex-col">
          <span className={settingTitleClass}>
            {model.name} <span className="font-normal text-muted-foreground">{size}</span>
          </span>
          <span className={clsx(settingDescriptionClass, "first-letter:uppercase")}>{model.note}</span>
        </span>
      </label>
      {model.downloading !== null ?
        <span className="shrink-0 text-xs tabular-nums text-muted-foreground">{model.downloading}%</span>
      : model.shared ?
        <span className="shrink-0 text-xs text-muted-foreground">From Hex</span>
      : chosen && model.installed ?
        null
      : model.installed ?
        <Button
          variant={confirming ? "destructive-transparent" : "outline"}
          size="sm"
          onBlur={() => setConfirming(false)}
          onMouseLeave={() => setConfirming(false)}
          onClick={() => {
            if (confirming) {
              setConfirming(false);
              onDelete();
            } else {
              setConfirming(true);
            }
          }}
        >
          {confirming ? `Delete ${size}?` : "Delete"}
        </Button>
      : <Button variant="outline" size="sm" onClick={onDownload}>
          Download
        </Button>
      }
    </div>
  );
}

function CaptureRow({ title, description, children }: { title: string; description: string; children: ReactNode }) {
  return (
    <div className="flex min-h-[56px] items-center justify-between gap-6 px-3.5 py-2.5">
      <div className="flex min-w-0 flex-col">
        <span className={settingTitleClass}>{title}</span>
        <span className={settingDescriptionClass}>{description}</span>
      </div>
      <div className="flex shrink-0 gap-2">{children}</div>
    </div>
  );
}

function CopyButton({ label, text }: { label: string; text: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);
  return (
    <Button variant="outline" size="sm" onClick={() => writeText(text).then(() => setCopied(true))}>
      {copied ? "Copied" : label}
    </Button>
  );
}

/** A path as it needs to be typed in a shell. */
const shellQuoted = (path: string) => (/^[\w./-]+$/.test(path) ? path : `'${path.replace(/'/g, `'\\''`)}'`);

function CaptureSettingsSection() {
  const [failure, setFailure] = useState("");
  const { data: capture, refetch } = useQuery({
    queryKey: ["capture-settings"],
    queryFn: () => typedInvoke("capture_settings"),
    refetchOnWindowFocus: true,
    refetchInterval: (query) => (query.state.data?.models.some((model) => model.downloading !== null) ? 1000 : false),
  });

  if (!capture) return null;

  const run = (work: Promise<unknown>) => {
    setFailure("");
    work.catch((error) => setFailure(String(error))).finally(() => refetch());
  };
  const change = (next: CaptureChange) => run(typedInvoke("capture_settings_change", { change: next }));
  const failedDownload = capture.models.find((model) => model.failed !== null);
  const liveTextPossible = capture.gpu && capture.model.startsWith("parakeet");
  const cli = shellQuoted(capture.cli_path);

  return (
    <>
      <p className="text-xs leading-[18px] text-muted-foreground">
        Capture transcribes a call on this computer and keeps it here. Nothing is uploaded.
      </p>

      <SettingsGroup>
        <SwitchRow
          title="Capture every call"
          description="Start transcribing as each call connects. Let people know you do."
          checked={capture.auto}
          onCheckedChange={(auto) => change({ auto })}
        />
      </SettingsGroup>

      <SettingsGroup title="Transcription model">
        {capture.models.map((model) => (
          <CaptureModelRow
            key={model.name}
            model={model}
            chosen={model.name === capture.model}
            onChoose={() => change({ model: model.name })}
            onDownload={() => run(typedInvoke("capture_model_download", { name: model.name }))}
            onDelete={() => run(typedInvoke("capture_model_delete", { name: model.name }))}
          />
        ))}
      </SettingsGroup>
      {failedDownload && (
        <p role="alert" className="text-xs leading-[18px] text-destructive">
          Download failed: {failedDownload.name}: {failedDownload.failed}
        </p>
      )}

      <SettingsGroup title="Transcription">
        <RadioCardsRow
          title="Runs on"
          description="Applies from the next capture"
          name="capture-device"
          items={[
            { id: "gpu", title: "GPU", description: "Faster, and uses less energy" },
            { id: "cpu", title: "CPU", description: "Slower, and only uses spare time" },
          ]}
          value={capture.gpu ? "gpu" : "cpu"}
          onValueChange={(device) => change({ gpu: device === "gpu" })}
        />
        <SwitchRow
          title="Live text"
          description="Show words about two seconds after they're said. The final text is the same either way. Needs a Parakeet model on the GPU."
          checked={capture.drafts && liveTextPossible}
          disabled={!liveTextPossible}
          onCheckedChange={(drafts) => change({ drafts })}
        />
      </SettingsGroup>

      <SettingsGroup title="Storage">
        <CaptureRow
          title="Storage used"
          description={`Captures ${formatBytes(capture.captures_bytes)}, models ${formatBytes(capture.models_bytes)}`}
        >
          <Button variant="outline" size="sm" onClick={() => run(tauriUtils.openCapturesWindow())}>
            Open captures
          </Button>
          <Button variant="outline" size="sm" onClick={() => run(typedInvoke("capture_open_folder"))}>
            Show folder
          </Button>
        </CaptureRow>
        <RadioCardsRow
          title="Delete captures after"
          description="Older captures go as soon as you choose, with their pictures and Markdown"
          name="capture-keep"
          items={[
            { id: "never", title: "Never", description: "Keep every capture" },
            { id: "90", title: "90 days", description: "About three months" },
            { id: "30", title: "30 days", description: "About a month" },
          ]}
          value={capture.keep_days === null ? "never" : (String(capture.keep_days) as "90" | "30" | "never")}
          onValueChange={(keep) => change({ keep_days: keep === "never" ? null : Number(keep) })}
        />
      </SettingsGroup>

      <SettingsGroup title="Coding agents">
        <CaptureRow title="Agents" description="Lets agents read, search and follow captures (MCP). Copy the setup.">
          <CopyButton label="Claude Code" text={`claude mcp add --scope user hopp -- ${cli} mcp`} />
          <CopyButton label="Codex" text={`codex mcp add hopp -- ${cli} mcp`} />
        </CaptureRow>
        <CaptureRow title="Command line" description="Use hopp in your terminal. Copy this, then run it in Terminal.">
          <CopyButton label="Copy" text={`sudo mkdir -p /usr/local/bin && sudo ln -sf ${cli} /usr/local/bin/hopp`} />
        </CaptureRow>
        <CaptureRow
          title="Hooks"
          description="Executable files named capture-started or capture-complete, given the call's details as JSON."
        >
          <Button variant="outline" size="sm" onClick={() => run(typedInvoke("capture_open_hooks"))}>
            Open folder
          </Button>
        </CaptureRow>
      </SettingsGroup>
      {failure && (
        <p role="alert" className="text-xs leading-[18px] text-destructive">
          {failure}
        </p>
      )}
    </>
  );
}

type SectionId = "appearance" | "call" | "capture" | "app-veil" | "shortcuts" | "misc";

const SECTIONS: { id: SectionId; title: string; icon: ReactNode; beta?: boolean; macOnly?: boolean }[] = [
  { id: "appearance", title: "Appearance", icon: <HiOutlineSwatch /> },
  { id: "call", title: "Calls", icon: <HiOutlinePhone /> },
  { id: "capture", title: "Capture", icon: <HiOutlineChatBubbleBottomCenterText />, macOnly: true },
  { id: "app-veil", title: "App Veil", icon: <HiOutlineEyeSlash />, beta: true, macOnly: true },
  { id: "shortcuts", title: "Shortcuts", icon: <LuCommand /> },
  { id: "misc", title: "General", icon: <HiOutlineAdjustmentsHorizontal /> },
];

function SettingsWindow() {
  useDisableNativeContextMenu();

  const [serverUrl, setServerUrl] = useState("");
  const [section, setSection] = useState<SectionId>("appearance");
  const [savingDefaultMic, setSavingDefaultMic] = useState(false);
  const [defaultMicError, setDefaultMicError] = useState("");

  const { data: settings, refetch: refetchSettings } = useQuery({
    queryKey: ["user-settings"],
    queryFn: () => typedInvoke("get_user_settings"),
    select: (data) => data,
    refetchOnWindowFocus: true,
  });

  const { data: installedApplications = [] } = useQuery({
    queryKey: ["installed-applications"],
    queryFn: () => typedInvoke("list_installed_applications"),
    enabled: OS === "macos",
    staleTime: Infinity,
  });

  const { data: microphones = [], refetch: refetchMicrophones } = useQuery({
    queryKey: ["list_microphones"],
    queryFn: () => typedInvoke("list_microphones"),
    enabled: section === "call",
    select: (devices) => devices.filter((device) => device.name !== "").sort((a, b) => a.name.localeCompare(b.name)),
  });

  useEffect(() => {
    if (settings) {
      setServerUrl(settings.hopp_server_url ?? "");
    }
  }, [settings]);

  async function commitShortcut(which: "mic" | "camera" | "screenshare" | "end_call", accel: string) {
    const setters = {
      mic: "set_shortcut_toggle_mic",
      camera: "set_shortcut_toggle_camera",
      screenshare: "set_shortcut_toggle_screenshare",
      end_call: "set_shortcut_end_call",
    } as const;

    if (settings) {
      const others = (["mic", "camera", "screenshare", "end_call"] as const).filter((k) => k !== which);
      for (const other of others) {
        const otherVal =
          other === "mic" ? settings.shortcut_toggle_mic
          : other === "camera" ? settings.shortcut_toggle_camera
          : other === "screenshare" ? settings.shortcut_toggle_screenshare
          : settings.shortcut_end_call;
        if (accel && otherVal === accel) {
          await typedInvoke(setters[other], { accel: "" });
        }
      }
    }

    await typedInvoke(setters[which], { accel });
    refetchSettings();
  }

  if (!settings) return <div className="h-full min-h-screen bg-background" />;

  const visibleSections = SECTIONS.filter((s) => !s.macOnly || OS === "macos");

  return (
    <div className="flex h-screen overflow-hidden bg-background text-foreground">
      <div data-tauri-drag-region className="fixed inset-x-0 top-0 z-50 h-[32px]" />

      <nav className="flex w-[188px] shrink-0 flex-col gap-0.5 border-r border-border bg-rail px-2.5 pb-3 pt-12">
        {visibleSections.map((s) => (
          <button
            key={s.id}
            onClick={() => setSection(s.id)}
            className={clsx(
              "flex h-8 items-center gap-2 rounded-lg px-2 text-left text-[13px] transition-colors duration-150 [&>svg]:size-4 [&>svg]:shrink-0",
              section === s.id ?
                "bg-subtle-strong font-medium text-foreground"
              : "text-muted-foreground hover:bg-subtle hover:text-foreground",
            )}
          >
            {s.icon}
            {s.title}
            {s.beta && (
              <span className="ml-auto rounded-[5px] bg-accent-soft px-1.5 text-[10px] leading-4 font-medium text-accent">
                Beta
              </span>
            )}
          </button>
        ))}
      </nav>

      <main key={section} className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-[580px] flex-col gap-5 px-8 pb-10 pt-12 animate-in fade-in-0 duration-200">
          <h1 className="text-[22px] leading-7 font-semibold tracking-[-0.02em] text-foreground">
            {visibleSections.find((s) => s.id === section)?.title}
          </h1>

          {section === "appearance" && <AppearanceSettings />}

          {section === "call" && (
            <>
              <SettingsGroup>
                <SwitchRow
                  title="Call feedback popup"
                  description="Ask how the call went when it ends"
                  checked={settings.call_feedback_popup}
                  onCheckedChange={(v) => {
                    typedInvoke("set_call_feedback_popup", { enabled: v }).then(() => refetchSettings());
                  }}
                />
                <SwitchRow
                  title="Show dock icon when in call"
                  description="Turn off to save dock space during calls"
                  checked={settings.show_dock_icon_in_call}
                  onCheckedChange={(v) => {
                    typedInvoke("set_show_dock_icon_in_call", { enabled: v }).then(() => refetchSettings());
                  }}
                />
              </SettingsGroup>

              <SettingsGroup title="Joining a call">
                <SwitchRow
                  title="Start with camera on"
                  description="Open your camera when you start a call"
                  checked={settings.start_camera_on_call}
                  onCheckedChange={(v) => {
                    typedInvoke("set_start_camera_on_call", { enabled: v }).then(() => refetchSettings());
                  }}
                />
                <SwitchRow
                  title="Start with microphone on"
                  description="Unmute your microphone when you start a call"
                  checked={settings.start_mic_on_call}
                  onCheckedChange={(v) => {
                    typedInvoke("set_start_mic_on_call", { enabled: v }).then(() => refetchSettings());
                  }}
                />
              </SettingsGroup>

              <SettingsGroup title="Audio">
                <div className="flex min-h-[56px] items-center justify-between gap-6 px-3.5 py-2.5">
                  <div className="flex min-w-0 flex-col">
                    <label htmlFor="default-microphone" className={settingTitleClass}>
                      Default microphone
                    </label>
                    <span id="default-microphone-description" className={settingDescriptionClass}>
                      Used at the start of each call when available. Changes apply to your next call.
                    </span>
                    {defaultMicError && (
                      <span role="alert" className="text-xs text-red-600 dark:text-red-400">
                        {defaultMicError}
                      </span>
                    )}
                  </div>
                  <Select
                    value={settings.default_mic == null ? "last-used" : `mic:${settings.default_mic}`}
                    disabled={savingDefaultMic}
                    onOpenChange={(open) => {
                      if (open) refetchMicrophones();
                    }}
                    onValueChange={async (value) => {
                      setSavingDefaultMic(true);
                      setDefaultMicError("");
                      try {
                        await typedInvoke("set_default_mic", {
                          deviceName: value === "last-used" ? null : value.slice(4),
                        });
                        await refetchSettings();
                      } catch (error) {
                        setDefaultMicError(String(error));
                      } finally {
                        setSavingDefaultMic(false);
                      }
                    }}
                  >
                    <SelectTrigger
                      id="default-microphone"
                      aria-describedby="default-microphone-description"
                      className="w-[220px] shrink-0"
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="last-used">Use last-used microphone</SelectItem>
                      {settings.default_mic && !microphones.some((device) => device.name === settings.default_mic) && (
                        <SelectItem value={`mic:${settings.default_mic}`} disabled>
                          {settings.default_mic} (Unavailable)
                        </SelectItem>
                      )}
                      {microphones.map((device) => (
                        <SelectItem key={device.name} value={`mic:${device.name}`}>
                          {device.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <SwitchRow
                  title="Noise cancellation"
                  description="Suppress background noise on your microphone"
                  checked={settings.noise_cancellation_enabled}
                  onCheckedChange={(v) => {
                    typedInvoke("set_noise_cancellation", { enabled: v }).then(() => refetchSettings());
                  }}
                />
              </SettingsGroup>

              <SettingsGroup title="Screen share">
                <SwitchRow
                  title="Enable remote control by default"
                  description="Allow teammates to control your computer when screen sharing starts"
                  checked={settings.remote_control_enabled}
                  onCheckedChange={(v) => {
                    typedInvoke("set_remote_control_enabled", { enabled: v }).then(() => refetchSettings());
                  }}
                />
                <RadioCardsRow
                  title="Resolution"
                  description="Choose stream quality for screen sharing"
                  name="screen-share-resolution"
                  items={screenShareResolutionItems}
                  value={settings.screen_share_resolution}
                  onValueChange={(resolution) => {
                    typedInvoke("set_screen_share_resolution", { resolution }).then(() => refetchSettings());
                  }}
                />
                {OS === "macos" && (
                  <RadioCardsRow
                    title="Default picker mode"
                    description="Choose what to select when screen sharing starts"
                    name="screen-share-picker-mode"
                    items={screenSharePickerModeItems}
                    value={settings.screen_share_picker_mode}
                    onValueChange={(mode) => {
                      typedInvoke("set_screen_share_picker_mode", { mode }).then(() => refetchSettings());
                    }}
                  />
                )}
              </SettingsGroup>
            </>
          )}

          {section === "capture" && OS === "macos" && <CaptureSettingsSection />}

          {section === "app-veil" && OS === "macos" && (
            <AppVeilSettings
              rows={settings.app_veil_applications}
              installedApplications={installedApplications}
              onChange={async (applications) => {
                try {
                  await typedInvoke("set_app_veil_applications", { applications });
                } finally {
                  await refetchSettings();
                }
              }}
            />
          )}

          {section === "shortcuts" && (
            <>
              <SettingsGroup>
                <ShortcutRow
                  title="Mute / unmute mic"
                  description="Toggle microphone during call"
                  value={settings.shortcut_toggle_mic}
                  onCommit={(accel) => commitShortcut("mic", accel)}
                />
                <ShortcutRow
                  title="Toggle camera"
                  description="Turn camera on or off during call"
                  value={settings.shortcut_toggle_camera}
                  onCommit={(accel) => commitShortcut("camera", accel)}
                />
                <ShortcutRow
                  title="Toggle screen share"
                  description="Start or stop screen sharing"
                  value={settings.shortcut_toggle_screenshare}
                  onCommit={(accel) => commitShortcut("screenshare", accel)}
                />
                <ShortcutRow
                  title="End call"
                  description="Leave the current call"
                  value={settings.shortcut_end_call}
                  onCommit={(accel) => commitShortcut("end_call", accel)}
                />
              </SettingsGroup>
              <p className="text-xs leading-[18px] text-muted-foreground">
                Click a shortcut, then press the keys. It needs {OS === "macos" ? "⌘, ⌃ or ⌥" : "Ctrl or Alt"}. Esc
                cancels.
              </p>
            </>
          )}

          {section === "misc" && (
            <>
              <SettingsGroup>
                <SwitchRow
                  title="Send anonymous telemetry"
                  description="Help improve Hopp by sending anonymous usage data and error reports"
                  checked={settings.telemetry_enabled}
                  onCheckedChange={(v) => {
                    typedInvoke("set_telemetry_enabled", { enabled: v }).then(() => {
                      refetchSettings();
                      if (v) {
                        posthog.opt_in_capturing();
                      } else {
                        posthog.opt_out_capturing();
                      }
                    });
                  }}
                />
                {OS === "macos" && (
                  <SwitchRow
                    title="Automatic updates"
                    description="Download and install updates automatically when you're not in a call"
                    checked={settings.auto_update_enabled}
                    onCheckedChange={(v) => {
                      typedInvoke("set_auto_update_enabled", { enabled: v }).then(() => refetchSettings());
                    }}
                  />
                )}
              </SettingsGroup>

              <SettingsGroup title="Server">
                <div className="flex flex-col gap-2.5 px-3.5 py-3">
                  <label htmlFor="custom-backend-url" className="flex flex-col">
                    <span className={settingTitleClass}>Custom backend URL</span>
                    <span className={settingDescriptionClass}>Change backend server. Leave empty to use default.</span>
                  </label>
                  <Input
                    id="custom-backend-url"
                    type="text"
                    placeholder={URLS.API_BASE_URL}
                    value={serverUrl}
                    onChange={(e) => setServerUrl(e.target.value)}
                    onKeyDown={async (e) => {
                      if (e.key === "Enter") {
                        const trimmed = serverUrl.trim() || null;
                        await tauriUtils.setHoppServerUrl(trimmed);
                        useStore.getState().setCustomServerUrl(trimmed);
                        posthog.capture("custom_backend_url_changed");
                        refetchSettings();
                      }
                    }}
                    onBlur={async () => {
                      const trimmed = serverUrl.trim() || null;
                      if (trimmed !== settings.hopp_server_url) {
                        await tauriUtils.setHoppServerUrl(trimmed);
                        useStore.getState().setCustomServerUrl(trimmed);
                        posthog.capture("custom_backend_url_changed");
                        refetchSettings();
                      }
                    }}
                  />
                </div>
              </SettingsGroup>
            </>
          )}
        </div>
      </main>
    </div>
  );
}
