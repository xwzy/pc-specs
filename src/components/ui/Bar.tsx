import { cn } from "@/lib/utils";
import { clamp } from "@/lib/format";

interface BarProps {
  value: number;
  max?: number;
  className?: string;
  warningAt?: number;
  dangerAt?: number;
}

export function Bar({ value, max = 100, className, warningAt = 75, dangerAt = 90 }: BarProps) {
  const pct = max > 0 ? clamp((value / max) * 100, 0, 100) : 0;
  const tone =
    pct >= dangerAt
      ? "bg-danger"
      : pct >= warningAt
        ? "bg-warning"
        : "bg-accent";
  return (
    <div className={cn("h-1.5 w-full bg-bg-elevated rounded-full overflow-hidden", className)}>
      <div className={cn("h-full transition-[width] duration-300 ease-out rounded-full", tone)} style={{ width: `${pct}%` }} />
    </div>
  );
}
