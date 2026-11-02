import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { CgSpinner } from "react-icons/cg";

import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center whitespace-nowrap rounded-lg text-sm font-medium transition-[background-color,color,border-color,box-shadow,transform,opacity] duration-150 active:scale-[0.98] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground shadow-card hover:bg-primary/90",
        destructive: "bg-destructive text-destructive-foreground shadow-card hover:bg-destructive/90",
        "destructive-transparent":
          "bg-destructive-soft text-destructive hover:bg-destructive hover:text-destructive-foreground",
        outline: "border border-border-strong bg-subtle text-foreground hover:bg-subtle-strong",
        secondary: "bg-subtle-strong text-foreground hover:bg-subtle",
        ghost: "text-muted-foreground hover:bg-subtle hover:text-foreground",
        link: "text-foreground underline-offset-4 hover:underline",
        hidden: "hidden",
        loading: "bg-primary text-primary-foreground shadow-card",
        "gradient-white":
          "btn-gradient-white text-foreground font-semibold text-xs rounded-lg px-6 py-1.5 h-[29px] w-[75px] flex items-center justify-center border border-border-strong hover:scale-[1.025] transition-all duration-300",
      },
      size: {
        default: "h-9 px-4 py-2",
        sm: "h-8 rounded-lg px-3 text-xs",
        lg: "h-10 rounded-lg px-8",
        icon: "h-9 w-9",
        "icon-sm": "h-7 w-7",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>, VariantProps<typeof buttonVariants> {
  asChild?: boolean;
  isLoading?: boolean;
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, isLoading = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp className={cn(buttonVariants({ variant, size, className }))} ref={ref} {...props}>
        {isLoading && <CgSpinner className="animate-spin mr-2 size-3 text-current" />}
        {props.children}
      </Comp>
    );
  },
);
Button.displayName = "Button";

export { Button, buttonVariants };
