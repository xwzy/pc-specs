import { useQuery } from "@tanstack/react-query";
import { getSensors } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { Empty } from "@/components/ui/Empty";
import { PageHeader } from "@/components/layout/PageHeader";
import { useFmt } from "@/lib/format";
import { useT } from "@/lib/store";
import type { DictKey } from "@/lib/i18n";

const KIND_KEY: Record<string, DictKey> = {
  temperature: "sensors_temperature",
  fan: "sensors_fan",
  voltage: "sensors_voltage",
  power: "sensors_power",
  current: "sensors_current",
};

export default function SensorsPage() {
  const t = useT();
  const fmt = useFmt();
  const { data: sensors = [] } = useQuery({
    queryKey: ["sensors"],
    queryFn: getSensors,
    refetchInterval: 5_000,
  });

  const grouped = sensors.reduce<Record<string, typeof sensors>>((acc, s) => {
    (acc[s.kind] ??= []).push(s);
    return acc;
  }, {});

  return (
    <div className="space-y-5">
      <PageHeader title={t("nav_sensors")} description={`${sensors.length}`} />
      {sensors.length === 0 ? (
        <Card>
          <Empty title={t("sensors_no_data")} hint={t("sensors_no_data_hint")} />
        </Card>
      ) : (
        Object.entries(grouped).map(([kind, list]) => {
          const key = KIND_KEY[kind] ?? "sensors_other";
          return (
            <Card
              key={kind}
              title={
                <span>
                  {t(key)} ({list.length})
                </span>
              }
            >
              <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-3">
                {list.map((s, i) => (
                  <div
                    key={i}
                    className="px-3 py-3 rounded-lg bg-bg-elevated/40 border border-border/50 flex flex-col gap-1.5"
                  >
                    <div className="flex items-center justify-between">
                      <Badge>{s.source}</Badge>
                      <span className="text-xs text-text-tertiary font-mono uppercase">
                        {s.unit === "C" && fmt.tempUnit === "F" ? "F" : s.unit}
                      </span>
                    </div>
                    <div className="text-text-secondary text-xs truncate font-mono">{s.label}</div>
                    <div className="font-mono text-text-primary text-xl tabular-nums">
                      {s.unit === "C"
                        ? fmt.temp(s.value).replace(/°[CF]/, "")
                        : s.value.toFixed(s.unit === "V" ? 2 : 0)}
                    </div>
                  </div>
                ))}
              </div>
            </Card>
          );
        })
      )}
    </div>
  );
}
