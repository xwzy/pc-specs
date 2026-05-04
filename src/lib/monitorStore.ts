import { create } from "zustand";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { listenMonitor, startMonitor, stopMonitor } from "./api";
import type { MonitorTick } from "./types";

const MAX_POINTS = 600;

interface MonitorStore {
  ticks: MonitorTick[];
  latest: MonitorTick | null;
  running: boolean;
  intervalMs: number;
  paused: boolean;
  push: (t: MonitorTick) => void;
  setRunning: (b: boolean) => void;
  setIntervalMs: (ms: number) => void;
  reset: () => void;
  setPaused: (b: boolean) => void;
}

export const useMonitorStore = create<MonitorStore>((set) => ({
  ticks: [],
  latest: null,
  running: false,
  intervalMs: 1000,
  paused: false,
  push: (t) =>
    set((s) => {
      if (s.paused) return s;
      const next = s.ticks.length >= MAX_POINTS ? s.ticks.slice(-MAX_POINTS + 1) : s.ticks.slice();
      next.push(t);
      return { ticks: next, latest: t };
    }),
  setRunning: (b) => set({ running: b }),
  setIntervalMs: (ms) => set({ intervalMs: ms }),
  reset: () => set({ ticks: [], latest: null }),
  setPaused: (paused) => set({ paused }),
}));

let unlisten: UnlistenFn | null = null;
let starting = false;
let installed = false;

export async function installMonitor(intervalMs: number): Promise<void> {
  if (installed) return;
  installed = true;
  unlisten = await listenMonitor((tick) => {
    useMonitorStore.getState().push(tick);
  });
  await ensureRunning(intervalMs);
}

export async function ensureRunning(intervalMs: number): Promise<void> {
  if (starting) return;
  starting = true;
  try {
    await startMonitor(intervalMs);
    useMonitorStore.getState().setRunning(true);
    useMonitorStore.getState().setIntervalMs(intervalMs);
  } catch {
    /* ignore */
  } finally {
    starting = false;
  }
}

export async function changeInterval(intervalMs: number): Promise<void> {
  useMonitorStore.getState().reset();
  await ensureRunning(intervalMs);
}

export async function shutdownMonitor(): Promise<void> {
  if (unlisten) {
    unlisten();
    unlisten = null;
  }
  await stopMonitor().catch(() => undefined);
  useMonitorStore.getState().setRunning(false);
  installed = false;
}
