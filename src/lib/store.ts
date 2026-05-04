import { create } from "zustand";
import type { Lang } from "./i18n";
import { translate, translateWith, type DictKey } from "./i18n";

type Theme = "dark" | "light";
export type TempUnit = "C" | "F";
export type ByteUnit = "binary" | "decimal";
export type NetSpeedUnit = "byte" | "bit";

interface Settings {
  theme: Theme;
  geekMode: boolean;
  monitorIntervalMs: number;
  lang: Lang;
  publicIpEnabled: boolean;
  publicIp: string | null;
  tempUnit: TempUnit;
  byteUnit: ByteUnit;
  netSpeedUnit: NetSpeedUnit;
  exportSensitive: boolean;
  setTheme: (t: Theme) => void;
  toggleGeek: () => void;
  setMonitorInterval: (ms: number) => void;
  setLang: (l: Lang) => void;
  setPublicIpEnabled: (b: boolean) => void;
  setPublicIp: (ip: string | null) => void;
  setTempUnit: (u: TempUnit) => void;
  setByteUnit: (u: ByteUnit) => void;
  setNetSpeedUnit: (u: NetSpeedUnit) => void;
  setExportSensitive: (b: boolean) => void;
}

const STORAGE_KEY = "pc-specs.settings";

interface PersistedSettings {
  theme: Theme;
  geekMode: boolean;
  monitorIntervalMs: number;
  lang: Lang;
  publicIpEnabled: boolean;
  tempUnit: TempUnit;
  byteUnit: ByteUnit;
  netSpeedUnit: NetSpeedUnit;
  exportSensitive: boolean;
}

function loadInitial(): PersistedSettings {
  const fallback: PersistedSettings = {
    theme: "dark",
    geekMode: false,
    monitorIntervalMs: 1000,
    lang: detectLang(),
    publicIpEnabled: false,
    tempUnit: "C",
    byteUnit: "binary",
    netSpeedUnit: "byte",
    exportSensitive: false,
  };
  if (typeof window === "undefined") {
    return fallback;
  }
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return fallback;
    const parsed = JSON.parse(raw) as Partial<PersistedSettings>;
    return {
      theme: parsed.theme === "light" ? "light" : "dark",
      geekMode: !!parsed.geekMode,
      monitorIntervalMs: parsed.monitorIntervalMs ?? 1000,
      lang: parsed.lang === "en" ? "en" : "zh",
      publicIpEnabled: !!parsed.publicIpEnabled,
      tempUnit: parsed.tempUnit === "F" ? "F" : "C",
      byteUnit: parsed.byteUnit === "decimal" ? "decimal" : "binary",
      netSpeedUnit: parsed.netSpeedUnit === "bit" ? "bit" : "byte",
      exportSensitive: !!parsed.exportSensitive,
    };
  } catch {
    return fallback;
  }
}

function detectLang(): Lang {
  return "zh";
}

function persist(s: PersistedSettings) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(s));
  } catch {
    /* ignore quota errors */
  }
}

const initial = loadInitial();

function snapshot(get: () => Settings): PersistedSettings {
  const s = get();
  return {
    theme: s.theme,
    geekMode: s.geekMode,
    monitorIntervalMs: s.monitorIntervalMs,
    lang: s.lang,
    publicIpEnabled: s.publicIpEnabled,
    tempUnit: s.tempUnit,
    byteUnit: s.byteUnit,
    netSpeedUnit: s.netSpeedUnit,
    exportSensitive: s.exportSensitive,
  };
}

export const useSettings = create<Settings>((set, get) => ({
  ...initial,
  publicIp: null,
  setTheme: (theme) => {
    set({ theme });
    if (typeof document !== "undefined") {
      document.documentElement.classList.toggle("light", theme === "light");
      document.documentElement.classList.toggle("dark", theme === "dark");
    }
    persist(snapshot(get));
  },
  toggleGeek: () => {
    set({ geekMode: !get().geekMode });
    persist(snapshot(get));
  },
  setMonitorInterval: (monitorIntervalMs) => {
    set({ monitorIntervalMs });
    persist(snapshot(get));
  },
  setLang: (lang) => {
    set({ lang });
    if (typeof document !== "undefined") {
      document.documentElement.lang = lang === "zh" ? "zh-CN" : "en";
    }
    persist(snapshot(get));
  },
  setPublicIpEnabled: (publicIpEnabled) => {
    set({ publicIpEnabled });
    if (!publicIpEnabled) {
      set({ publicIp: null });
    }
    persist(snapshot(get));
  },
  setPublicIp: (publicIp) => set({ publicIp }),
  setTempUnit: (tempUnit) => {
    set({ tempUnit });
    persist(snapshot(get));
  },
  setByteUnit: (byteUnit) => {
    set({ byteUnit });
    persist(snapshot(get));
  },
  setNetSpeedUnit: (netSpeedUnit) => {
    set({ netSpeedUnit });
    persist(snapshot(get));
  },
  setExportSensitive: (exportSensitive) => {
    set({ exportSensitive });
    persist(snapshot(get));
  },
}));

if (typeof document !== "undefined") {
  document.documentElement.classList.toggle("light", initial.theme === "light");
  document.documentElement.classList.toggle("dark", initial.theme === "dark");
  document.documentElement.lang = initial.lang === "zh" ? "zh-CN" : "en";
}

export function useT() {
  const lang = useSettings((s) => s.lang);
  return (key: DictKey) => translate(lang, key);
}

export function useTWith() {
  const lang = useSettings((s) => s.lang);
  return (key: DictKey, params: Record<string, string | number>) =>
    translateWith(lang, key, params);
}

export function useLang() {
  return useSettings((s) => s.lang);
}
