import { useCallback, useEffect } from "react";
import clsx from "clsx";
import { HiChevronLeft, HiChevronRight, HiXMark } from "react-icons/hi2";
import type { CapturePicture } from "@/core_payloads";
import { useFrame } from "./frames";

export const IconButton = ({
  label,
  onClick,
  children,
  danger,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
  danger?: boolean;
}) => (
  <button
    title={label}
    aria-label={label}
    onClick={onClick}
    className={clsx(
      "flex size-[30px] shrink-0 items-center justify-center rounded-lg transition-colors [&>svg]:size-4",
      danger ? "bg-destructive-soft text-destructive" : "text-muted-foreground hover:bg-subtle hover:text-foreground",
    )}
  >
    {children}
  </button>
);

const StripThumb = ({
  callId,
  picture,
  on,
  onClick,
}: {
  callId: string;
  picture: CapturePicture;
  on: boolean;
  onClick: () => void;
}) => {
  const { url } = useFrame(callId, picture.frame);
  return (
    <button
      onClick={onClick}
      aria-label={picture.title}
      className={clsx(
        "h-[31px] w-[72px] shrink-0 overflow-hidden rounded border bg-subtle transition-opacity",
        on ? "border-accent" : "border-border opacity-55 hover:opacity-100",
      )}
    >
      {url && <img src={url} alt="" draggable={false} className="block size-full object-cover" />}
    </button>
  );
};

/** A picture full size over the window: arrows step through the call's pictures, Esc closes. */
export const Viewer = ({
  callId,
  pictures,
  index,
  onIndex,
  onClose,
}: {
  callId: string;
  pictures: CapturePicture[];
  index: number;
  onIndex: (index: number) => void;
  onClose: () => void;
}) => {
  const step = useCallback(
    (by: number) => onIndex((index + by + pictures.length) % pictures.length),
    [index, onIndex, pictures.length],
  );

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
      else if (event.key === "ArrowLeft") step(-1);
      else if (event.key === "ArrowRight") step(1);
      else return;
      event.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, step]);

  const picture = pictures[index];
  const { url } = useFrame(callId, picture?.frame ?? 0, true);
  const { url: small } = useFrame(callId, picture?.frame ?? 0, false);
  if (!picture) return null;
  const from = Math.max(index - 6, 0);

  return (
    <div className="fixed inset-0 z-40 flex flex-col bg-background p-[18px] pt-3">
      <div data-tauri-drag-region className="mb-3 flex items-center gap-2.5 pl-[68px]">
        <span className="shrink-0 font-semibold text-foreground">{picture.title}</span>
        <span className="min-w-0 flex-1 truncate text-muted-foreground">{picture.detail}</span>
        <IconButton label="Previous (←)" onClick={() => step(-1)}>
          <HiChevronLeft />
        </IconButton>
        <IconButton label="Next (→)" onClick={() => step(1)}>
          <HiChevronRight />
        </IconButton>
        <IconButton label="Close (Esc)" onClick={onClose}>
          <HiXMark />
        </IconButton>
      </div>
      <div className="flex min-h-0 flex-1 items-center justify-center">
        <img
          // The small copy stands in until the sharp one has loaded.
          src={url ?? small}
          alt={picture.title}
          draggable={false}
          className="block max-h-full max-w-full rounded-[10px] bg-black object-contain shadow-card"
          style={{ aspectRatio: `${Math.max(picture.width, 1)} / ${Math.max(picture.height, 1)}` }}
        />
      </div>
      <div className="mt-3 flex justify-center gap-1.5 overflow-hidden">
        {pictures.slice(from, from + 13).map((other, offset) => (
          <StripThumb
            key={other.frame}
            callId={callId}
            picture={other}
            on={from + offset === index}
            onClick={() => onIndex(from + offset)}
          />
        ))}
      </div>
    </div>
  );
};
