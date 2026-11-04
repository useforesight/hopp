import "../../App.css";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import clsx from "clsx";
import {
  HiCheck,
  HiMagnifyingGlass,
  HiOutlineBars3,
  HiOutlineBookmark,
  HiOutlineClipboard,
  HiOutlineDocumentText,
  HiOutlineMicrophone,
  HiOutlineStopCircle,
  HiOutlineTrash,
  HiXMark,
} from "react-icons/hi2";
import { listen } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { initTheme } from "@/lib/theme";
import {
  typedInvoke,
  type CaptureCall,
  type CaptureListed,
  type CaptureOverview,
  type CapturePicture,
  type CaptureRow,
  type CaptureUpdate,
} from "@/core_payloads";
import { Feed, type FeedHandle, type FeedItem } from "./feed";
import { agentPrompt, clock, duration, heading, longDate, plural, tickStep, when } from "./format";
import { CallContext, estimateRow, Faces, Row } from "./timeline";
import { IconButton, Viewer } from "./viewer";

initTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <CapturesWindow />
  </React.StrictMode>,
);

/** Below this width the call list hides and the timeline fills the window. */
const NARROW = 760;
const POLL_MS = 600;
const LIST_POLL_MS = 5_000;
const SEARCH_WAITS_MS = 200;
const COPIED_MS = 2_000;
const DELETE_ARMED_MS = 4_000;

type Filter = "everything" | "said" | "drawing" | "screen";

interface Reading {
  call: CaptureCall;
  rows: CaptureRow[];
  overview: CaptureOverview;
  colours: Record<string, string>;
  pictures: CapturePicture[];
}

const sameColours = (a: Record<string, string>, b: Record<string, string>) => {
  const keys = Object.keys(a);
  return keys.length === Object.keys(b).length && keys.every((key) => a[key] === b[key]);
};

const applied = (reading: Reading | null, update: CaptureUpdate): Reading => {
  const same = reading?.call.id === update.call.id ? reading : null;
  return {
    call: update.call,
    rows: [...(same?.rows.slice(0, update.from) ?? []), ...update.rows],
    overview: update.overview,
    colours: same && sameColours(same.colours, update.colours) ? same.colours : update.colours,
    pictures: update.pictures ?? same?.pictures ?? [],
  };
};

/** The call being read, kept current. Opening and polling take turns, so an update always
 * applies to the rows it was worked out from. */
const useReading = (onGone: () => void) => {
  const [reading, setReading] = useState<Reading | null>(null);
  const turn = useRef<Promise<unknown>>(Promise.resolve());
  const onGoneRef = useRef(onGone);
  onGoneRef.current = onGone;

  const inTurn = useCallback(<T,>(job: () => Promise<T>): Promise<T> => {
    const done = turn.current.then(job, job);
    turn.current = done.catch(() => undefined);
    return done;
  }, []);

  const open = useCallback(
    (which: string) =>
      inTurn(async () => {
        const update = await typedInvoke("captures_open", { which });
        setReading(applied(null, update));
      }),
    [inTurn],
  );

  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      if (!document.hidden) {
        try {
          const update = await inTurn(() => typedInvoke("captures_poll"));
          if (update && !stopped) {
            setReading((current) => (current?.call.id === update.call.id ? applied(current, update) : current));
          }
        } catch (error) {
          // The call has been deleted. Anything else may pass, so keep following it.
          if (error === "gone") {
            if (!stopped) onGoneRef.current();
          } else {
            console.error("Following a capture:", error);
          }
        }
      }
      if (!stopped) timer = setTimeout(poll, POLL_MS);
    };
    timer = setTimeout(poll, POLL_MS);
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [inTurn]);

  return { reading, open, close: useCallback(() => setReading(null), []) };
};

const useNarrow = () => {
  const [narrow, setNarrow] = useState(() => window.innerWidth < NARROW);
  useEffect(() => {
    const query = window.matchMedia(`(max-width: ${NARROW - 1}px)`);
    const onChange = () => setNarrow(query.matches);
    onChange();
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return narrow;
};

/** The time now, moving once a second while `moving`. */
const useNow = (moving: boolean) => {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!moving) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(timer);
  }, [moving]);
  return now;
};

const SearchInput = ({
  value,
  onChange,
  placeholder,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
}) => (
  <label className="flex h-[30px] items-center gap-1.5 rounded-lg border border-border bg-surface px-2 text-muted-foreground focus-within:border-border-strong">
    <HiMagnifyingGlass className="size-3.5 shrink-0" />
    <input
      value={value}
      onChange={(event) => onChange(event.target.value)}
      placeholder={placeholder}
      spellCheck={false}
      className="min-w-0 flex-1 bg-transparent text-[12.5px] text-foreground outline-none placeholder:text-faint-foreground"
    />
    {value && (
      <button aria-label="Clear" onClick={() => onChange("")} className="shrink-0 hover:text-foreground">
        <HiXMark className="size-3.5" />
      </button>
    )}
  </label>
);

/** Search hits carry their matches in [brackets]. */
const Found = ({ text }: { text: string }) => (
  <>
    {text.split(/(\[[^\]]*\])/).map((part, index) =>
      part.startsWith("[") && part.endsWith("]") ?
        <mark key={index} className="rounded-[3px] bg-warning-soft text-inherit">
          {part.slice(1, -1)}
        </mark>
      : part,
    )}
  </>
);

const CallList = ({
  calls,
  selected,
  colours,
  searching,
  onSelect,
}: {
  calls: CaptureListed[];
  selected?: string;
  colours: Record<string, string>;
  searching: boolean;
  onSelect: (id: string) => void;
}) => (
  <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2 pb-3">
    {calls.length === 0 && (
      <p className="px-2 text-xs leading-[18px] text-muted-foreground">
        {searching ? "No call mentions that." : "Nothing captured yet."}
      </p>
    )}
    {calls.map((call) => {
      const note = call.found ?? call.summary;
      const chosen = call.id === selected;
      return (
        <button
          key={call.id}
          onClick={() => onSelect(call.id)}
          className={clsx(
            "flex flex-col rounded-[9px] px-2.5 py-2 text-left transition-colors",
            chosen ? "bg-subtle-strong" : "hover:bg-subtle",
          )}
        >
          <span className="flex items-center gap-1.5">
            {call.ended_ms === undefined && <HiOutlineMicrophone className="size-3 shrink-0 text-destructive" />}
            <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-foreground">{heading(call)}</span>
          </span>
          <span className="text-[11px] text-muted-foreground">
            {when(call.started_ms)} ·{" "}
            {call.ended_ms === undefined ? "capturing" : duration(call.ended_ms - call.started_ms)}
          </span>
          {note && (
            <span className="mt-[3px] line-clamp-2 text-[11.5px] leading-[15.5px] text-muted-foreground">
              {call.found ?
                <Found text={note} />
              : note}
            </span>
          )}
          {call.participants.length > 0 && (
            <span className="mt-1.5">
              <Faces
                names={call.participants.slice(0, 5)}
                size={18}
                colours={chosen ? colours : {}}
                ring={chosen ? "transparent" : "var(--rail)"}
              />
            </span>
          )}
        </button>
      );
    })}
  </div>
);

const Action = ({
  icon,
  label,
  compact,
  tone,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  compact: boolean;
  tone?: "danger" | "armed";
  onClick: () => void;
}) => (
  <button
    title={label}
    aria-label={label}
    onClick={onClick}
    className={clsx(
      "flex h-[30px] shrink-0 items-center gap-1.5 rounded-lg text-xs font-medium transition-colors [&>svg]:size-4",
      compact ? "w-[30px] justify-center" : "px-2",
      tone === "armed" && "bg-destructive-soft text-destructive",
      tone === "danger" && "text-destructive hover:bg-destructive-soft",
      !tone && "text-muted-foreground hover:bg-subtle hover:text-foreground",
    )}
  >
    {icon}
    {!compact && <span>{label}</span>}
  </button>
);

/** Who spoke when, drawings and screen changes across the whole call; click to jump. */
const Scrubber = ({
  overview,
  total,
  colours,
  onJump,
}: {
  overview: CaptureOverview;
  total: number;
  colours: Record<string, string>;
  onJump: (t_ms: number) => void;
}) => {
  const people = useMemo(() => [...new Set(overview.speech.map(([who]) => who))], [overview.speech]);
  const laneHeight = Math.min(Math.max(24 / Math.max(people.length, 1), 4), 8);
  const at = (t: number) => `${Math.min(Math.max(t / total, 0), 1) * 100}%`;
  const across = (from: number, to: number, least: number) =>
    `${Math.min(Math.max((to - from) / total, least), 1) * 100}%`;
  const step = tickStep(total);
  const ticks: number[] = [];
  for (let t = 0; t < total - step / 3; t += step) ticks.push(t);
  ticks.push(total);
  return (
    <div className="border-b border-border px-[18px] pt-2.5 pb-2">
      <div
        role="slider"
        aria-label="Jump through the call"
        aria-valuemin={0}
        aria-valuemax={total}
        aria-valuenow={0}
        tabIndex={-1}
        onMouseDown={(event) => {
          const area = event.currentTarget.getBoundingClientRect();
          onJump(((event.clientX - area.left) / Math.max(area.width, 1)) * total);
        }}
        className="relative h-[46px] overflow-hidden rounded-[7px] bg-subtle"
      >
        {overview.speech.map(([who, from, to]) => (
          <span
            key={`${who}-${from}`}
            className="absolute rounded-[3px]"
            style={{
              left: at(from),
              top: 6 + people.indexOf(who) * (laneHeight + 2),
              width: across(from, to, 0.003),
              height: laneHeight,
              background: colours[who] ?? "var(--muted-foreground)",
            }}
          />
        ))}
        {overview.screens.map(([from, to]) => (
          <span
            key={from}
            className="absolute bottom-2.5 h-[3px] rounded-sm bg-muted-foreground/45"
            style={{ left: at(from), width: across(from, to, 0.004) }}
          />
        ))}
        {overview.drawings.map(([t, who]) => (
          <span
            key={`${t}-${who}`}
            className="absolute bottom-[3px] size-1.5 rounded-full"
            style={{ left: at(t), background: colours[who] ?? "var(--muted-foreground)" }}
          />
        ))}
      </div>
      <div className="mt-[3px] flex justify-between font-mono text-[10.5px] text-faint-foreground">
        {ticks.map((t) => (
          <span key={t}>{clock(t)}</span>
        ))}
      </div>
    </div>
  );
};

const FILTERS: { id: Filter; label: string; kind?: CaptureRow["kind"] }[] = [
  { id: "everything", label: "Everything" },
  { id: "said", label: "Said", kind: "said" },
  { id: "drawing", label: "Drawings", kind: "drawing" },
  { id: "screen", label: "Screen", kind: "screen" },
];

const rowKeys = (rows: CaptureRow[]): FeedItem<CaptureRow>[] => {
  const seen = new Map<string, number>();
  return rows.map((row) => {
    const base = `${row.kind}-${row.t_ms}-${row.kind === "said" ? row.speaker : ""}`;
    const times = seen.get(base) ?? 0;
    seen.set(base, times + 1);
    return { key: times === 0 ? base : `${base}#${times}`, value: row };
  });
};

const Detail = ({
  reading,
  narrow,
  onShowList,
  onDeleted,
}: {
  reading: Reading;
  narrow: boolean;
  onShowList: () => void;
  onDeleted: () => void;
}) => {
  const { call, rows, overview, colours, pictures } = reading;
  const live = call.ended_ms === undefined;
  const now = useNow(live);
  const [filter, setFilter] = useState<Filter>("everything");
  const [find, setFind] = useState("");
  const [viewing, setViewing] = useState<number | null>(null);
  const [whole, setWhole] = useState<ReadonlySet<number>>(new Set());
  const [copied, setCopied] = useState(false);
  const [armed, setArmed] = useState(false);
  const feed = useRef<FeedHandle>(null);
  const needle = find.trim().toLowerCase();

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), COPIED_MS);
    return () => clearTimeout(timer);
  }, [copied]);

  useEffect(() => {
    if (!armed) return;
    const timer = setTimeout(() => setArmed(false), DELETE_ARMED_MS);
    return () => clearTimeout(timer);
  }, [armed]);

  const shown = useMemo(
    () =>
      rowKeys(
        rows.filter((row) => {
          const kind = FILTERS.find((each) => each.id === filter)?.kind;
          if (kind && row.kind !== kind) return false;
          return !needle || (row.kind === "said" && row.text.toLowerCase().includes(needle));
        }),
      ),
    [rows, filter, needle],
  );

  const jump = useCallback(
    (t_ms: number) => {
      const index = shown.findIndex(({ value }) => value.t_ms >= t_ms - 1_000);
      feed.current?.jumpTo(index >= 0 ? index : Math.max(shown.length - 1, 0));
    },
    [shown],
  );

  const onWhole = useCallback((frame: number, on: boolean) => {
    setWhole((current) => {
      const next = new Set(current);
      if (on) next.add(frame);
      else next.delete(frame);
      return next;
    });
  }, []);

  const onPicture = useCallback(
    (frame: number) => {
      const index = pictures.findIndex((picture) => picture.frame === frame);
      if (index >= 0) setViewing(index);
    },
    [pictures],
  );

  const onCloseViewer = useCallback(() => setViewing(null), []);
  const context = useMemo(() => ({ callId: call.id, colours }), [call.id, colours]);

  const renderRow = useCallback(
    (row: CaptureRow) => (
      <Row
        row={row}
        find={needle}
        whole={row.kind === "drawing" && whole.has(row.frame)}
        onTime={jump}
        onWhole={onWhole}
        onPicture={onPicture}
      />
    ),
    [jump, needle, onPicture, onWhole, whole],
  );

  const elapsed = (call.ended_ms ?? now) - call.started_ms;
  const total = Math.max(live ? Math.max(overview.duration_ms, elapsed) : overview.duration_ms, 1);
  const count = (kind: CaptureRow["kind"]) => rows.filter((row) => row.kind === kind).length;
  const drawings = count("drawing");
  const screens = count("screen");
  const details = [
    longDate(call.started_ms),
    ...(drawings ? [plural(drawings, "drawing")] : []),
    ...(screens ? [plural(screens, "screen change")] : []),
  ].join(" · ");

  const onDelete = () => {
    if (!armed) {
      setArmed(true);
      return;
    }
    typedInvoke("captures_delete", { callId: call.id }).then(
      () => onDeleted(),
      (error) => console.error("Deleting a capture:", error),
    );
  };

  const copyPrompt = () => {
    writeText(agentPrompt(call)).then(
      () => setCopied(true),
      (error) => console.error("Copying the prompt:", error),
    );
  };

  return (
    <CallContext.Provider value={context}>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
        <div
          data-tauri-drag-region
          className={clsx(
            "flex flex-col gap-2 border-b border-border pt-3 pr-[18px] pb-3",
            narrow ? "pl-[76px]" : "pl-[18px]",
          )}
        >
          <div data-tauri-drag-region className="flex items-start gap-3">
            <div data-tauri-drag-region className="flex min-w-0 flex-1 flex-col gap-1.5">
              <div data-tauri-drag-region className="flex items-center gap-2">
                <h1 className="min-w-0 truncate text-base font-semibold text-foreground selectable">{heading(call)}</h1>
                {live ?
                  <span className="flex shrink-0 items-center gap-[5px] rounded-full bg-destructive-soft px-2 py-0.5 text-[11px] font-semibold text-destructive tabular-nums">
                    <span className="size-1.5 rounded-full bg-destructive" />
                    Capturing · {clock(elapsed)}
                  </span>
                : <span className="shrink-0 rounded-full bg-subtle px-2 py-0.5 text-[11px] font-semibold text-muted-foreground tabular-nums">
                    Ended · {clock(elapsed)}
                  </span>
                }
              </div>
              <div data-tauri-drag-region className="flex items-center gap-2.5">
                {call.participants.length > 0 && (
                  <Faces names={call.participants} size={22} colours={colours} ring="var(--background)" />
                )}
                <span className="min-w-0 truncate text-xs text-muted-foreground">{details}</span>
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-1">
              {live && (
                <>
                  <Action
                    compact={narrow}
                    icon={<HiOutlineBookmark />}
                    label="Bookmark"
                    onClick={() => typedInvoke("captures_bookmark")}
                  />
                  <Action
                    compact={narrow}
                    tone="danger"
                    icon={<HiOutlineStopCircle />}
                    label="Stop"
                    onClick={() => typedInvoke("captures_stop")}
                  />
                </>
              )}
              <Action
                compact={narrow}
                icon={copied ? <HiCheck /> : <HiOutlineClipboard />}
                label={copied ? "Copied" : "Copy agent prompt"}
                onClick={copyPrompt}
              />
              {!live && (
                <>
                  <Action
                    compact={narrow}
                    icon={<HiOutlineDocumentText />}
                    label="Open Markdown"
                    onClick={() => typedInvoke("captures_open_markdown", { callId: call.id })}
                  />
                  <Action
                    compact={narrow}
                    tone={armed ? "armed" : undefined}
                    icon={<HiOutlineTrash />}
                    label={armed ? "Click again to delete" : "Delete"}
                    onClick={onDelete}
                  />
                </>
              )}
              {narrow && (
                <IconButton label="All calls" onClick={onShowList}>
                  <HiOutlineBars3 />
                </IconButton>
              )}
            </div>
          </div>
          {call.summary && (
            <div className="mt-1 rounded-[9px] bg-subtle px-3 py-2 leading-[18px] text-foreground selectable">
              {call.summary}
            </div>
          )}
        </div>
        {rows.length > 0 && (
          <>
            <Scrubber overview={overview} total={total} colours={colours} onJump={jump} />
            <div className="flex items-center gap-2.5 border-b border-border px-[18px] py-[9px]">
              <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5">
                {FILTERS.map(({ id, label, kind }) => {
                  const number = kind ? count(kind) : rows.length;
                  if (kind && number === 0) return null;
                  return (
                    <button
                      key={id}
                      onClick={() => setFilter(id)}
                      className={clsx(
                        "flex items-center gap-[5px] rounded-full border px-2.5 py-1 text-xs transition-colors",
                        filter === id ?
                          "border-transparent bg-subtle-strong text-foreground"
                        : "border-border text-muted-foreground hover:bg-subtle",
                      )}
                    >
                      {label}
                      {kind && <span className="text-faint-foreground">{number}</span>}
                    </button>
                  );
                })}
              </div>
              <div className={clsx("shrink-0", narrow ? "w-[130px]" : "w-[210px]")}>
                <SearchInput value={find} onChange={setFind} placeholder="Search this call" />
              </div>
            </div>
          </>
        )}
        {shown.length === 0 ?
          <div className="flex flex-1 items-center justify-center px-6 text-center text-xs text-muted-foreground">
            {rows.length > 0 ?
              "Nothing here matches."
            : live ?
              "Listening. What is said appears within a few seconds."
            : "Nothing was said."}
          </div>
        : <Feed
            // Each call scrolls and measures its rows from scratch; a filter starts from the top.
            key={`${call.id}/${filter}/${needle ? "find" : ""}`}
            ref={feed}
            items={shown}
            follow={live && filter === "everything" && !needle}
            estimate={estimateRow}
            render={renderRow}
          />
        }
      </div>
      {viewing !== null && pictures[viewing] && (
        <Viewer callId={call.id} pictures={pictures} index={viewing} onIndex={setViewing} onClose={onCloseViewer} />
      )}
    </CallContext.Provider>
  );
};

function CapturesWindow() {
  const narrow = useNarrow();
  const [calls, setCalls] = useState<CaptureListed[]>([]);
  const [listError, setListError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [query, setQuery] = useState("");
  const [showList, setShowList] = useState(false);
  const lastLive = useRef<string | undefined>(undefined);
  const queryRef = useRef(query);
  queryRef.current = query;
  const selectedRef = useRef<CaptureCall | undefined>(undefined);

  const reloadList = useCallback(async () => {
    const asked = queryRef.current;
    try {
      const listed = await typedInvoke("captures_list", { query: asked });
      if (asked === queryRef.current) setCalls(listed);
      setListError(null);
      return listed;
    } catch (error) {
      setListError(String(error));
      return [];
    } finally {
      setLoaded(true);
    }
  }, []);

  const { reading, open, close } = useReading(() => showAnother());
  selectedRef.current = reading?.call;

  const select = useCallback(
    (which: string) => {
      setShowList(false);
      return open(which).catch((error) => console.error("Opening a capture:", error));
    },
    [open],
  );

  // A new capture is selected unless you're reading another live one.
  const followNewCapture = useCallback(
    (listed: CaptureListed[]) => {
      const live = listed.find((call) => call.ended_ms === undefined)?.id;
      if (live && live !== lastLive.current) {
        const reading = selectedRef.current;
        if (!reading || reading.ended_ms !== undefined) select(live);
      }
      lastLive.current = live;
    },
    [select],
  );

  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const again = async () => {
      if (!document.hidden && !queryRef.current) {
        const listed = await reloadList();
        if (!stopped) followNewCapture(listed);
      }
      if (!stopped) timer = setTimeout(again, LIST_POLL_MS);
    };
    (async () => {
      const requested = await typedInvoke("captures_requested_call");
      const listed = await reloadList();
      if (stopped) return;
      const first = listed.find((call) => call.ended_ms === undefined) ?? listed[0];
      lastLive.current = listed.find((call) => call.ended_ms === undefined)?.id;
      const which = requested ?? first?.id;
      if (which) select(which);
      timer = setTimeout(again, LIST_POLL_MS);
    })();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [followNewCapture, reloadList, select]);

  useEffect(() => {
    const timer = setTimeout(reloadList, SEARCH_WAITS_MS);
    return () => clearTimeout(timer);
  }, [query, reloadList]);

  // `hopp capture window <call>` with the window already open.
  useEffect(() => {
    const unlisten = listen("captures_call_requested", async () => {
      const requested = await typedInvoke("captures_requested_call");
      if (requested) select(requested);
    });
    return () => {
      unlisten.then((stop) => stop());
    };
  }, [select]);

  // The call being read is no more.
  const showAnother = useCallback(async () => {
    close();
    const listed = await reloadList();
    if (listed[0]) select(listed[0].id);
  }, [close, reloadList, select]);

  const onShowList = useCallback(() => setShowList(true), []);

  // Words picked out can be copied; nothing else has a menu.
  useEffect(() => {
    const onMenu = (event: MouseEvent) => {
      if (window.getSelection()?.isCollapsed !== false) event.preventDefault();
    };
    document.addEventListener("contextmenu", onMenu);
    return () => document.removeEventListener("contextmenu", onMenu);
  }, []);

  if (listError && calls.length === 0) {
    return (
      <div className="flex h-screen flex-col gap-2 bg-background p-6 pt-12 text-[13px] text-muted-foreground">
        <span className="font-semibold text-foreground">Captures</span>
        The capture database couldn&rsquo;t be opened: {listError}
      </div>
    );
  }

  const side = (
    <div
      className={clsx("flex min-h-0 flex-col bg-rail", narrow ? "flex-1" : "w-[256px] shrink-0 border-r border-border")}
    >
      <div data-tauri-drag-region className="flex items-center gap-2 px-4 pt-11 pb-2.5">
        <span className="flex-1 text-[13px] font-semibold text-foreground">Calls</span>
        {narrow && reading && (
          <IconButton label="Back to the call" onClick={() => setShowList(false)}>
            <HiXMark />
          </IconButton>
        )}
      </div>
      <div className="px-3 pb-2.5">
        <SearchInput value={query} onChange={setQuery} placeholder="Search every call" />
      </div>
      <CallList
        calls={calls}
        selected={reading?.call.id}
        colours={reading?.colours ?? {}}
        searching={query.trim().length > 0}
        onSelect={select}
      />
    </div>
  );

  const detail =
    reading ?
      <Detail key={reading.call.id} reading={reading} narrow={narrow} onShowList={onShowList} onDeleted={showAnother} />
    : <div
        data-tauri-drag-region
        className="flex flex-1 items-center justify-center bg-background text-[13px] text-muted-foreground"
      >
        {loaded ? "Choose a call" : ""}
      </div>;

  return (
    <div className="flex h-screen overflow-hidden bg-background text-[13px] text-foreground">
      {narrow ?
        showList || !reading ?
          side
        : detail
      : <>
          {side}
          {detail}
        </>
      }
    </div>
  );
}
