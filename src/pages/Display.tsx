import { useQuery } from "@tanstack/react-query";
import { Star } from "lucide-react";
import { getDisplays } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Empty } from "@/components/ui/Empty";
import { Badge } from "@/components/ui/Badge";
import { PageHeader } from "@/components/layout/PageHeader";
import { nullable } from "@/lib/format";
import { useT } from "@/lib/store";

export default function DisplayPage() {
  const t = useT();
  const { data: displays = [] } = useQuery({
    queryKey: ["displays"],
    queryFn: getDisplays,
  });

  const maxW = displays.reduce((m, d) => Math.max(m, d.width_px), 1);
  const maxH = displays.reduce((m, d) => Math.max(m, d.height_px), 1);
  const scaleBaseW = 220;
  const scaleBaseH = 140;

  return (
    <div className="space-y-5">
      <PageHeader
        title={t("nav_display")}
        description={`${displays.length} ${t("disp_count_suffix")}`}
      />
      {displays.length === 0 ? (
        <Card>
          <Empty title={t("disp_no_data")} hint={t("disp_no_data_hint")} />
        </Card>
      ) : (
        <>
          <Card title={t("disp_layout_preview")}>
            <div className="flex flex-wrap items-end gap-3 py-3">
              {displays.map((d, i) => {
                const w = (d.width_px / maxW) * scaleBaseW;
                const h = (d.height_px / maxH) * scaleBaseH;
                return (
                  <div
                    key={i}
                    className={
                      "relative rounded-md border-2 flex flex-col items-center justify-center text-xs font-mono " +
                      (d.is_primary
                        ? "border-accent/60 bg-accent/5 text-accent"
                        : "border-border bg-bg-elevated/40 text-text-secondary")
                    }
                    style={{ width: `${Math.max(60, w)}px`, height: `${Math.max(40, h)}px` }}
                  >
                    {d.is_primary && (
                      <Star
                        size={11}
                        className="absolute top-1 right-1 fill-accent text-accent"
                      />
                    )}
                    <div className="leading-tight">
                      {d.width_px}×{d.height_px}
                    </div>
                    {d.scale_factor != null && d.scale_factor !== 1 && (
                      <div className="text-[10px] opacity-60">
                        {d.scale_factor.toFixed(1)}×
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
            <div className="text-text-tertiary text-[11px] mt-1">{t("disp_layout_hint")}</div>
          </Card>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            {displays.map((d, i) => (
              <Card
                key={i}
                title={d.name}
                action={d.is_primary ? <Badge tone="accent">{t("disp_primary")}</Badge> : null}
              >
                <div className="font-mono text-2xl text-text-primary">
                  {d.width_px} × {d.height_px}
                  {d.refresh_hz ? (
                    <span className="text-base text-text-secondary"> @ {d.refresh_hz}Hz</span>
                  ) : null}
                </div>
                <div className="mt-2 grid grid-cols-2 gap-x-6 gap-y-1.5 text-xs text-text-secondary font-mono">
                  {d.scale_factor != null && (
                    <span>
                      {t("disp_scale")} {d.scale_factor.toFixed(2)}×
                    </span>
                  )}
                  {d.scale_factor != null && d.scale_factor > 1.5 && (
                    <span>
                      {t("disp_logical")} {Math.round(d.width_px / d.scale_factor)} ×{" "}
                      {Math.round(d.height_px / d.scale_factor)}
                    </span>
                  )}
                  {d.color_depth != null && (
                    <span>
                      {t("disp_color")} {d.color_depth}-bit
                    </span>
                  )}
                  {(d.physical_width_mm != null || d.physical_height_mm != null) && (
                    <span>
                      {t("disp_physical")} {nullable(d.physical_width_mm)} ×{" "}
                      {nullable(d.physical_height_mm)} mm
                    </span>
                  )}
                </div>
              </Card>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
