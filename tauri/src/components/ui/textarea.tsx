import * as React from "react";

import { cn } from "@/lib/utils";

const Textarea = React.forwardRef<HTMLTextAreaElement, React.ComponentProps<"textarea">>(
  ({ className, ...props }, ref) => {
    return (
      <textarea
        className={cn(
          "flex min-h-[60px] w-full rounded-lg border border-border bg-subtle px-3 py-2 text-sm text-foreground transition-[border-color,box-shadow] duration-150 placeholder:text-faint-foreground hover:border-border-strong focus-visible:outline-none focus-visible:border-accent focus-visible:ring-3 focus-visible:ring-accent-soft disabled:cursor-not-allowed disabled:opacity-50",
          className,
        )}
        ref={ref}
        {...props}
      />
    );
  },
);
Textarea.displayName = "Textarea";

export { Textarea };
