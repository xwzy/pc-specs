import { useQuery } from "@tanstack/react-query";
import {
  Cpu,
  HardDrive,
  MemoryStick,
  MonitorPlay,
  Network as NetIcon,
  Thermometer,
  Server,
  Sparkles,
  Monitor,
  BatteryFull,
  type LucideIcon,
} from "lucide-react";
import { useMemo } from "react";
import { getFullSnapshot } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Stat } from "@/components/ui/Stat";
import { Bar } from "@/components/ui/Bar";
import { Badge } from "@/components/ui/Badge";
import { Spark } from "@/components/charts/Spark";
import { fmtHz, fmtPercent, fmtUptime, nullable, useFmt } from "@/lib/format";
import { useMonitor } from "@/lib/useMonitor";
import { PageHeader } from "@/components/layout/PageHeader";
import { useT } from "@/lib/store";
import type { DictKey } from "@/lib/i18n";
import type { SystemSnapshot } from "@/lib/types";

export default function Dashboard() {
  const t = useT();
  const fmt = useFmt();
  const { data: snap, isLoading } = useQuery({
    queryKey: ["full-snapshot"],
    queryFn: getFullSnapshot,
    refetchOnMount: true,
  });
  const { ticks, latest } = useMonitor(true);

  const cpuSpark = useMemo(() => ticks.map((t) => t.cpu_overall), [ticks]);
  const memSpark = useMemo(
    () =>
      ticks.map((t) =>
        t.mem_total_bytes > 0 ? (t.mem_used_bytes / t.mem_total_bytes) * 100 : 0,
      ),
    [ticks],
  );
  const netSpark = useMemo(
    () => ticks.map((t) => (t.net_rx_bps + t.net_tx_bps) / 1024 / 1024),
    [ticks],
  );

  if (isLoading || !snap) {
    return <SkeletonDashboard />;
  }

  const memPct =
    snap.memory.total_bytes > 0
      ? (snap.memory.used_bytes / snap.memory.total_bytes) * 100
      : 0;
  const primaryStorage = snap.storages[0];
  const storagePct =
    primaryStorage && primaryStorage.total_bytes > 0
      ? (primaryStorage.used_bytes / primaryStorage.total_bytes) * 100
      : 0;
  const primaryNet =
    snap.network.interfaces.find((i) => !i.is_loopback && i.ipv4.length > 0) ??
    snap.network.interfaces[0];

  return (
    <div className="space-y-5">
      <CoverCard snap={snap} />

      <SpecsSummary snap={snap} />

      <div className="grid grid-cols-12 gap-4">
        <Card
          className="col-span-12 md:col-span-4 relative overflow-hidden"
          title={
            <>
              <Cpu size={14} /> {t("dash_card_cpu")}
            </>
          }
          action={<Badge tone="accent">{snap.cpu.arch}</Badge>}
        >
          <Stat
            label={snap.cpu.brand}
            value={fmtPercent(latest?.cpu_overall ?? snap.cpu.usage_overall, 1)}
            sub={`${snap.cpu.physical_cores}P / ${snap.cpu.logical_cores}L · ${fmtHz(snap.cpu.current_frequency_hz)}`}
          />
          <div className="mt-4 -mx-1">
            <Spark data={cpuSpark} width={400} height={36} className="w-full" />
          </div>
        </Card>

        <Card
          className="col-span-12 md:col-span-4"
          title={
            <>
              <MonitorPlay size={14} /> {t("dash_card_gpu")}
            </>
          }
          action={snap.gpus[0] ? <Badge>{snap.gpus[0].backend}</Badge> : null}
        >
          {snap.gpus.length === 0 ? (
            <Stat label="GPU" value="—" sub={t("dash_no_gpu")} />
          ) : (
            <Stat
              label={snap.gpus[0].vendor}
              value={snap.gpus[0].name}
              sub={`${snap.gpus[0].is_discrete ? t("dash_gpu_discrete") : t("dash_gpu_integrated")} · ${nullable(snap.gpus[0].driver, t("dash_gpu_no_driver"))}`}
            />
          )}
        </Card>

        <Card
          className="col-span-12 md:col-span-4"
          title={
            <>
              <MemoryStick size={14} /> {t("dash_card_memory")}
            </>
          }
        >
          <Stat
            label={t("dash_mem_usage")}
            value={fmtPercent(memPct, 0)}
            sub={`${fmt.bytes(snap.memory.used_bytes)} / ${fmt.bytes(snap.memory.total_bytes)}`}
          />
          <div className="mt-3">
            <Bar value={memPct} />
          </div>
          <div className="mt-3 -mx-1">
            <Spark data={memSpark} width={400} height={32} className="w-full" />
          </div>
        </Card>
      </div>

      <div className="grid grid-cols-12 gap-4">
        <Card
          className="col-span-12 md:col-span-6"
          title={
            <>
              <HardDrive size={14} /> {t("dash_card_storage")}
            </>
          }
        >
          {primaryStorage ? (
            <>
              <Stat
                label={primaryStorage.kind + " · " + (primaryStorage.filesystem ?? "—")}
                value={primaryStorage.name}
                sub={`${fmt.bytes(primaryStorage.used_bytes)} / ${fmt.bytes(primaryStorage.total_bytes)}`}
              />
              <div className="mt-3">
                <Bar value={storagePct} />
              </div>
              <div className="mt-3 grid grid-cols-2 gap-2 text-xs text-text-secondary font-mono">
                <span>
                  {t("dash_io_read")} {fmt.netSpeed(primaryStorage.read_bytes_per_sec)}
                </span>
                <span>
                  {t("dash_io_write")} {fmt.netSpeed(primaryStorage.write_bytes_per_sec)}
                </span>
              </div>
            </>
          ) : (
            <Stat label={t("dash_card_storage")} value="—" sub={t("dash_no_disk")} />
          )}
        </Card>

        <Card
          className="col-span-12 md:col-span-6"
          title={
            <>
              <NetIcon size={14} /> {t("dash_card_network")}
            </>
          }
        >
          {primaryNet ? (
            <>
              <Stat
                label={primaryNet.kind.toUpperCase()}
                value={primaryNet.name}
                sub={primaryNet.ipv4.join(", ") || "—"}
              />
              <div className="mt-3 grid grid-cols-2 gap-2 text-xs text-text-secondary font-mono">
                <span>↓ {fmt.netSpeed(latest?.net_rx_bps ?? primaryNet.rx_bytes_per_sec)}</span>
                <span>↑ {fmt.netSpeed(latest?.net_tx_bps ?? primaryNet.tx_bytes_per_sec)}</span>
              </div>
              <div className="mt-3 -mx-1">
                <Spark data={netSpark} width={400} height={32} className="w-full" />
              </div>
            </>
          ) : (
            <Stat label={t("dash_card_network")} value="—" sub={t("dash_no_net")} />
          )}
        </Card>
      </div>

      <Card
        title={
          <>
            <Thermometer size={14} /> {t("dash_card_sensors")}
          </>
        }
      >
        {snap.sensors.length === 0 ? (
          <div className="text-text-tertiary text-sm">{t("dash_no_sensors")}</div>
        ) : (
          <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-3">
            {snap.sensors.slice(0, 12).map((s, i) => (
              <div
                key={i}
                className="px-3 py-2 rounded-lg bg-bg-elevated/40 border border-border/50 flex items-center justify-between gap-2"
              >
                <div className="min-w-0">
                  <div className="text-text-tertiary text-[10px] uppercase tracking-wider truncate">
                    {s.source}
                  </div>
                  <div className="text-text-secondary text-xs truncate font-mono">{s.label}</div>
                </div>
                <div className="font-mono text-text-primary text-sm tabular-nums whitespace-nowrap">
                  {s.kind === "temperature" ? fmt.temp(s.value) : `${s.value.toFixed(0)} ${s.unit}`}
                </div>
              </div>
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}

interface CoverCardProps {
  snap: SystemSnapshot;
}

function CoverCard({ snap }: CoverCardProps) {
  const t = useT();
  const fmt = useFmt();
  const memPct =
    snap.memory.total_bytes > 0
      ? (snap.memory.used_bytes / snap.memory.total_bytes) * 100
      : 0;
  const primary = snap.storages[0];
  const storagePct =
    primary && primary.total_bytes > 0
      ? (primary.used_bytes / primary.total_bytes) * 100
      : 0;
  const maxTemp = snap.sensors
    .filter((s) => s.kind === "temperature")
    .reduce((m, s) => Math.max(m, s.value), 0);
  const cpuPct = snap.cpu.usage_overall;

  type IssueTone = "success" | "warning" | "danger";
  const issues: Array<{ tone: IssueTone; level: number; text: string }> = [];
  const push = (cond: boolean, tone: "warning" | "danger", text: string) => {
    if (cond) issues.push({ tone, level: tone === "danger" ? 2 : 1, text });
  };
  push(cpuPct >= 95, "danger", t("dash_alert_cpu_overload"));
  push(memPct >= 95, "danger", t("dash_alert_mem_full"));
  push(memPct >= 85 && memPct < 95, "warning", t("dash_alert_mem_high"));
  push(storagePct >= 95, "danger", t("dash_alert_disk_full"));
  push(storagePct >= 85 && storagePct < 95, "warning", t("dash_alert_disk_high"));
  push(maxTemp >= 88, "danger", `${t("dash_alert_temp")} ${fmt.temp(maxTemp)}`);
  push(maxTemp >= 75 && maxTemp < 88, "warning", `${t("dash_alert_temp")} ${fmt.temp(maxTemp)}`);

  const overall = issues.reduce((m, i) => Math.max(m, i.level), 0);
  const overallBadge =
    overall === 2 ? (
      <Badge tone="danger">{t("dash_health_critical")}</Badge>
    ) : overall === 1 ? (
      <Badge tone="warning">{t("dash_health_warning")}</Badge>
    ) : (
      <Badge tone="success">{t("dash_health_healthy")}</Badge>
    );

  const familyInitial = snap.os.family.charAt(0).toUpperCase() || "?";

  return (
    <div className="relative rounded-card border border-border bg-gradient-to-br from-bg-surface to-bg-base p-6 overflow-hidden">
      <div className="absolute inset-0 pointer-events-none opacity-50">
        <div className="absolute -top-24 -right-24 size-72 rounded-full bg-accent/10 blur-3xl" />
        <div className="absolute -bottom-24 -left-24 size-72 rounded-full bg-accent-2/10 blur-3xl" />
      </div>
      <div className="relative">
        <div className="flex flex-wrap items-center gap-3">
          <div className="size-12 rounded-xl bg-gradient-to-br from-accent to-accent-2 flex items-center justify-center text-bg-base font-bold text-xl">
            {familyInitial}
          </div>
          <div>
            <div className="text-text-primary text-2xl font-semibold tracking-tight">
              {snap.host.hostname}
            </div>
            <div className="text-text-secondary text-sm font-mono">
              {snap.os.name} · {snap.os.version} · {snap.os.arch} · up {fmtUptime(snap.host.uptime_secs)}
            </div>
          </div>
          <div className="ml-auto flex items-center gap-2 flex-wrap justify-end">
            {overallBadge}
            {issues.slice(0, 3).map((i, idx) => (
              <Badge key={idx} tone={i.tone}>
                {i.text}
              </Badge>
            ))}
            <Badge>{snap.cpu.brand}</Badge>
          </div>
        </div>
      </div>
    </div>
  );
}

interface SpecsSummaryProps {
  snap: SystemSnapshot;
}

interface SpecRow {
  icon: LucideIcon;
  labelKey: DictKey;
  primary: string;
  secondary?: string | null;
}

/// 顶部硬件摘要卡片：把 CPU / 内存 / GPU / 主硬盘 / 主板 / OS / 显示 / 网络 / 电池
/// 等关键 spec 一屏展示。响应式：lg 4 列、md 2 列、mobile 1 列。
function SpecsSummary({ snap }: SpecsSummaryProps) {
  const t = useT();
  const fmt = useFmt();

  const cpuTopo = snap.cpu.topology;
  const cpuFreq = snap.cpu.max_frequency_hz || snap.cpu.current_frequency_hz;
  const cpuTopoLabel = (() => {
    if (cpuTopo && (cpuTopo.p_cores ?? 0) > 0 && (cpuTopo.e_cores ?? 0) > 0) {
      return `${cpuTopo.p_cores}P · ${cpuTopo.e_cores}E`;
    }
    return `${snap.cpu.physical_cores}C / ${snap.cpu.logical_cores}T`;
  })();

  const memModule = snap.memory.modules.find((m) => m.kind || m.speed_mt_s);
  const memDetail = (() => {
    const parts: string[] = [];
    if (memModule?.kind) parts.push(memModule.kind);
    if (memModule?.speed_mt_s) parts.push(`${memModule.speed_mt_s} MT/s`);
    if (snap.memory.modules.length > 0) parts.push(`× ${snap.memory.modules.length}`);
    return parts.join(" · ") || null;
  })();

  const gpu = snap.gpus[0];
  const gpuDetail = (() => {
    if (!gpu) return null;
    const parts: string[] = [];
    parts.push(gpu.is_discrete ? t("dash_gpu_discrete") : t("dash_gpu_integrated"));
    if (gpu.backend) parts.push(gpu.backend);
    if (gpu.vram_total_bytes) parts.push(fmt.bytes(gpu.vram_total_bytes));
    return parts.join(" · ");
  })();

  const storage = snap.storages[0];
  const storageDetail = storage
    ? `${storage.kind} · ${fmt.bytes(storage.total_bytes)}`
    : null;

  const mb = snap.motherboard;
  const mbPrimary = mb
    ? [mb.vendor, mb.model].filter(Boolean).join(" ").trim() || "—"
    : "—";
  const mbDetail = mb
    ? [mb.bios_vendor, mb.bios_version].filter(Boolean).join(" ") || null
    : null;

  const display = snap.displays.find((d) => d.is_primary) ?? snap.displays[0];
  const displayPrimary = display
    ? `${display.width_px}×${display.height_px}`
    : "—";
  const displayDetail = (() => {
    if (!display) return null;
    const parts: string[] = [];
    if (display.refresh_hz) parts.push(`${display.refresh_hz.toFixed(0)} Hz`);
    if (display.scale_factor && display.scale_factor !== 1)
      parts.push(`${display.scale_factor.toFixed(2)}×`);
    return parts.join(" · ") || null;
  })();

  const primaryNet =
    snap.network.interfaces.find((i) => !i.is_loopback && i.ipv4.length > 0) ??
    snap.network.interfaces.find((i) => !i.is_loopback);
  const netPrimary = primaryNet?.name ?? "—";
  const netDetail = (() => {
    if (!primaryNet) return null;
    const parts: string[] = [];
    if (primaryNet.ipv4[0]) parts.push(primaryNet.ipv4[0]);
    if (primaryNet.link_speed_mbps)
      parts.push(`${primaryNet.link_speed_mbps} Mbps`);
    return parts.join(" · ") || null;
  })();

  const battery = snap.battery;
  const batteryPrimary = battery ? `${battery.percentage.toFixed(0)}%` : null;
  const batteryDetail = battery
    ? [battery.state, battery.cycle_count != null ? `${battery.cycle_count} cycles` : null]
        .filter(Boolean)
        .join(" · ")
    : null;

  const rows: SpecRow[] = [
    {
      icon: Cpu,
      labelKey: "dash_summary_cpu",
      primary: snap.cpu.brand,
      secondary: `${cpuTopoLabel} · ${snap.cpu.arch}${cpuFreq ? ` · ${fmtHz(cpuFreq)}` : ""}`,
    },
    {
      icon: MemoryStick,
      labelKey: "dash_summary_memory",
      primary: fmt.bytes(snap.memory.total_bytes),
      secondary: memDetail,
    },
    {
      icon: MonitorPlay,
      labelKey: "dash_summary_gpu",
      primary: gpu ? `${gpu.vendor} · ${gpu.name}` : t("dash_no_gpu"),
      secondary: gpuDetail,
    },
    {
      icon: HardDrive,
      labelKey: "dash_summary_storage",
      primary: storage ? storage.name : t("dash_no_disk"),
      secondary: storageDetail,
    },
    {
      icon: Server,
      labelKey: "dash_summary_motherboard",
      primary: mbPrimary,
      secondary: mbDetail,
    },
    {
      icon: Sparkles,
      labelKey: "dash_summary_os",
      primary: `${snap.os.name} ${snap.os.version}`.trim(),
      secondary: `${snap.os.arch}${snap.os.kernel ? ` · ${snap.os.kernel}` : ""}`,
    },
    {
      icon: Monitor,
      labelKey: "dash_summary_display",
      primary: displayPrimary,
      secondary: displayDetail,
    },
    {
      icon: NetIcon,
      labelKey: "dash_summary_network",
      primary: netPrimary,
      secondary: netDetail,
    },
  ];

  if (battery && batteryPrimary) {
    rows.push({
      icon: BatteryFull,
      labelKey: "dash_summary_battery",
      primary: batteryPrimary,
      secondary: batteryDetail,
    });
  }

  return (
    <Card title={t("dash_summary_title")} action={<span className="text-text-tertiary text-xs">{t("dash_summary_desc")}</span>}>
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
        {rows.map((row) => (
          <SpecCell key={row.labelKey} row={row} label={t(row.labelKey)} />
        ))}
      </div>
    </Card>
  );
}

function SpecCell({ row, label }: { row: SpecRow; label: string }) {
  const Icon = row.icon;
  return (
    <div className="px-3 py-2.5 rounded-lg bg-bg-elevated/40 border border-border/50 flex items-start gap-3 min-w-0">
      <div className="size-8 shrink-0 rounded-md bg-bg-base/60 border border-border/60 flex items-center justify-center text-text-secondary">
        <Icon size={15} />
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-text-tertiary text-[10px] uppercase tracking-widest">
          {label}
        </div>
        <div
          className="text-text-primary text-sm font-medium truncate"
          title={row.primary}
        >
          {row.primary || "—"}
        </div>
        {row.secondary ? (
          <div className="text-text-secondary text-xs font-mono truncate" title={row.secondary}>
            {row.secondary}
          </div>
        ) : null}
      </div>
    </div>
  );
}

function SkeletonDashboard() {
  return (
    <div className="space-y-5">
      <PageHeader title="Loading..." />
      <div className="grid grid-cols-12 gap-4">
        {Array.from({ length: 6 }).map((_, i) => (
          <div
            key={i}
            className="col-span-12 md:col-span-4 h-32 rounded-card bg-bg-surface border border-border animate-pulse"
          />
        ))}
      </div>
    </div>
  );
}
