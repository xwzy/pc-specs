/**
 * 桌面网速悬浮窗：常驻置顶的小条，显示当前 ↓/↑。
 *
 * 交互：
 *  - 整窗 `data-tauri-drag-region` → 鼠标按住可拖动到屏幕任意位置
 *  - 双击 / 右键 → 关闭悬浮窗（同步 settings 回退到 disabled，由 lib.rs 的 Destroyed
 *    监听 + 前端 store 完成）
 *
 * 实现取舍（important）：
 *  - 悬浮窗是独立 webview，**不与主窗口共享 zustand store / localStorage 命名空间**。
 *    所以这里不读 `useSettings`、不读 `useFmt`，避免主窗口改主题 / 单位时悬浮窗
 *    显示残留旧值。固定深色 + 二进制单位，体验稳定。
 *  - 由于后端 `start_monitor` 已经做了幂等（相同 interval 跳过重启），悬浮窗调 useMonitor
 *    与主窗口并存不会再 thrash 出第一帧 0 BPS。
 *  - 不使用 borderRadius，因为没启用窗口透明（跨平台稳定考虑），圆角外会露出 webview
 *    默认背景色破坏视觉。改用矩形 + border 即可。
 *
 * 这个组件只在 `index.html#/floating/net-speed` 路由下挂载，main.tsx 入口判定。
 */

import { useEffect } from "react";
import { useMonitor } from "@/lib/useMonitor";
import { closeFloatingWindow } from "@/lib/api";

const FLOATING_LABEL = "floating-net-speed";

export default function FloatingNetSpeed() {
  const { latest } = useMonitor();

  useEffect(() => {
    document.documentElement.classList.add("dark");
    document.documentElement.classList.remove("light");
    document.body.style.background = "#0d1117";
    document.body.style.margin = "0";
    document.body.style.overflow = "hidden";
    document.documentElement.style.background = "#0d1117";
    document.documentElement.style.overflow = "hidden";
  }, []);

  const close = () => closeFloatingWindow(FLOATING_LABEL);

  const rx = latest?.net_rx_bps ?? 0;
  const tx = latest?.net_tx_bps ?? 0;

  return (
    <div
      data-tauri-drag-region
      onDoubleClick={close}
      onContextMenu={(e) => {
        e.preventDefault();
        close();
      }}
      style={{
        width: "100vw",
        height: "100vh",
        boxSizing: "border-box",
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        gap: 10,
        padding: "8px 14px",
        background: "#0d1117",
        color: "#e7ecf3",
        border: "1px solid #2c3645",
        fontFamily:
          "'JetBrains Mono','Cascadia Code','Menlo',ui-monospace,monospace",
        userSelect: "none",
        cursor: "move",
        WebkitUserSelect: "none",
      }}
      title="拖动移动 · 双击关闭"
    >
      <Speed direction="rx" bps={rx} />
      <div
        style={{
          width: 1,
          height: 22,
          background: "#2c3645",
        }}
      />
      <Speed direction="tx" bps={tx} />
    </div>
  );
}

function Speed({ direction, bps }: { direction: "rx" | "tx"; bps: number }) {
  const arrow = direction === "rx" ? "↓" : "↑";
  const accent = direction === "rx" ? "#22d3ee" : "#a78bfa";
  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        alignItems: "baseline",
        justifyContent: "center",
        gap: 4,
        fontVariantNumeric: "tabular-nums",
      }}
    >
      <span style={{ color: accent, fontSize: 13, fontWeight: 700 }}>{arrow}</span>
      <span style={{ fontSize: 13, fontWeight: 600 }}>{formatBytesPerSec(bps)}</span>
    </div>
  );
}

/** 极简二进制字节速率格式化（binary, KiB/s）。仅供悬浮窗使用，不依赖全局 settings。 */
function formatBytesPerSec(bps: number): string {
  if (!Number.isFinite(bps) || bps <= 0) return "0 KB/s";
  const units = ["B/s", "KB/s", "MB/s", "GB/s"];
  let v = bps;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  if (i === 0) return `${v.toFixed(0)} ${units[i]}`;
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
}
