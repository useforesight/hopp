import {
  forwardRef,
  useCallback,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
import { HiArrowDown } from "react-icons/hi2";

/** Rows kept ready above and below what's showing, in pixels. */
const OVERSCAN = 900;
/** This close to the end still counts as at the end. */
const AT_THE_END = 48;
/** How long a jump keeps correcting itself while the rows it lands on are measured. */
const JUMP_SETTLES_MS = 400;

export interface FeedItem<T> {
  key: string;
  value: T;
}

export interface FeedHandle {
  jumpTo: (index: number) => void;
}

interface FeedProps<T> {
  items: FeedItem<T>[];
  /** The call is live: stay at the end until the reader scrolls away. */
  follow: boolean;
  /** A guess at a row's height until it has been drawn. */
  estimate: (value: T) => number;
  render: (value: T) => ReactNode;
}

const Measured = ({
  id,
  onSize,
  children,
}: {
  id: string;
  onSize: (id: string, height: number) => void;
  children: ReactNode;
}) => {
  const box = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = box.current;
    if (!element) return;
    onSize(id, element.offsetHeight);
    const observer = new ResizeObserver(() => onSize(id, element.offsetHeight));
    observer.observe(element);
    return () => observer.disconnect();
  }, [id, onSize]);
  return <div ref={box}>{children}</div>;
};

/**
 * A list that only draws the rows in view, so a long call stays light, whatever their
 * heights. Rows are measured as they're drawn. Rows growing above the reader don't move
 * what they're reading.
 */
const FeedInner = <T,>({ items, follow, estimate, render }: FeedProps<T>, ref: React.ForwardedRef<FeedHandle>) => {
  const scroller = useRef<HTMLDivElement>(null);
  const heights = useRef(new Map<string, number>());
  const [measured, rowsMeasured] = useReducer((count: number) => count + 1, 0);
  const [view, setView] = useState({ top: 0, height: 800 });
  const [following, setFollowing] = useState(follow);
  const stuck = useRef(follow);
  const lastTop = useRef(0);
  const jump = useRef<{ index: number; until: number } | null>(null);

  const offsets = useMemo(() => {
    const out = new Array<number>(items.length + 1);
    out[0] = 0;
    items.forEach((item, index) => {
      out[index + 1] = out[index]! + (heights.current.get(item.key) ?? estimate(item.value));
    });
    return out;
    // `measured` says that `heights` has changed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [items, measured, estimate]);

  // What the row-size callback needs, without changing it every render.
  const latest = useRef({ items, offsets, estimate });
  latest.current = { items, offsets, estimate };

  const onSize = useCallback((id: string, height: number) => {
    const known = heights.current.get(id);
    if (known === height) return;
    heights.current.set(id, height);
    const element = scroller.current;
    if (element && !stuck.current && !jump.current) {
      const { items, offsets, estimate } = latest.current;
      const index = items.findIndex((item) => item.key === id);
      if (index >= 0) {
        const before = known ?? estimate(items[index]!.value);
        if (offsets[index + 1]! <= element.scrollTop) element.scrollTop += height - before;
      }
    }
    rowsMeasured();
  }, []);

  useLayoutEffect(() => {
    const element = scroller.current;
    if (!element) return;
    if (!follow) stuck.current = false;
    if (jump.current && performance.now() > jump.current.until) jump.current = null;
    if (jump.current) {
      element.scrollTop = offsets[Math.min(jump.current.index, items.length)] ?? 0;
    } else if (stuck.current) {
      element.scrollTop = element.scrollHeight;
    }
  });

  useLayoutEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const observer = new ResizeObserver(() => {
      setView({ top: element.scrollTop, height: element.clientHeight });
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useImperativeHandle(
    ref,
    () => ({
      jumpTo: (index) => {
        stuck.current = false;
        setFollowing(false);
        jump.current = { index, until: performance.now() + JUMP_SETTLES_MS };
        rowsMeasured();
      },
    }),
    [],
  );

  const onScroll = (event: React.UIEvent<HTMLDivElement>) => {
    const element = event.currentTarget;
    setView({ top: element.scrollTop, height: element.clientHeight });
    const movedUp = element.scrollTop < lastTop.current - 1;
    lastTop.current = element.scrollTop;
    if (jump.current) return;
    // Rows arriving move the end away from the reader, so only their scrolling up lets go.
    if (element.scrollHeight - element.scrollTop - element.clientHeight < AT_THE_END) stuck.current = follow;
    else if (movedUp) stuck.current = false;
    setFollowing(stuck.current);
  };

  const backToLive = () => {
    jump.current = null;
    stuck.current = true;
    setFollowing(true);
    rowsMeasured();
  };

  let first = 0;
  while (first < items.length && offsets[first + 1]! < view.top - OVERSCAN) first += 1;
  let end = first;
  while (end < items.length && offsets[end]! < view.top + view.height + OVERSCAN) end += 1;

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={scroller}
        onScroll={onScroll}
        onWheel={() => {
          jump.current = null;
        }}
        className="min-h-0 flex-1 overflow-y-auto py-2"
      >
        <div style={{ height: offsets[first] }} />
        {items.slice(first, end).map((item) => (
          <Measured key={item.key} id={item.key} onSize={onSize}>
            {render(item.value)}
          </Measured>
        ))}
        <div style={{ height: offsets[items.length]! - offsets[end]! }} />
      </div>
      {follow && !following && (
        <button
          onClick={backToLive}
          className="absolute bottom-4 left-1/2 flex h-8 -translate-x-1/2 items-center gap-1.5 rounded-full border border-border-strong bg-raised px-3 text-xs font-medium text-foreground shadow-card transition-colors hover:bg-subtle-strong"
        >
          <HiArrowDown className="size-3.5" />
          Back to now
        </button>
      )}
    </div>
  );
};

export const Feed = forwardRef(FeedInner) as <T>(
  props: FeedProps<T> & { ref?: React.Ref<FeedHandle> },
) => React.ReactElement;
