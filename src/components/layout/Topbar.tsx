import { Moon, Sparkles, Sun } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { getHost, getOs } from "@/lib/api";
import { fmtUptime, useFmt } from "@/lib/format";
import { useSettings, useT } from "@/lib/store";
import { cn } from "@/lib/utils";
import { useMonitor } from "@/lib/useMonitor";

export function Topbar() {
  const t = useT();
  const theme = useSettings((s) => s.theme);
  const setTheme = useSettings((s) => s.setTheme);
  const geek = useSettings((s) => s.geekMode);
  const toggleGeek = useSettings((s) => s.toggleGeek);
  const fmt = useFmt();

  const { data: host } = useQuery({ queryKey: ["host"], queryFn: getHost });
  const { data: os } = useQuery({ queryKey: ["os"], queryFn: getOs });
  const { latest } = useMonitor(true);

  const memPct =
    latest && latest.mem_total_bytes > 0
      ? (latest.mem_used_bytes / latest.mem_total_bytes) * 100
      : null;
  const cpuPct = latest?.cpu_overall ?? null;
  // maxTempC 始终是摄氏度（后端语义），用于阈值判定；展示走 fmt.temp 转换
  const maxTempC =
    latest && latest.temperatures.length > 0
      ? Math.max(...latest.temperatures.map((t) => t.value))
      : null;

  return (
    <header className="h-12 shrink-0 border-b border-border bg-bg-surface flex items-center justify-between px-5">
      <div className="flex items-center gap-4 text-xs text-text-secondary">
        <div className="flex items-center gap-2">
          <span className="text-text-primary font-mono text-[13px]">{host?.hostname ?? "—"}</span>
          <span className="text-text-tertiary">·</span>
          <span className="font-mono">{os?.name ?? "—"}</span>
          {host && (
            <>
              <span className="text-text-tertiary">·</span>
              <span className="font-mono">up {fmtUptime(host.uptime_secs)}</span>
            </>
          )}
        </div>
      </div>
      <div className="flex items-center gap-3">
        <HealthLight label="CPU" value={cpuPct} fmt={(v) => `${v.toFixed(0)}%`} />
        <HealthLight label="RAM" value={memPct} fmt={(v) => `${v.toFixed(0)}%`} />
        <HealthLight
          label="TEMP"
          value={maxTempC}
          fmt={(v) => fmt.temp(v)}
          warningAt={70}
          dangerAt={88}
          max={100}
        />

        <button
          type="button"
          onClick={toggleGeek}
          className={cn(
            "flex items-center gap-1.5 px-2 py-1 rounded-md border border-border text-[11px] uppercase tracking-wider hover:bg-bg-elevated",
            geek ? "text-accent border-accent/40 bg-accent/5" : "text-text-secondary",
          )}
          title={t("settings_geek")}
        >
          <Sparkles size={12} />
          {t("topbar_geek")}
        </button>

        <button
          type="button"
          onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
          className="size-8 rounded-md border border-border flex items-center justify-center text-text-secondary hover:bg-bg-elevated"
          title={t("topbar_toggle_theme")}
        >
          {theme === "dark" ? <Sun size={14} /> : <Moon size={14} />}
        </button>
      </div>
    </header>
  );
}

interface HealthLightProps {
  label: string;
  value: number | null;
  fmt: (v: number) => string;
  warningAt?: number;
  dangerAt?: number;
  max?: number;
}

function HealthLight({ label, value, fmt, warningAt = 75, dangerAt = 90, max = 100 }: HealthLightProps) {
  let tone = "bg-text-tertiary";
  let display = "—";
  if (value !== null && !Number.isNaN(value)) {
    const pct = max > 0 ? (value / max) * 100 : value;
    tone = pct >= dangerAt ? "bg-danger" : pct >= warningAt ? "bg-warning" : "bg-success";
    display = fmt(value);
  }
  return (
    <div className="flex items-center gap-1.5 text-[11px] font-mono">
      <span className={cn("size-2 rounded-full", tone)} />
      <span className="text-text-tertiary uppercase tracking-wider">{label}</span>
      <span className="text-text-secondary tabular-nums">{display}</span>
    </div>
  );
}
