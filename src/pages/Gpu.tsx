import { useQuery } from "@tanstack/react-query";
import { getGpus } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { KeyValueTable } from "@/components/ui/KeyValueTable";
import { Empty } from "@/components/ui/Empty";
import { Section } from "@/components/ui/Section";
import { fmtPercent, nullable, useFmt } from "@/lib/format";
import { PageHeader } from "@/components/layout/PageHeader";
import { useSettings, useT } from "@/lib/store";

export default function GpuPage() {
  const t = useT();
  const fmt = useFmt();
  const geek = useSettings((s) => s.geekMode);
  const { data: gpus = [] } = useQuery({ queryKey: ["gpus"], queryFn: getGpus });

  return (
    <div className="space-y-5">
      <PageHeader
        title={t("nav_gpu")}
        description={`${gpus.length} ${t("gpu_count_suffix")}`}
      />
      {gpus.length === 0 ? (
        <Card>
          <Empty title={t("gpu_no_devices")} hint={t("gpu_no_devices_hint")} />
        </Card>
      ) : (
        gpus.map((g) => (
          <Card
            key={g.index}
            title={
              <span>
                [{g.index}] {g.name}
              </span>
            }
            action={
              <span className="flex items-center gap-2">
                <Badge tone="accent">{g.vendor}</Badge>
                <Badge>{g.backend}</Badge>
                {g.is_discrete ? (
                  <Badge tone="warning">{t("gpu_discrete")}</Badge>
                ) : (
                  <Badge>{t("gpu_integrated")}</Badge>
                )}
              </span>
            }
          >
            <KeyValueTable
              rows={[
                { key: t("spec_vendor"), value: g.vendor },
                { key: t("spec_brand"), value: g.name },
                { key: t("spec_backend"), value: g.backend },
                { key: t("spec_driver"), value: nullable(g.driver) },
                { key: `${t("spec_vram")} ${t("spec_total")}`, value: fmt.bytes(g.vram_total_bytes) },
                { key: t("gpu_vram_used"), value: fmt.bytes(g.vram_used_bytes) },
                { key: t("gpu_utilization"), value: fmtPercent(g.utilization ?? null) },
                { key: t("spec_temp"), value: fmt.temp(g.temperature_c) },
                { key: t("gpu_power"), value: g.power_w == null ? "—" : `${g.power_w.toFixed(1)} W` },
                { key: t("spec_pcie"), value: nullable(g.pcie_link) },
                { key: t("spec_kind"), value: g.is_discrete ? t("gpu_discrete") : t("gpu_integrated") },
              ]}
            />

            <Section title={t("gpu_geek_panel")} defaultOpen={geek}>
              <KeyValueTable
                rows={[
                  { key: "wgpu index", value: String(g.index) },
                  { key: "vendor (raw)", value: g.vendor },
                  { key: "device type", value: g.is_discrete ? "DiscreteGpu" : "IntegratedGpu/Cpu" },
                  { key: "driver string", value: nullable(g.driver) },
                  { key: "pcie link", value: nullable(g.pcie_link) },
                ]}
              />
              <div className="mt-3 text-text-tertiary text-xs">{t("gpu_geek_hint")}</div>
            </Section>
          </Card>
        ))
      )}
    </div>
  );
}
