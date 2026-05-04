import { useQuery } from "@tanstack/react-query";
import { getBattery } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Empty } from "@/components/ui/Empty";
import { KeyValueTable } from "@/components/ui/KeyValueTable";
import { Bar } from "@/components/ui/Bar";
import { nullable, useFmt } from "@/lib/format";
import { PageHeader } from "@/components/layout/PageHeader";
import { useT } from "@/lib/store";
import type { DictKey } from "@/lib/i18n";

const STATE_KEY: Record<string, DictKey> = {
  charging: "bat_state_charging",
  discharging: "bat_state_discharging",
  full: "bat_state_full",
  empty: "bat_state_empty",
  unknown: "bat_state_unknown",
};

function localizedState(t: (k: DictKey) => string, raw: string) {
  const key = STATE_KEY[raw.toLowerCase()];
  return key ? t(key) : raw;
}

export default function BatteryPage() {
  const t = useT();
  const fmt = useFmt();
  const { data: bat } = useQuery({
    queryKey: ["battery"],
    queryFn: getBattery,
    refetchInterval: 10_000,
  });

  if (!bat) {
    return (
      <div className="space-y-5">
        <PageHeader title={t("nav_battery")} />
        <Card>
          <Empty title={t("bat_no_battery")} hint={t("bat_no_battery_hint")} />
        </Card>
      </div>
    );
  }

  const stateLabel = localizedState(t, bat.state);

  return (
    <div className="space-y-5">
      <PageHeader title={t("nav_battery")} description={stateLabel} />
      <Card>
        <div className="font-mono text-text-primary text-3xl mb-2">
          {bat.percentage.toFixed(0)}%
        </div>
        <Bar value={bat.percentage} warningAt={30} dangerAt={15} />
        <div className="mt-4">
          <KeyValueTable
            rows={[
              { key: t("spec_vendor"), value: nullable(bat.vendor) },
              { key: t("spec_brand"), value: nullable(bat.model) },
              { key: t("spec_status"), value: stateLabel },
              { key: t("bat_cycle"), value: nullable(bat.cycle_count) },
              {
                key: t("bat_design_cap"),
                value: bat.design_capacity_mwh ? `${bat.design_capacity_mwh} mWh` : "—",
              },
              {
                key: t("bat_full_cap"),
                value: bat.full_capacity_mwh ? `${bat.full_capacity_mwh} mWh` : "—",
              },
              {
                key: t("bat_curr_cap"),
                value: bat.current_capacity_mwh ? `${bat.current_capacity_mwh} mWh` : "—",
              },
              { key: t("spec_temp"), value: fmt.temp(bat.temperature_c) },
              {
                key: t("bat_power"),
                value:
                  bat.power_now_mw == null
                    ? "—"
                    : `${(bat.power_now_mw / 1000).toFixed(2)} W`,
              },
              {
                key: t("bat_time_to_empty"),
                value: bat.time_to_empty_secs
                  ? `${Math.round(bat.time_to_empty_secs / 60)}min`
                  : "—",
              },
              {
                key: t("bat_time_to_full"),
                value: bat.time_to_full_secs
                  ? `${Math.round(bat.time_to_full_secs / 60)}min`
                  : "—",
              },
            ]}
          />
        </div>
      </Card>
    </div>
  );
}
