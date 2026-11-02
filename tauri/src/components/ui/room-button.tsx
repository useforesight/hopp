import clsx from "clsx";
import React from "react";
import { HiArrowRight } from "react-icons/hi2";

export const RoomButton: React.FC<
  React.PropsWithChildren<
    {
      cornerIcon?: React.ReactNode;
      presenceAvatars?: React.ReactNode;
      isCurrentRoom?: boolean;
      title: string;
    } & React.ComponentPropsWithoutRef<"button">
  >
> = ({ cornerIcon, presenceAvatars, isCurrentRoom = false, title, className = "", style, ...props }) => {
  return (
    <div className={clsx("group relative min-w-0", className)} style={style}>
      <button
        {...props}
        className={clsx(
          "flex h-[78px] w-full flex-col justify-between overflow-hidden rounded-xl p-2.5 text-left ring-1 ring-inset transition-[background-color,box-shadow,transform] duration-150",
          isCurrentRoom ? "bg-accent-soft ring-accent/40"
          : presenceAvatars ? "bg-success-soft ring-success/25 hover:ring-success/45"
          : "bg-surface shadow-card ring-border hover:bg-subtle hover:ring-border-strong",
          props.disabled ? "opacity-50" : "active:scale-[0.98]",
        )}
      >
        <span className="line-clamp-2 pr-5 text-[13px] leading-[17px] font-medium tracking-[-0.005em] text-balance text-foreground">
          {title}
        </span>
        <span className="flex h-5 w-full items-center justify-between gap-1.5">
          {presenceAvatars ?? <span className="text-[11px] text-faint-foreground">Empty</span>}
          {isCurrentRoom ?
            <span className="shrink-0 text-[11px] font-medium text-accent">You're here</span>
          : <span className="flex shrink-0 items-center gap-0.5 text-[11px] font-medium">
              {presenceAvatars && (
                <span className="flex items-center gap-1 text-success group-hover:hidden">
                  <span className="size-1.5 rounded-full bg-success" />
                  Live
                </span>
              )}
              <span
                className={clsx(
                  "items-center gap-0.5 text-muted-foreground group-hover:flex",
                  presenceAvatars ? "hidden" : "flex opacity-0 transition-opacity duration-150 group-hover:opacity-100",
                )}
              >
                Join
                <HiArrowRight className="size-3" />
              </span>
            </span>
          }
        </span>
      </button>
      {cornerIcon && (
        <div className="absolute right-1.5 top-1.5 flex opacity-0 transition-opacity duration-150 group-hover:opacity-100 focus-within:opacity-100 has-data-[state=open]:opacity-100">
          {cornerIcon}
        </div>
      )}
    </div>
  );
};
