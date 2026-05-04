import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { getCpu } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Stat } from "@/components/ui/Stat";
import { RingProgress } from "@/components/ui/RingProgress";
import { Badge } from "@/components/ui/Badge";
import { KeyValueTable } from "@/components/ui/KeyValueTable";
import { Section } from "@/components/ui/Section";
import { LiveLineChart } from "@/components/charts/LiveLineChart";
import { CoreBars } from "@/components/charts/CoreBars";
import { fmtBytes, fmtHz, fmtPercent, fmtTemp, nullable } from "@/lib/format";
import { useMonitor } from "@/lib/useMonitor";
import { useSettings, useT } from "@/lib/store";
import { PageHeader } from "@/components/layout/PageHeader";

export default function CpuPage() {
  const t = useT();
  const { data: cpu } = useQuery({ queryKey: ["cpu"], queryFn: getCpu, refetchInterval: 5_000 });
  const geek = useSettings((s) => s.geekMode);
  const { ticks, latest } = useMonitor(true);

  const chartData = useMemo(
    () => ticks.map((t) => ({ t: t.timestamp, overall: t.cpu_overall })),
    [ticks],
  );

  if (!cpu) return <PageSkeleton />;

  const usageOverall = latest?.cpu_overall ?? cpu.usage_overall;
  const perCore = latest?.cpu_per_core ?? cpu.usage_per_core;

  return (
    <div className="space-y-5">
      <PageHeader
        title={t("nav_cpu")}
        description={cpu.brand}
        actions={
          <>
            <Badge tone="accent">{cpu.arch}</Badge>
            {cpu.topology?.p_cores != null && <Badge>P×{cpu.topology.p_cores}</Badge>}
            {cpu.topology?.e_cores != null && <Badge>E×{cpu.topology.e_cores}</Badge>}
            {cpu.virtualization && <Badge tone="success">{t("cpu_virtualization")}</Badge>}
          </>
        }
      />

      <Card>
        <div className="flex flex-wrap items-center gap-6">
          <RingProgress
            value={usageOverall}
            label={`${usageOverall.toFixed(1)}%`}
            sub={t("cpu_overall")}
            size={108}
          />
          <div className="grid grid-cols-2 md:grid-cols-3 gap-x-8 gap-y-3 flex-1">
            <Stat label={t("spec_brand")} value={cpu.brand} />
            <Stat label={t("spec_vendor")} value={cpu.vendor || "—"} />
            <Stat
              label={t("spec_cores")}
              value={`${cpu.physical_cores} / ${cpu.logical_cores}`}
              sub="physical / logical"
            />
            <Stat
              label={t("spec_freq")}
              value={fmtHz(cpu.current_frequency_hz)}
              sub={`${t("spec_freq_max")} ${fmtHz(cpu.max_frequency_hz)}`}
            />
            <Stat label={t("cpu_cache_l2")} value={fmtBytes(cpu.cache_l2_bytes)} />
            <Stat label={t("cpu_cache_l3")} value={fmtBytes(cpu.cache_l3_bytes)} />
          </div>
        </div>
      </Card>

      <Card title={t("cpu_per_core")}>
        <CoreBars values={perCore} />
      </Card>

      <Card
        title={t("cpu_usage_live")}
        action={<span className="text-xs text-text-tertiary">{ticks.length} pts</span>}
      >
        <LiveLineChart
          data={chartData}
          series={[{ key: "overall", label: "CPU %" }]}
          height={200}
          yFormatter={(v) => `${v.toFixed(0)}%`}
          yDomain={[0, 100]}
        />
      </Card>

      <Card title={t("cpu_details")}>
        <KeyValueTable
          rows={[
            { key: t("spec_vendor"), value: cpu.vendor || "—" },
            { key: t("spec_brand"), value: cpu.brand },
            { key: t("spec_arch"), value: cpu.arch },
            {
              key: t("spec_cores"),
              value: `${cpu.physical_cores} physical / ${cpu.logical_cores} logical`,
            },
            {
              key: t("spec_freq"),
              value: `${fmtHz(cpu.current_frequency_hz)} (max ${fmtHz(cpu.max_frequency_hz)})`,
            },
            { key: t("cpu_cache_l1"), value: fmtBytes(cpu.cache_l1_bytes) },
            { key: t("cpu_cache_l2"), value: fmtBytes(cpu.cache_l2_bytes) },
            { key: t("cpu_cache_l3"), value: fmtBytes(cpu.cache_l3_bytes) },
            {
              key: t("cpu_virtualization"),
              value:
                cpu.virtualization === null
                  ? "—"
                  : cpu.virtualization
                    ? t("common_yes")
                    : t("common_no"),
            },
            { key: t("spec_temp"), value: fmtTemp(cpu.temperature_c) },
            { key: t("cpu_overall"), value: fmtPercent(usageOverall) },
            {
              key: t("cpu_topology"),
              value: nullable(
                cpu.topology
                  ? `${cpu.topology.sockets} socket · ${cpu.topology.numa_nodes} numa`
                  : null,
              ),
            },
          ]}
        />
      </Card>

      {(geek || cpu.features.length > 0) && (
        <Section title={t("cpu_geek_panel")} defaultOpen={geek}>
          <KeyValueTable
            rows={[
              { key: t("cpu_sockets"), value: String(cpu.topology?.sockets ?? 1) },
              { key: t("cpu_p_cores"), value: nullable(cpu.topology?.p_cores) },
              { key: t("cpu_e_cores"), value: nullable(cpu.topology?.e_cores) },
              { key: t("cpu_numa"), value: String(cpu.topology?.numa_nodes ?? 1) },
              { key: t("spec_vendor"), value: cpu.vendor || "—" },
              { key: t("cpu_isa_features"), value: `${cpu.features.length} flags` },
            ]}
          />
          {cpu.features.length === 0 ? (
            <div className="text-text-tertiary text-sm mt-3">{t("cpu_no_features")}</div>
          ) : (
            <div className="mt-3 flex flex-wrap gap-1.5">
              {cpu.features.map((f) => (
                <Badge key={f}>{f}</Badge>
              ))}
            </div>
          )}
        </Section>
      )}
    </div>
  );
}

function PageSkeleton() {
  return (
    <div className="space-y-4">
      <div className="h-32 rounded-card bg-bg-surface border border-border animate-pulse" />
      <div className="h-48 rounded-card bg-bg-surface border border-border animate-pulse" />
    </div>
  );
}
