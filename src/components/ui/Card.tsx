import { type ReactNode } from "react";
import { cn } from "@/lib/utils";

interface CardProps {
  title?: ReactNode;
  action?: ReactNode;
  className?: string;
  children?: ReactNode;
  variant?: "default" | "muted" | "accent";
  noPad?: boolean;
}

export function Card({ title, action, className, children, variant = "default", noPad }: CardProps) {
  const variantCls =
    variant === "muted"
      ? "bg-bg-surface border-transparent"
      : variant === "accent"
        ? "bg-bg-surface border-l-2 border-l-accent border-y border-r border-border"
        : "bg-bg-surface border border-border";
  return (
    <div className={cn("rounded-card", variantCls, className)}>
      {(title || action) && (
        <div className="flex items-center justify-between px-5 pt-4 pb-3 border-b border-border/60">
          <div className="text-text-primary font-medium text-[13px] tracking-wide flex items-center gap-2">{title}</div>
          {action && <div className="text-text-secondary">{action}</div>}
        </div>
      )}
      <div className={cn(noPad ? "" : "p-5")}>{children}</div>
    </div>
  );
}
