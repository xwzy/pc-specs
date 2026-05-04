import {
  Area,
  AreaChart,
  CartesianGrid,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

export interface SeriesConfig {
  key: string;
  label: string;
  color?: string;
}

interface LiveLineChartProps {
  data: Array<Record<string, number>>;
  series: SeriesConfig[];
  height?: number;
  yFormatter?: (v: number) => string;
  xKey?: string;
  yDomain?: [number | "auto" | "dataMin", number | "auto" | "dataMax"];
  showLegend?: boolean;
}

const DEFAULT_COLORS = [
  "rgb(var(--accent))",
  "rgb(var(--accent-2))",
  "rgb(var(--success))",
  "rgb(var(--warning))",
  "rgb(var(--info))",
  "rgb(var(--danger))",
];

export function LiveLineChart({
  data,
  series,
  height = 180,
  yFormatter,
  xKey = "t",
  yDomain,
  showLegend = false,
}: LiveLineChartProps) {
  return (
    <div style={{ width: "100%", height }}>
      <ResponsiveContainer>
        <AreaChart data={data} margin={{ top: 8, right: 12, left: 8, bottom: 0 }}>
          <defs>
            {series.map((s, i) => (
              <linearGradient key={s.key} id={`grad-${s.key}`} x1="0" x2="0" y1="0" y2="1">
                <stop offset="0%" stopColor={s.color ?? DEFAULT_COLORS[i % DEFAULT_COLORS.length]} stopOpacity={0.35} />
                <stop offset="100%" stopColor={s.color ?? DEFAULT_COLORS[i % DEFAULT_COLORS.length]} stopOpacity={0} />
              </linearGradient>
            ))}
          </defs>
          <CartesianGrid stroke="rgb(var(--border))" strokeOpacity={0.4} vertical={false} />
          <XAxis
            dataKey={xKey}
            tick={{ fontSize: 10, fill: "rgb(var(--text-tertiary))" }}
            stroke="rgb(var(--border))"
            tickFormatter={(v) => {
              const d = new Date(v as number);
              return `${d.getHours().toString().padStart(2, "0")}:${d
                .getMinutes()
                .toString()
                .padStart(2, "0")}:${d.getSeconds().toString().padStart(2, "0")}`;
            }}
            minTickGap={48}
          />
          <YAxis
            tick={{ fontSize: 10, fill: "rgb(var(--text-tertiary))" }}
            stroke="rgb(var(--border))"
            tickFormatter={(v) => (yFormatter ? yFormatter(v as number) : `${v}`)}
            domain={yDomain ?? ["auto", "auto"]}
            width={48}
          />
          <Tooltip
            contentStyle={{
              background: "rgb(var(--bg-surface-2))",
              border: "1px solid rgb(var(--border-strong))",
              borderRadius: 8,
              fontFamily: "JetBrains Mono, monospace",
              fontSize: 12,
              color: "rgb(var(--text-primary))",
            }}
            labelFormatter={(v) => new Date(v as number).toLocaleTimeString()}
            formatter={(v: number, name: string) => [yFormatter ? yFormatter(v) : v, name]}
          />
          {showLegend && <Legend wrapperStyle={{ fontSize: 11, color: "rgb(var(--text-secondary))" }} />}
          {series.map((s, i) => (
            <Area
              key={s.key}
              type="monotone"
              name={s.label}
              dataKey={s.key}
              stroke={s.color ?? DEFAULT_COLORS[i % DEFAULT_COLORS.length]}
              strokeWidth={1.5}
              fill={`url(#grad-${s.key})`}
              isAnimationActive={false}
              dot={false}
            />
          ))}
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}
