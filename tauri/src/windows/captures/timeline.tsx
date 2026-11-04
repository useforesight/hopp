import { createContext, memo, useContext, useState } from "react";
import type { ReactNode } from "react";
import clsx from "clsx";
import { HiOutlineBookmark, HiOutlineComputerDesktop, HiOutlineMicrophone } from "react-icons/hi2";
import type { CaptureMark, CaptureRow } from "@/core_payloads";
import { clock, everyoneWhoDrew, firstName, initials, joinNames, markGroups, span } from "./format";
import { useFrame } from "./frames";

/** What's the same for every row of a call, so rows only draw again when they change. */
export interface CallView {
  callId: string;
  colours: Record<string, string>;
}

export const CallContext = createContext<CallView>({ callId: "", colours: {} });

/** A round face with initials, in the person's colour if the call gave them one. */
export const Face = ({ name, size, colour }: { name: string; size: number; colour?: string }) => (
  <span
    title={name}
    className={clsx(
      "flex shrink-0 items-center justify-center rounded-full font-bold select-none",
      !colour && "bg-subtle-strong text-muted-foreground",
    )}
    style={{
      width: size,
      height: size,
      fontSize: Math.max(size * 0.4, 7),
      ...(colour ? { background: colour, color: "#14151a" } : {}),
    }}
  >
    {initials(name)}
  </span>
);

/** Faces overlapping a little, left to right. */
export const Faces = ({
  names,
  size,
  colours,
  ring,
}: {
  names: string[];
  size: number;
  colours: Record<string, string>;
  ring: string;
}) => (
  <span className="flex shrink-0 items-center">
    {names.map((name, index) => (
      <span
        key={name}
        className="rounded-full"
        style={{ marginLeft: index === 0 ? 0 : -size * 0.18, boxShadow: `0 0 0 2px ${ring}` }}
      >
        <Face name={name} size={size} colour={colours[name]} />
      </span>
    ))}
  </span>
);

/** `text` with whatever matches `find` picked out. */
const Highlighted = ({ text, find }: { text: string; find: string }) => {
  if (!find) return <>{text}</>;
  const lower = text.toLowerCase();
  const parts: ReactNode[] = [];
  let at = 0;
  for (let found = lower.indexOf(find); found >= 0; found = lower.indexOf(find, at)) {
    parts.push(text.slice(at, found));
    parts.push(
      <mark key={found} className="rounded-[3px] bg-warning-soft text-inherit">
        {text.slice(found, found + find.length)}
      </mark>,
    );
    at = found + find.length;
  }
  parts.push(text.slice(at));
  return <>{parts}</>;
};

/** A frame, showing only `crop` of it, with the marks in `lit` outlined. */
const Cropped = ({
  frame,
  width,
  height,
  crop,
  marks,
  lit,
}: {
  frame: number;
  width: number;
  height: number;
  crop: [number, number, number, number];
  marks: CaptureMark[];
  lit: number[];
}) => {
  const { callId } = useContext(CallContext);
  const { url, failed } = useFrame(callId, frame);
  const [left, top, w, h] = crop;
  return (
    <div
      className="relative w-full overflow-hidden bg-black"
      style={{ aspectRatio: (w * Math.max(width, 1)) / (h * Math.max(height, 1)) }}
    >
      {failed && (
        <span className="absolute inset-0 flex items-center justify-center text-xs text-white/60">
          This picture is missing
        </span>
      )}
      <div
        className="absolute"
        style={{
          left: `${(-left / w) * 100}%`,
          top: `${(-top / h) * 100}%`,
          width: `${(1 / w) * 100}%`,
          height: `${(1 / h) * 100}%`,
        }}
      >
        {url && <img src={url} alt="" draggable={false} className="block size-full" />}
        {lit.map((index) => {
          const mark = marks[index];
          if (!mark) return null;
          const [l, t, r, b] = mark.bounds;
          const padX = 0.006;
          const padY = 0.014;
          return (
            <span
              key={index}
              className="absolute rounded-md"
              style={{
                left: `${(Math.min(l, r) - padX) * 100}%`,
                top: `${(Math.min(t, b) - padY) * 100}%`,
                width: `${Math.max(Math.abs(r - l) + padX * 2, 0.02) * 100}%`,
                height: `${Math.max(Math.abs(b - t) + padY * 2, 0.04) * 100}%`,
                border: `2px solid ${mark.colour}`,
                background: `${mark.colour}24`,
              }}
            />
          );
        })}
      </div>
    </div>
  );
};

const Said = ({
  speaker,
  text,
  draft,
  also,
  find,
}: {
  speaker: string;
  text: string;
  draft: boolean;
  also: string | null;
  find: string;
}) => {
  const { colours } = useContext(CallContext);
  const colour = colours[speaker];
  return (
    <div className="flex gap-2.5">
      <span className="mt-px">
        <Face name={speaker} size={26} colour={colour} />
      </span>
      <div className="min-w-0 flex-1">
        <div
          className="text-[12.5px] leading-[18px] font-semibold"
          style={colour ? { color: `color-mix(in srgb, ${colour} 78%, var(--foreground))` } : undefined}
        >
          {speaker}
        </div>
        <div className="mt-px text-[14px] leading-[21px] selectable">
          {draft ?
            <span className="text-muted-foreground italic">{text ? `${text} …` : "…"}</span>
          : <span className="text-foreground">
              <Highlighted text={text} find={find} />
            </span>
          }
        </div>
        {also && (
          <div className="mt-0.5 flex items-center gap-1 text-[11px] text-faint-foreground">
            <HiOutlineMicrophone className="size-3" />
            also on {firstName(also)}&rsquo;s mic, shown once
          </div>
        )}
      </div>
    </div>
  );
};

const Segment = ({ on, label, onClick }: { on: boolean; label: string; onClick: () => void }) => (
  <button
    onClick={onClick}
    className={clsx(
      "px-2 py-0.5 text-[11.5px] transition-colors",
      on ? "bg-subtle-strong text-foreground" : "text-muted-foreground hover:text-foreground",
    )}
  >
    {label}
  </button>
);

const Card = ({ children }: { children: ReactNode }) => (
  <div className="overflow-hidden rounded-xl border border-border bg-surface shadow-card">{children}</div>
);

const Drawing = ({
  row,
  whole,
  onWhole,
  onPicture,
}: {
  row: Extract<CaptureRow, { kind: "drawing" }>;
  whole: boolean;
  onWhole: (frame: number, whole: boolean) => void;
  onPicture: (frame: number) => void;
}) => {
  const { colours } = useContext(CallContext);
  const [lit, setLit] = useState<number[]>([]);
  const people = everyoneWhoDrew(row.marks);
  return (
    <Card>
      <div className="flex items-center gap-2 px-3 py-2 text-[12.5px]">
        <Faces names={people} size={18} colours={colours} ring="var(--surface)" />
        <span className="min-w-0 truncate font-semibold text-foreground">{joinNames(people.map(firstName))} drew</span>
        <span className="flex-1" />
        <span className="text-muted-foreground">{span(row.t_ms, row.end_ms)}</span>
        <span className="flex shrink-0 overflow-hidden rounded-[7px] border border-border">
          <Segment on={!whole} label="Drawing" onClick={() => onWhole(row.frame, false)} />
          <Segment on={whole} label="Whole screen" onClick={() => onWhole(row.frame, true)} />
        </span>
      </div>
      <button aria-label="Open this picture" className="block w-full" onClick={() => onPicture(row.frame)}>
        <Cropped
          frame={row.frame}
          width={row.width}
          height={row.height}
          crop={whole ? [0, 0, 1, 1] : row.crop}
          marks={row.marks}
          lit={lit}
        />
      </button>
      <div className="flex flex-wrap gap-1.5 px-3 py-2.5">
        {markGroups(row.marks).map(({ by, label, indexes }) => (
          <span
            key={label}
            onMouseEnter={() => setLit(indexes)}
            onMouseLeave={() => setLit([])}
            className="flex items-center gap-1.5 rounded-full bg-subtle py-[3px] pr-2.5 pl-[7px] text-xs text-foreground transition-colors hover:bg-subtle-strong"
          >
            <span className="size-[9px] rounded-full" style={{ background: colours[by] }} />
            {label}
          </span>
        ))}
      </div>
    </Card>
  );
};

const Thumb = ({ frame, t_ms, onPicture }: { frame: number; t_ms: number; onPicture: (frame: number) => void }) => {
  const { callId } = useContext(CallContext);
  const { url } = useFrame(callId, frame);
  return (
    <button aria-label="Open this picture" className="group shrink-0 text-left" onClick={() => onPicture(frame)}>
      <span className="block h-[60px] w-[140px] overflow-hidden rounded-md border border-border bg-subtle transition-colors group-hover:border-accent">
        {url && <img src={url} alt="" draggable={false} className="block size-full object-cover" />}
      </span>
      <span className="mt-[3px] block font-mono text-[10.5px] text-faint-foreground">{clock(t_ms)}</span>
    </button>
  );
};

const Screen = ({
  row,
  onPicture,
}: {
  row: Extract<CaptureRow, { kind: "screen" }>;
  onPicture: (frame: number) => void;
}) => (
  <Card>
    <div className="flex items-center gap-2 px-3 py-2 text-[12.5px]">
      <HiOutlineComputerDesktop className="size-[14px] shrink-0 text-muted-foreground" />
      <span className="font-semibold text-foreground">
        {row.count === 1 ? "The shared screen" : `The shared screen changed ${row.count} times`}
      </span>
      <span className="flex-1" />
      <span className="text-muted-foreground">{span(row.t_ms, row.end_ms)}</span>
    </div>
    <div className="flex gap-2 overflow-x-auto px-3 pb-3">
      {row.thumbs.map(([frame, t_ms]) => (
        <Thumb key={frame} frame={frame} t_ms={t_ms} onPicture={onPicture} />
      ))}
    </div>
  </Card>
);

const body = (
  row: CaptureRow,
  find: string,
  whole: boolean,
  onWhole: (frame: number, whole: boolean) => void,
  onPicture: (frame: number) => void,
): ReactNode => {
  switch (row.kind) {
    case "event":
      return (
        <div className="flex items-center gap-2 pt-[3px] text-xs text-muted-foreground">
          <span className="size-1.5 shrink-0 rounded-full bg-faint-foreground/60" />
          <span className="selectable">{row.text}</span>
        </div>
      );
    case "bookmark":
      return (
        <div className="flex items-center gap-1.5 pt-0.5 text-[13px] text-warning">
          <HiOutlineBookmark className="size-[14px] shrink-0" />
          <span className="selectable">{row.who ? `${row.who}: ${row.text}` : row.text}</span>
        </div>
      );
    case "gap":
      return <div className="pt-[3px] text-xs text-destructive">Not transcribed: {row.text}</div>;
    case "said":
      return <Said speaker={row.speaker} text={row.text} draft={row.draft} also={row.also} find={find} />;
    case "drawing":
      return <Drawing row={row} whole={whole} onWhole={onWhole} onPicture={onPicture} />;
    case "screen":
      return <Screen row={row} onPicture={onPicture} />;
  }
};

/** One row of the timeline, centred at reading width with the time in the gutter. */
export const Row = memo(function Row({
  row,
  find,
  whole,
  onTime,
  onWhole,
  onPicture,
}: {
  row: CaptureRow;
  find: string;
  whole: boolean;
  onTime: (t_ms: number) => void;
  onWhole: (frame: number, whole: boolean) => void;
  onPicture: (frame: number) => void;
}) {
  const picture = row.kind === "drawing" || row.kind === "screen";
  return (
    <div className="flex justify-center px-[18px]">
      <div className={clsx("flex w-full max-w-[860px] gap-3", picture ? "py-2" : "py-[5px]")}>
        <button
          onClick={() => onTime(row.t_ms)}
          className="w-[52px] shrink-0 self-start pt-[3px] text-right font-mono text-[11px] text-faint-foreground transition-colors hover:text-foreground"
        >
          {clock(row.t_ms)}
        </button>
        <div className="min-w-0 flex-1">{body(row, find, whole, onWhole, onPicture)}</div>
      </div>
    </div>
  );
});

/** A rough height for a row that hasn't been drawn yet. */
export const estimateRow = (row: CaptureRow): number => {
  switch (row.kind) {
    case "said":
      return 58 + Math.floor(row.text.length / 90) * 21;
    case "drawing":
      return 470;
    case "screen":
      return 150;
    default:
      return 30;
  }
};
