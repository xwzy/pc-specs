import { type ReactNode } from "react";
import { clamp } from "@/lib/format";

interface RingProgressProps {
  value: number;
  max?: number;
  size?: number;
  strokeWidth?: number;
  label?: ReactNode;
  sub?: ReactNode;
  warningAt?: number;
  dangerAt?: number;
}

export function RingProgress({
  value,
  max = 100,
  size = 96,
  strokeWidth = 8,
  label,
  sub,
  warningAt = 75,
  dangerAt = 90,
}: RingProgressProps) {
  const pct = max > 0 ? clamp((value / max) * 100, 0, 100) : 0;
  const r = (size - strokeWidth) / 2;
  const c = 2 * Math.PI * r;
  const offset = c - (pct / 100) * c;
  const stroke =
    pct >= dangerAt
      ? "rgb(var(--danger))"
      : pct >= warningAt
        ? "rgb(var(--warning))"
        : "rgb(var(--accent))";
  return (
    <div className="relative inline-flex items-center justify-center" style={{ width: size, height: size }}>
      <svg width={size} height={size} className="-rotate-90">
        <circle cx={size / 2} cy={size / 2} r={r} stroke="rgb(var(--bg-elevated))" strokeWidth={strokeWidth} fill="none" />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          stroke={stroke}
          strokeWidth={strokeWidth}
          strokeLinecap="round"
          strokeDasharray={c}
          strokeDashoffset={offset}
          fill="none"
          style={{ transition: "stroke-dashoffset 300ms ease-out" }}
        />
      </svg>
      <div className="absolute inset-0 flex flex-col items-center justify-center">
        <div className="font-mono text-text-primary text-base leading-none tabular-nums">{label}</div>
        {sub ? <div className="text-text-tertiary text-[10px] mt-1 uppercase tracking-wider">{sub}</div> : null}
      </div>
    </div>
  );
}
