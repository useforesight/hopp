import { ReactNode } from "react";

// Stays at the top of a tab while its content scrolls underneath
export const PageHeader = ({
  title,
  aside,
  children,
}: {
  title: ReactNode;
  aside?: ReactNode;
  children?: ReactNode;
}) => (
  <header className="sticky top-0 z-10 flex flex-col gap-2.5 bg-background/75 px-3.5 pb-2.5 pt-3.5 backdrop-blur-xl">
    <div className="flex min-h-6 items-center justify-between gap-2">
      <h1 className="text-[19px] leading-6 font-semibold tracking-[-0.015em] text-foreground">{title}</h1>
      {aside}
    </div>
    {children}
  </header>
);

export const SectionLabel = ({ children }: { children: ReactNode }) => (
  <h2 className="border-none px-2 pb-1 pt-2 text-[11px] leading-4 font-medium tracking-[0.05em] text-faint-foreground uppercase">
    {children}
  </h2>
);

export const EmptyState = ({
  icon,
  title,
  description,
  children,
}: {
  icon?: ReactNode;
  title: string;
  description?: ReactNode;
  children?: ReactNode;
}) => (
  <div className="flex flex-col items-center gap-1.5 px-6 py-10 text-center animate-in fade-in-0 duration-300">
    {icon && (
      <div className="mb-1.5 flex size-11 items-center justify-center rounded-[14px] bg-subtle text-muted-foreground ring-1 ring-inset ring-border">
        {icon}
      </div>
    )}
    <p className="text-[13px] leading-[18px] font-medium text-foreground">{title}</p>
    {description && <p className="text-xs leading-[18px] text-muted-foreground">{description}</p>}
    {children && <div className="mt-2.5 flex flex-col items-center gap-1.5">{children}</div>}
  </div>
);
