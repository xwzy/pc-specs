import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { getMemory } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Stat } from "@/components/ui/Stat";
import { Bar } from "@/components/ui/Bar";
import { RingProgress } from "@/components/ui/RingProgress";
import { Empty } from "@/components/ui/Empty";
import { LiveLineChart } from "@/components/charts/LiveLineChart";
import { fmtPercent, nullable, useFmt } from "@/lib/format";
import { useMonitor } from "@/lib/useMonitor";
import { PageHeader } from "@/components/layout/PageHeader";
import { useT } from "@/lib/store";

export default function MemoryPage() {
  const t = useT();
  const fmt = useFmt();
  const { data: mem } = useQuery({
    queryKey: ["memory"],
    queryFn: getMemory,
    refetchInterval: 5_000,
  });
  const { ticks } = useMonitor(true);

  const data = useMemo(
    () =>
      ticks.map((tick) => ({
        t: tick.timestamp,
        used:
          tick.mem_total_bytes > 0
            ? (tick.mem_used_bytes / tick.mem_total_bytes) * 100
            : 0,
      })),
    [ticks],
  );

  if (!mem) return null;
  const usedPct = mem.total_bytes > 0 ? (mem.used_bytes / mem.total_bytes) * 100 : 0;
  const swapPct =
    mem.swap_total_bytes > 0 ? (mem.swap_used_bytes / mem.swap_total_bytes) * 100 : 0;

  return (
    <div className="space-y-5">
      <PageHeader
        title={t("nav_memory")}
        description={`${fmt.bytes(mem.total_bytes)} ${t("mem_total_suffix")} · ${mem.modules.length} ${t("mem_modules_suffix")}`}
      />

      <Card>
        <div className="flex flex-wrap items-center gap-6">
          <RingProgress
            value={usedPct}
            label={fmtPercent(usedPct, 0)}
            sub={t("spec_used")}
            size={108}
          />
          <div className="grid grid-cols-2 md:grid-cols-3 gap-x-8 gap-y-3 flex-1">
            <Stat label={t("spec_total")} value={fmt.bytes(mem.total_bytes)} />
            <Stat label={t("spec_used")} value={fmt.bytes(mem.used_bytes)} />
            <Stat label={t("spec_available")} value={fmt.bytes(mem.available_bytes)} />
            <Stat label={t("mem_swap_total")} value={fmt.bytes(mem.swap_total_bytes)} />
            <Stat label={t("mem_swap_used")} value={fmt.bytes(mem.swap_used_bytes)} />
            <Stat label={t("mem_swap_pct")} value={fmtPercent(swapPct, 1)} />
          </div>
        </div>
        <div className="mt-4 space-y-2">
          <div className="flex items-center justify-between text-xs text-text-secondary">
            <span>RAM</span>
            <span className="font-mono">
              {fmt.bytes(mem.used_bytes)} / {fmt.bytes(mem.total_bytes)}
            </span>
          </div>
          <Bar value={usedPct} />
          <div className="flex items-center justify-between text-xs text-text-secondary mt-2">
            <span>SWAP</span>
            <span className="font-mono">
              {fmt.bytes(mem.swap_used_bytes)} / {fmt.bytes(mem.swap_total_bytes)}
            </span>
          </div>
          <Bar value={swapPct} warningAt={50} dangerAt={80} />
        </div>
      </Card>

      <Card title={t("mem_usage_live")}>
        <LiveLineChart
          data={data}
          series={[{ key: "used", label: "RAM %" }]}
          height={200}
          yDomain={[0, 100]}
          yFormatter={(v) => `${v.toFixed(0)}%`}
        />
      </Card>

      <Card title={t("mem_modules")}>
        {mem.modules.length === 0 ? (
          <Empty title={t("mem_no_modules")} hint={t("mem_no_modules_hint")} />
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm font-mono">
              <thead>
                <tr className="text-text-tertiary text-[11px] uppercase tracking-wider">
                  <th className="text-left py-2 pr-3">{t("mem_table_slot")}</th>
                  <th className="text-left py-2 pr-3">{t("mem_table_manufacturer")}</th>
                  <th className="text-left py-2 pr-3">{t("mem_table_part")}</th>
                  <th className="text-right py-2 pr-3">{t("mem_table_capacity")}</th>
                  <th className="text-right py-2 pr-3">{t("mem_table_speed")}</th>
                  <th className="text-left py-2 pr-3">{t("mem_table_type")}</th>
                  <th className="text-left py-2 pr-3">{t("mem_table_form")}</th>
                </tr>
              </thead>
              <tbody>
                {mem.modules.map((m, i) => (
                  <tr key={i} className="border-t border-border/40 text-text-primary">
                    <td className="py-2 pr-3">{m.slot}</td>
                    <td className="py-2 pr-3">{nullable(m.manufacturer)}</td>
                    <td className="py-2 pr-3">{nullable(m.part_number)}</td>
                    <td className="py-2 pr-3 text-right tabular-nums">
                      {fmt.bytes(m.capacity_bytes)}
                    </td>
                    <td className="py-2 pr-3 text-right tabular-nums">
                      {m.speed_mt_s ? `${m.speed_mt_s} MT/s` : "—"}
                    </td>
                    <td className="py-2 pr-3">{nullable(m.kind)}</td>
                    <td className="py-2 pr-3">{nullable(m.form_factor)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>
    </div>
  );
}
