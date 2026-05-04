import { type ReactNode } from "react";
import { cn } from "@/lib/utils";

interface StatProps {
  label: string;
  value: ReactNode;
  sub?: ReactNode;
  className?: string;
  size?: "md" | "lg";
}

export function Stat({ label, value, sub, className, size = "md" }: StatProps) {
  return (
    <div className={cn("flex flex-col gap-1", className)}>
      <div className="label-tt">{label}</div>
      <div
        className={cn(
          "font-mono leading-none text-text-primary tabular-nums",
          size === "lg" ? "text-metric" : "text-[22px]",
        )}
      >
        {value}
      </div>
      {sub ? <div className="text-text-secondary text-xs mt-0.5">{sub}</div> : null}
    </div>
  );
}
