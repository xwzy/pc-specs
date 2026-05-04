import { clamp } from "@/lib/format";

interface CoreBarsProps {
  values: number[];
  max?: number;
  label?: string;
}

export function CoreBars({ values, max = 100, label }: CoreBarsProps) {
  return (
    <div>
      {label && <div className="label-tt mb-2">{label}</div>}
      <div className="grid grid-flow-col auto-cols-fr gap-1 h-20 items-end">
        {values.map((v, i) => {
          const pct = clamp((v / max) * 100, 0, 100);
          const tone =
            pct >= 90 ? "bg-danger" : pct >= 75 ? "bg-warning" : "bg-accent";
          return (
            <div key={i} className="relative h-full bg-bg-elevated rounded-sm overflow-hidden">
              <div
                className={`absolute bottom-0 left-0 right-0 ${tone} transition-[height] duration-300`}
                style={{ height: `${pct}%` }}
                title={`Core ${i}: ${pct.toFixed(0)}%`}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}
