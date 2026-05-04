import { useQuery } from "@tanstack/react-query";
import { getPeripherals } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Empty } from "@/components/ui/Empty";
import { Badge } from "@/components/ui/Badge";
import { PageHeader } from "@/components/layout/PageHeader";
import { nullable } from "@/lib/format";
import { useT } from "@/lib/store";

export default function PeripheralsPage() {
  const t = useT();
  const { data: list = [] } = useQuery({
    queryKey: ["peripherals"],
    queryFn: getPeripherals,
    refetchInterval: 10_000,
  });
  return (
    <div className="space-y-5">
      <PageHeader
        title={t("nav_peripherals")}
        description={`${list.length} ${t("peri_count_suffix")}`}
      />
      {list.length === 0 ? (
        <Card>
          <Empty title={t("peri_no_devices")} hint={t("peri_no_devices_hint")} />
        </Card>
      ) : (
        <Card>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            {list.map((p, i) => (
              <div
                key={i}
                className="px-4 py-3 rounded-lg bg-bg-elevated/40 border border-border/50 flex items-center justify-between"
              >
                <div className="min-w-0">
                  <div className="text-text-primary font-mono text-sm truncate">{p.name}</div>
                  <div className="text-text-tertiary text-xs font-mono">
                    {nullable(p.vendor_id)} : {nullable(p.product_id)} · {nullable(p.bus)}
                  </div>
                </div>
                <Badge tone="accent">{p.kind.toUpperCase()}</Badge>
              </div>
            ))}
          </div>
        </Card>
      )}
    </div>
  );
}
