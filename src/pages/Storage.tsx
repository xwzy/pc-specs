import { useQuery } from "@tanstack/react-query";
import { getStorages } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Bar } from "@/components/ui/Bar";
import { Badge } from "@/components/ui/Badge";
import { Empty } from "@/components/ui/Empty";
import { fmtPercent, nullable, useFmt } from "@/lib/format";
import { PageHeader } from "@/components/layout/PageHeader";
import { useT } from "@/lib/store";

export default function StoragePage() {
  const t = useT();
  const fmt = useFmt();
  const { data: storages = [] } = useQuery({
    queryKey: ["storages"],
    queryFn: getStorages,
    refetchInterval: 5_000,
  });

  return (
    <div className="space-y-5">
      <PageHeader
        title={t("nav_storage")}
        description={`${storages.length} ${t("storage_count_suffix")}`}
      />
      {storages.length === 0 ? (
        <Card>
          <Empty title={t("storage_no_disks")} />
        </Card>
      ) : (
        storages.map((s, i) => {
          const pct = s.total_bytes > 0 ? (s.used_bytes / s.total_bytes) * 100 : 0;
          return (
            <Card
              key={i}
              title={s.name}
              action={
                <span className="flex items-center gap-2">
                  <Badge tone={s.kind === "SSD" || s.kind === "NVMe" ? "accent" : "default"}>
                    {s.kind}
                  </Badge>
                  {s.smart_health && (
                    <Badge tone={s.smart_health === "OK" ? "success" : "warning"}>
                      SMART {s.smart_health}
                    </Badge>
                  )}
                </span>
              }
            >
              <div className="flex flex-wrap items-baseline gap-4">
                <div className="font-mono text-text-primary text-2xl">
                  {fmt.bytes(s.used_bytes)}
                </div>
                <div className="text-text-secondary text-sm font-mono">
                  / {fmt.bytes(s.total_bytes)} ({fmtPercent(pct, 0)})
                </div>
                <div className="ml-auto flex gap-3 text-xs text-text-secondary font-mono">
                  {(s.read_bytes_per_sec > 0 || s.write_bytes_per_sec > 0) && (
                    <>
                      <span>
                        {t("dash_io_read")} {fmt.netSpeed(s.read_bytes_per_sec)}
                      </span>
                      <span>
                        {t("dash_io_write")} {fmt.netSpeed(s.write_bytes_per_sec)}
                      </span>
                    </>
                  )}
                  {s.temperature_c != null && <span>{fmt.temp(s.temperature_c)}</span>}
                </div>
              </div>
              <div className="mt-3">
                <Bar value={pct} />
              </div>
              <div className="mt-3 grid grid-cols-2 md:grid-cols-4 gap-x-6 gap-y-1.5 text-xs">
                <KV label={t("spec_mount")} value={nullable(s.mount_point)} />
                <KV label={t("spec_filesystem")} value={nullable(s.filesystem)} />
                <KV label={t("spec_serial")} value={nullable(s.serial)} />
                <KV label={t("spec_kind")} value={s.kind} />
              </div>
            </Card>
          );
        })
      )}
    </div>
  );
}

function KV({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex flex-col">
      <span className="text-text-tertiary text-[10px] uppercase tracking-wider">{label}</span>
      <span className="text-text-secondary font-mono">{value}</span>
    </div>
  );
}
