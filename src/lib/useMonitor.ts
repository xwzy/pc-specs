import { useEffect } from "react";
import { useMonitorStore, ensureRunning, installMonitor, changeInterval } from "./monitorStore";
import { useSettings } from "./store";
import type { MonitorTick } from "./types";

export interface MonitorState {
  ticks: MonitorTick[];
  latest: MonitorTick | null;
  running: boolean;
}

/**
 * 应用级单例监控订阅。
 * 任何组件可以多次调用，它只会在第一次时安装事件监听 + 启动后台采样任务，
 * 切页 / 卸载都不会停止后台任务，避免多组件互相杀死监听。
 */
export function useMonitor(_autostart = true): MonitorState {
  void _autostart;
  const interval = useSettings((s) => s.monitorIntervalMs);
  const ticks = useMonitorStore((s) => s.ticks);
  const latest = useMonitorStore((s) => s.latest);
  const running = useMonitorStore((s) => s.running);
  const currentInterval = useMonitorStore((s) => s.intervalMs);

  useEffect(() => {
    installMonitor(interval).catch(() => undefined);
  }, [interval]);

  useEffect(() => {
    if (currentInterval !== interval) {
      changeInterval(interval).catch(() => undefined);
    } else if (!running) {
      ensureRunning(interval).catch(() => undefined);
    }
  }, [interval, currentInterval, running]);

  return { ticks, latest, running };
}
