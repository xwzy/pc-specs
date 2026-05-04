import { useMemo, useState } from "react";
import { Pause, Play, Trash2 } from "lucide-react";
import { Card } from "@/components/ui/Card";
import { LiveLineChart } from "@/components/charts/LiveLineChart";
import { CoreBars } from "@/components/charts/CoreBars";
import { Stat } from "@/components/ui/Stat";
import { fmtPercent, useFmt } from "@/lib/format";
import { useMonitor } from "@/lib/useMonitor";
import { useSettings, useT, useTWith } from "@/lib/store";
import { useMonitorStore } from "@/lib/monitorStore";
import { PageHeader } from "@/components/layout/PageHeader";

const INTERVALS = [
  { label: "500ms", value: 500 },
  { label: "1s", value: 1000 },
  { label: "2s", value: 2000 },
  { label: "5s", value: 5000 },
];

const WINDOWS = [
  { label: "1m", seconds: 60 },
  { label: "5m", seconds: 300 },
  { label: "10m", seconds: 600 },
];

export default function MonitorPage() {
  const t = useT();
  const tw = useTWith();
  const fmt = useFmt();
  const { ticks, latest } = useMonitor(true);
  const interval = useSettings((s) => s.monitorIntervalMs);
  const setMonitorInterval = useSettings((s) => s.setMonitorInterval);

  const paused = useMonitorStore((s) => s.paused);
  const setPaused = useMonitorStore((s) => s.setPaused);
  const reset = useMonitorStore((s) => s.reset);

  const [windowSec, setWindowSec] = useState<number>(300);

  const visibleTicks = useMemo(() => {
    if (ticks.length === 0) return ticks;
    const cutoff = Date.now() - windowSec * 1000;
    return ticks.filter((tick) => tick.timestamp >= cutoff);
  }, [ticks, windowSec]);

  const cpuData = useMemo(
    () => visibleTicks.map((tick) => ({ t: tick.timestamp, overall: tick.cpu_overall })),
    [visibleTicks],
  );
  const memData = useMemo(
    () =>
      visibleTicks.map((tick) => ({
        t: tick.timestamp,
        used:
          tick.mem_total_bytes > 0
            ? (tick.mem_used_bytes / tick.mem_total_bytes) * 100
            : 0,
      })),
    [visibleTicks],
  );
  const netData = useMemo(
    () =>
      visibleTicks.map((tick) => {
        const factor = fmt.netSpeedUnit === "bit" ? 8 / 1_000_000 : 1 / 1024;
        return {
          t: tick.timestamp,
          rx: tick.net_rx_bps * factor,
          tx: tick.net_tx_bps * factor,
        };
      }),
    [visibleTicks, fmt.netSpeedUnit],
  );
  const diskData = useMemo(
    () =>
      visibleTicks.map((tick) => {
        const factor = fmt.byteUnit === "binary" ? 1 / 1024 / 1024 : 1 / 1_000_000;
        return {
          t: tick.timestamp,
          read: tick.disk_read_bps * factor,
          write: tick.disk_write_bps * factor,
        };
      }),
    [visibleTicks, fmt.byteUnit],
  );
  const tempData = useMemo(() => {
    const map: Array<Record<string, number>> = [];
    visibleTicks.forEach((tick) => {
      const row: Record<string, number> = { t: tick.timestamp };
      tick.temperatures.forEach((s) => {
        const v = fmt.tempUnit === "F" ? (s.value * 9) / 5 + 32 : s.value;
        row[s.label] = v;
      });
      map.push(row);
    });
    return map;
  }, [visibleTicks, fmt.tempUnit]);

  const tempSeries = useMemo(() => {
    const labels = new Set<string>();
    visibleTicks.forEach((tick) => tick.temperatures.forEach((s) => labels.add(s.label)));
    return Array.from(labels).map((l) => ({ key: l, label: l }));
  }, [visibleTicks]);

  const netSpeedUnitLabel = fmt.netSpeedUnit === "bit" ? "Mbps" : "KiB/s";
  const diskUnitLabel = fmt.byteUnit === "binary" ? "MiB/s" : "MB/s";
  const tempUnitLabel = fmt.tempUnit === "F" ? "°F" : "°C";

  return (
    <div className="space-y-5">
      <PageHeader
        title={t("monitor_title")}
        description={
          <span className="font-mono text-xs">
            {tw("monitor_pts", { visible: visibleTicks.length, total: ticks.length })} ·{" "}
            {tw("monitor_per_sample", { ms: interval })} ·{" "}
            {tw("monitor_window", { min: windowSec / 60 })}
            {paused && <span className="ml-2 text-warning">{t("monitor_paused")}</span>}
          </span>
        }
        actions={
          <div className="flex items-center gap-2">
            <div className="flex items-center gap-1 bg-bg-surface border border-border rounded-md p-0.5">
              {WINDOWS.map((w) => (
                <button
                  key={w.label}
                  type="button"
                  onClick={() => setWindowSec(w.seconds)}
                  className={
                    "px-2.5 py-1 text-xs rounded-sm font-mono " +
                    (windowSec === w.seconds
                      ? "bg-accent/15 text-accent"
                      : "text-text-secondary hover:text-text-primary")
                  }
                >
                  {w.label}
                </button>
              ))}
            </div>
            <div className="flex items-center gap-1 bg-bg-surface border border-border rounded-md p-0.5">
              {INTERVALS.map((opt) => (
                <button
                  key={opt.value}
                  type="button"
                  onClick={() => setMonitorInterval(opt.value)}
                  className={
                    "px-2.5 py-1 text-xs rounded-sm font-mono " +
                    (opt.value === interval
                      ? "bg-accent/15 text-accent"
                      : "text-text-secondary hover:text-text-primary")
                  }
                >
                  {opt.label}
                </button>
              ))}
            </div>
            <button
              type="button"
              onClick={() => setPaused(!paused)}
              className={
                "flex items-center gap-1.5 px-3 py-1.5 rounded-md border text-xs font-mono " +
                (paused
                  ? "bg-warning/10 border-warning/40 text-warning"
                  : "bg-bg-surface border-border text-text-secondary hover:text-text-primary")
              }
              title={paused ? t("monitor_resume") : t("monitor_pause")}
            >
              {paused ? <Play size={12} /> : <Pause size={12} />}
              {paused ? t("monitor_resume") : t("monitor_pause")}
            </button>
            <button
              type="button"
              onClick={reset}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-md border border-border bg-bg-surface text-xs text-text-secondary hover:text-text-primary"
              title={t("monitor_clear")}
            >
              <Trash2 size={12} /> {t("monitor_clear")}
            </button>
          </div>
        }
      />

      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <Card>
          <Stat label={t("dash_card_cpu")} value={fmtPercent(latest?.cpu_overall ?? 0, 1)} />
        </Card>
        <Card>
          <Stat
            label={t("dash_card_memory")}
            value={
              latest && latest.mem_total_bytes > 0
                ? fmtPercent((latest.mem_used_bytes / latest.mem_total_bytes) * 100, 0)
                : "—"
            }
            sub={latest ? fmt.bytes(latest.mem_used_bytes) : ""}
          />
        </Card>
        <Card>
          <Stat
            label={t("dash_card_network")}
            value={latest ? fmt.netSpeed(latest.net_rx_bps + latest.net_tx_bps) : "—"}
          />
        </Card>
        <Card>
          <Stat
            label={t("monitor_hottest")}
            value={
              latest && latest.temperatures.length > 0
                ? fmt.temp(Math.max(...latest.temperatures.map((s) => s.value)))
                : "—"
            }
          />
        </Card>
      </div>

      <Card title={t("cpu_usage_live")}>
        <LiveLineChart
          data={cpuData}
          series={[{ key: "overall", label: "CPU %" }]}
          height={220}
          yDomain={[0, 100]}
          yFormatter={(v) => `${v.toFixed(0)}%`}
        />
      </Card>

      <Card title={t("monitor_per_core_latest")}>
        {latest && latest.cpu_per_core.length > 0 ? (
          <CoreBars values={latest.cpu_per_core} />
        ) : (
          <div className="text-text-tertiary text-sm">{t("monitor_no_data")}</div>
        )}
      </Card>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <Card title={t("monitor_mem_pct")}>
          <LiveLineChart
            data={memData}
            series={[{ key: "used", label: "Memory %" }]}
            height={200}
            yDomain={[0, 100]}
            yFormatter={(v) => `${v.toFixed(0)}%`}
          />
        </Card>
        <Card title={`${t("dash_card_network")} (${netSpeedUnitLabel})`}>
          <LiveLineChart
            data={netData}
            series={[
              { key: "rx", label: "RX" },
              { key: "tx", label: "TX", color: "rgb(var(--accent-2))" },
            ]}
            height={200}
            showLegend
            yFormatter={(v) => v.toFixed(fmt.netSpeedUnit === "bit" ? 1 : 0)}
          />
        </Card>
      </div>

      <Card title={tw("monitor_disk_io", { unit: diskUnitLabel })}>
        <LiveLineChart
          data={diskData}
          series={[
            { key: "read", label: "Read" },
            { key: "write", label: "Write", color: "rgb(var(--accent-2))" },
          ]}
          height={200}
          showLegend
          yFormatter={(v) => v.toFixed(2)}
        />
      </Card>

      <Card title={tw("monitor_temp_unit", { unit: tempUnitLabel })}>
        {tempSeries.length === 0 ? (
          <div className="text-text-tertiary text-sm">{t("monitor_no_temp")}</div>
        ) : (
          <LiveLineChart
            data={tempData}
            series={tempSeries}
            height={220}
            showLegend
            yFormatter={(v) => `${v.toFixed(0)}${tempUnitLabel}`}
          />
        )}
      </Card>
    </div>
  );
}
