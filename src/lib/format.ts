import { useSettings, type ByteUnit, type NetSpeedUnit, type TempUnit } from "./store";

export function fmtBytes(
  n: number | null | undefined,
  opts: { binary?: boolean; digits?: number } = {},
): string {
  if (n === null || n === undefined || Number.isNaN(n)) return "—";
  const { binary = true, digits = 1 } = opts;
  const base = binary ? 1024 : 1000;
  const units = binary
    ? ["B", "KiB", "MiB", "GiB", "TiB", "PiB"]
    : ["B", "KB", "MB", "GB", "TB", "PB"];
  let v = n;
  let i = 0;
  while (Math.abs(v) >= base && i < units.length - 1) {
    v /= base;
    i++;
  }
  return `${v.toFixed(i === 0 ? 0 : digits)} ${units[i]}`;
}

export function fmtBytesPerSec(n: number | null | undefined): string {
  if (n === null || n === undefined || Number.isNaN(n)) return "—";
  return `${fmtBytes(n)}/s`;
}

export function fmtHz(hz: number | null | undefined): string {
  if (hz === null || hz === undefined || hz === 0) return "—";
  if (hz >= 1_000_000_000) return `${(hz / 1_000_000_000).toFixed(2)} GHz`;
  if (hz >= 1_000_000) return `${(hz / 1_000_000).toFixed(0)} MHz`;
  return `${hz} Hz`;
}

export function fmtPercent(p: number | null | undefined, digits = 1): string {
  if (p === null || p === undefined || Number.isNaN(p)) return "—";
  return `${p.toFixed(digits)}%`;
}

export function fmtTemp(c: number | null | undefined): string {
  if (c === null || c === undefined || Number.isNaN(c)) return "—";
  return `${c.toFixed(1)}°C`;
}

export function fmtUptime(secs: number): string {
  if (!secs || secs < 0) return "—";
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const parts: string[] = [];
  if (d > 0) parts.push(`${d}d`);
  if (h > 0) parts.push(`${h}h`);
  if (parts.length === 0 || (d === 0 && m > 0)) parts.push(`${m}m`);
  return parts.join(" ");
}

export function fmtTimestamp(ms: number): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

export function nullable<T>(v: T | null | undefined, fallback = "—"): string {
  if (v === null || v === undefined) return fallback;
  return String(v);
}

export function clamp(v: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, v));
}

// 单位感知版本——尊重 Settings 中的单位选择
function fmtBytesWith(n: number | null | undefined, unit: ByteUnit, digits = 1): string {
  return fmtBytes(n, { binary: unit === "binary", digits });
}

function fmtTempWith(c: number | null | undefined, unit: TempUnit): string {
  if (c === null || c === undefined || Number.isNaN(c)) return "—";
  if (unit === "F") return `${((c * 9) / 5 + 32).toFixed(1)}°F`;
  return `${c.toFixed(1)}°C`;
}

function fmtNetSpeedWith(bytesPerSec: number | null | undefined, unit: NetSpeedUnit): string {
  if (bytesPerSec === null || bytesPerSec === undefined || Number.isNaN(bytesPerSec)) return "—";
  if (unit === "bit") {
    const bps = bytesPerSec * 8;
    const units = ["bps", "Kbps", "Mbps", "Gbps", "Tbps"];
    let v = bps;
    let i = 0;
    while (v >= 1000 && i < units.length - 1) {
      v /= 1000;
      i++;
    }
    return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
  }
  return fmtBytesPerSec(bytesPerSec);
}

/** Hook 返回一组带单位偏好的格式化函数。 */
export function useFmt() {
  const tempUnit = useSettings((s) => s.tempUnit);
  const byteUnit = useSettings((s) => s.byteUnit);
  const netSpeedUnit = useSettings((s) => s.netSpeedUnit);
  return {
    bytes: (n: number | null | undefined, digits = 1) => fmtBytesWith(n, byteUnit, digits),
    temp: (c: number | null | undefined) => fmtTempWith(c, tempUnit),
    netSpeed: (bps: number | null | undefined) => fmtNetSpeedWith(bps, netSpeedUnit),
    byteUnit,
    tempUnit,
    netSpeedUnit,
  };
}
