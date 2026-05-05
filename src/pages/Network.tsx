import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Globe } from "lucide-react";
import { getNetwork, getPublicIp } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { KeyValueTable } from "@/components/ui/KeyValueTable";
import { Empty } from "@/components/ui/Empty";
import { LiveLineChart } from "@/components/charts/LiveLineChart";
import { nullable, useFmt } from "@/lib/format";
import { useMonitor } from "@/lib/useMonitor";
import { PageHeader } from "@/components/layout/PageHeader";
import { useSettings, useT } from "@/lib/store";

export default function NetworkPage() {
  const t = useT();
  const fmt = useFmt();
  const { data: net } = useQuery({
    queryKey: ["network"],
    queryFn: getNetwork,
    refetchInterval: 5_000,
  });
  const { ticks, latest } = useMonitor(true);
  const liveByName = useMemo(() => {
    const m = new Map<string, { rx: number; tx: number }>();
    latest?.per_interface.forEach((p) => m.set(p.name, { rx: p.rx_bps, tx: p.tx_bps }));
    return m;
  }, [latest]);
  const publicIpEnabled = useSettings((s) => s.publicIpEnabled);
  const publicIp = useSettings((s) => s.publicIp);
  const setPublicIp = useSettings((s) => s.setPublicIp);
  const [fetching, setFetching] = useState(false);

  const fetchIp = async () => {
    setFetching(true);
    try {
      const ip = await getPublicIp();
      setPublicIp(ip);
    } finally {
      setFetching(false);
    }
  };
  const speedFactor = fmt.netSpeedUnit === "bit" ? 8 / 1_000_000 : 1 / 1024;
  const speedUnitLabel = fmt.netSpeedUnit === "bit" ? "Mbps" : "KiB/s";
  const data = useMemo(
    () =>
      ticks.map((tick) => ({
        t: tick.timestamp,
        rx: tick.net_rx_bps * speedFactor,
        tx: tick.net_tx_bps * speedFactor,
      })),
    [ticks, speedFactor],
  );

  if (!net) return null;

  return (
    <div className="space-y-5">
      <PageHeader
        title={t("nav_network")}
        description={
          <span className="flex items-center gap-3 text-xs font-mono">
            <span>
              {net.interfaces.length} {t("net_count_suffix")}
            </span>
            {net.default_gateway && (
              <span className="text-text-tertiary">
                {t("net_gw_short")} {net.default_gateway}
              </span>
            )}
            {net.dns_servers.length > 0 && (
              <span className="text-text-tertiary">
                {t("net_dns_short")} {net.dns_servers.slice(0, 2).join(", ")}
              </span>
            )}
          </span>
        }
        actions={
          publicIpEnabled ? (
            <button
              type="button"
              onClick={fetchIp}
              disabled={fetching}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-md border border-accent/40 bg-accent/10 text-accent text-xs disabled:opacity-50"
            >
              <Globe size={12} />
              {fetching
                ? t("net_query_loading")
                : publicIp
                  ? `${t("net_public_prefix")} ${publicIp}`
                  : t("net_query_public")}
            </button>
          ) : null
        }
      />

      <Card title={`${t("net_throughput")} (${speedUnitLabel})`}>
        <LiveLineChart
          data={data}
          series={[
            { key: "rx", label: `RX ${speedUnitLabel}` },
            { key: "tx", label: `TX ${speedUnitLabel}`, color: "rgb(var(--accent-2))" },
          ]}
          height={200}
          showLegend
          yFormatter={(v) => v.toFixed(fmt.netSpeedUnit === "bit" ? 1 : 0)}
        />
      </Card>

      {net.interfaces.length === 0 ? (
        <Card>
          <Empty title={t("net_no_interfaces")} />
        </Card>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          {net.interfaces.map((n) => {
            const live = liveByName.get(n.name);
            return (
              <Card
                key={n.name}
                title={
                  <span className="flex items-center gap-2">
                    <span className="font-mono">{n.name}</span>
                    <Badge
                      tone={
                        n.kind === "wifi"
                          ? "accent"
                          : n.kind === "ethernet"
                            ? "success"
                            : "default"
                      }
                    >
                      {n.kind.toUpperCase()}
                    </Badge>
                    {n.is_loopback && <Badge>LOOPBACK</Badge>}
                  </span>
                }
                action={
                  live ? (
                    <span className="text-xs font-mono tabular-nums text-text-secondary flex items-center gap-3">
                      <span>↓ {fmt.netSpeed(live.rx)}</span>
                      <span>↑ {fmt.netSpeed(live.tx)}</span>
                    </span>
                  ) : null
                }
              >
                <KeyValueTable
                  rows={[
                    { key: t("spec_mac"), value: nullable(n.mac) },
                    { key: "ipv4", value: n.ipv4.length === 0 ? "—" : n.ipv4.join(", ") },
                    {
                      key: "ipv6",
                      value:
                        n.ipv6.length === 0
                          ? "—"
                          : n.ipv6.slice(0, 2).join(", ") +
                            (n.ipv6.length > 2 ? ` (+${n.ipv6.length - 2})` : ""),
                    },
                    {
                      key: t("spec_link_speed"),
                      value: n.link_speed_mbps ? `${n.link_speed_mbps} Mbps` : "—",
                    },
                    {
                      key: t("spec_status"),
                      value: n.is_up ? t("spec_status_up") : t("spec_status_down"),
                    },
                    { key: t("net_rx_total"), value: fmt.bytes(n.rx_total_bytes) },
                    { key: t("net_tx_total"), value: fmt.bytes(n.tx_total_bytes) },
                    {
                      key: t("net_iface_live"),
                      value: live
                        ? `↓ ${fmt.netSpeed(live.rx)}  ↑ ${fmt.netSpeed(live.tx)}`
                        : "—",
                    },
                  ]}
                />
              </Card>
            );
          })}
        </div>
      )}
    </div>
  );
}
