import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  BatteryInfo,
  CpuInfo,
  DevEnvInfo,
  DisplayInfo,
  GpuInfo,
  HostInfo,
  MemoryInfo,
  MonitorTick,
  MotherboardInfo,
  NetworkInfo,
  OsInfo,
  PeripheralInfo,
  SensorReading,
  StorageInfo,
  SystemSnapshot,
} from "./types";

export const MONITOR_TICK_EVENT = "monitor://tick";

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function getFullSnapshot(): Promise<SystemSnapshot> {
  if (!isTauri) return mockSnapshot();
  return invoke<SystemSnapshot>("get_full_snapshot");
}

export async function getHost(): Promise<HostInfo> {
  if (!isTauri) return mockSnapshot().host;
  return invoke<HostInfo>("get_host");
}

export async function getOs(): Promise<OsInfo> {
  if (!isTauri) return mockSnapshot().os;
  return invoke<OsInfo>("get_os");
}

export async function getCpu(): Promise<CpuInfo> {
  if (!isTauri) return mockSnapshot().cpu;
  return invoke<CpuInfo>("get_cpu");
}

export async function getGpus(): Promise<GpuInfo[]> {
  if (!isTauri) return mockSnapshot().gpus;
  return invoke<GpuInfo[]>("get_gpus");
}

export async function getMemory(): Promise<MemoryInfo> {
  if (!isTauri) return mockSnapshot().memory;
  return invoke<MemoryInfo>("get_memory");
}

export async function getStorages(): Promise<StorageInfo[]> {
  if (!isTauri) return mockSnapshot().storages;
  return invoke<StorageInfo[]>("get_storages");
}

export async function getMotherboard(): Promise<MotherboardInfo | null> {
  if (!isTauri) return mockSnapshot().motherboard;
  return invoke<MotherboardInfo | null>("get_motherboard");
}

export async function getNetwork(): Promise<NetworkInfo> {
  if (!isTauri) return mockSnapshot().network;
  return invoke<NetworkInfo>("get_network");
}

export async function getDisplays(): Promise<DisplayInfo[]> {
  if (!isTauri) return mockSnapshot().displays;
  return invoke<DisplayInfo[]>("get_displays");
}

export async function getSensors(): Promise<SensorReading[]> {
  if (!isTauri) return mockSnapshot().sensors;
  return invoke<SensorReading[]>("get_sensors");
}

export async function getBattery(): Promise<BatteryInfo | null> {
  if (!isTauri) return mockSnapshot().battery;
  return invoke<BatteryInfo | null>("get_battery");
}

export async function getPeripherals(): Promise<PeripheralInfo[]> {
  if (!isTauri) return mockSnapshot().peripherals;
  return invoke<PeripheralInfo[]>("get_peripherals");
}

export async function getDevEnv(): Promise<DevEnvInfo> {
  if (!isTauri) return mockSnapshot().dev_env;
  return invoke<DevEnvInfo>("get_dev_env");
}

export async function getPublicIp(): Promise<string | null> {
  if (!isTauri) return null;
  return invoke<string | null>("get_public_ip");
}

export async function startMonitor(intervalMs = 1000): Promise<void> {
  if (!isTauri) return;
  return invoke("start_monitor", { intervalMs });
}

export async function stopMonitor(): Promise<void> {
  if (!isTauri) return;
  return invoke("stop_monitor");
}

export async function exportMarkdown(includeSensitive = false): Promise<string> {
  if (!isTauri) return "# PC Specs (browser preview)\n\n_Tauri runtime not detected._";
  return invoke<string>("export_markdown", { includeSensitive });
}

export async function exportJson(pretty = true, includeSensitive = false): Promise<string> {
  if (!isTauri) return JSON.stringify(mockSnapshot(), null, pretty ? 2 : 0);
  return invoke<string>("export_json", { pretty, includeSensitive });
}

export async function saveExport(path: string, content: string): Promise<void> {
  if (!isTauri) return;
  return invoke("save_export", { path, content });
}

export interface TraySettings {
  show_cpu: boolean;
  show_memory: boolean;
  show_disk: boolean;
  show_network: boolean;
  show_temperature: boolean;
  macos_show_title: boolean;
}

export async function applyTraySettings(settings: TraySettings): Promise<void> {
  if (!isTauri) return;
  return invoke("apply_tray_settings", { settings });
}

export async function setFloatingNetSpeed(enabled: boolean): Promise<void> {
  if (!isTauri) return;
  return invoke("set_floating_net_speed", { enabled });
}

export async function closeFloatingWindow(label: string): Promise<void> {
  if (!isTauri) return;
  return invoke("close_floating_window", { label });
}

export async function listenFloatingNetSpeedClosed(
  cb: () => void,
): Promise<UnlistenFn> {
  if (!isTauri) return () => undefined;
  return listen<unknown>("floating://net-speed-closed", () => cb());
}

export async function listenMonitor(
  cb: (tick: MonitorTick) => void,
): Promise<UnlistenFn> {
  if (!isTauri) {
    const id = window.setInterval(() => {
      cb(mockTick());
    }, 1000);
    return () => window.clearInterval(id);
  }
  return listen<MonitorTick>(MONITOR_TICK_EVENT, (e) => cb(e.payload));
}

function mockTick(): MonitorTick {
  const cpuPerCore = Array.from({ length: 8 }, () => Math.random() * 100);
  return {
    timestamp: Date.now(),
    cpu_overall: cpuPerCore.reduce((a, b) => a + b, 0) / cpuPerCore.length,
    cpu_per_core: cpuPerCore,
    mem_used_bytes: 30 * 1024 ** 3 + Math.random() * 4 * 1024 ** 3,
    mem_total_bytes: 64 * 1024 ** 3,
    net_rx_bps: Math.random() * 4 * 1024 ** 2,
    net_tx_bps: Math.random() * 1 * 1024 ** 2,
    disk_read_bps: 0,
    disk_write_bps: 0,
    gpu_utilizations: [Math.random() * 100],
    temperatures: [
      { source: "mock", label: "CPU Package", kind: "temperature", value: 50 + Math.random() * 20, unit: "C" },
    ],
    per_interface: [
      { name: "en0", rx_bps: Math.random() * 2 * 1024 ** 2, tx_bps: Math.random() * 512 * 1024 },
      { name: "lo0", rx_bps: 0, tx_bps: 0 },
    ],
  };
}

let _mock: SystemSnapshot | null = null;

function mockSnapshot(): SystemSnapshot {
  if (_mock) return _mock;
  _mock = {
    timestamp: Date.now(),
    host: {
      hostname: "demo.local",
      username: "geek",
      uptime_secs: 3600 * 24 * 3,
      boot_time: Math.floor(Date.now() / 1000) - 3600 * 24 * 3,
      app_version: "0.1.0",
    },
    os: {
      family: "macos",
      name: "macOS Sonoma",
      version: "14.5",
      kernel: "Darwin 23.5.0",
      arch: "aarch64",
      locale: "zh_CN.UTF-8",
      shell: "/bin/zsh",
      desktop: "Aqua",
    },
    cpu: {
      vendor: "Apple",
      brand: "Apple M3 Max",
      arch: "aarch64",
      physical_cores: 16,
      logical_cores: 16,
      base_frequency_hz: 4_060_000_000,
      max_frequency_hz: 4_060_000_000,
      current_frequency_hz: 4_060_000_000,
      cache_l1_bytes: null,
      cache_l2_bytes: 16 * 1024 ** 2,
      cache_l3_bytes: 32 * 1024 ** 2,
      features: ["neon", "fp16", "sve", "dotprod"],
      virtualization: true,
      usage_per_core: Array.from({ length: 16 }, () => Math.random() * 30),
      usage_overall: 18.5,
      temperature_c: 56,
      topology: { sockets: 1, p_cores: 12, e_cores: 4, numa_nodes: 1 },
    },
    gpus: [
      {
        index: 0,
        vendor: "Apple",
        name: "Apple M3 Max GPU",
        backend: "Metal",
        driver: null,
        vram_total_bytes: null,
        vram_used_bytes: null,
        utilization: 17,
        temperature_c: 52,
        power_w: null,
        pcie_link: null,
        is_discrete: false,
      },
    ],
    memory: {
      total_bytes: 64 * 1024 ** 3,
      used_bytes: 38 * 1024 ** 3,
      available_bytes: 26 * 1024 ** 3,
      swap_total_bytes: 4 * 1024 ** 3,
      swap_used_bytes: 0.4 * 1024 ** 3,
      modules: [],
    },
    storages: [
      {
        name: "Macintosh HD",
        mount_point: "/",
        filesystem: "apfs",
        kind: "SSD",
        total_bytes: 1024 ** 4,
        used_bytes: 620 * 1024 ** 3,
        read_bytes_per_sec: 12 * 1024 ** 2,
        write_bytes_per_sec: 4 * 1024 ** 2,
        temperature_c: 41,
        smart_health: "OK",
        serial: null,
      },
    ],
    motherboard: {
      vendor: "Apple",
      model: "MacBookPro18,2",
      version: null,
      serial: null,
      bios_vendor: null,
      bios_version: null,
      bios_date: null,
      chassis: "Laptop",
    },
    network: {
      interfaces: [
        {
          name: "en0",
          mac: "aa:bb:cc:dd:ee:ff",
          ipv4: ["192.168.1.123"],
          ipv6: [],
          is_up: true,
          is_loopback: false,
          kind: "wifi",
          link_speed_mbps: 1200,
          rx_bytes_per_sec: 2.4 * 1024 ** 2,
          tx_bytes_per_sec: 0.3 * 1024 ** 2,
          rx_total_bytes: 12 * 1024 ** 3,
          tx_total_bytes: 3 * 1024 ** 3,
        },
      ],
      public_ip: null,
      default_gateway: "192.168.1.1",
      dns_servers: ["192.168.1.1", "8.8.8.8"],
    },
    displays: [],
    sensors: [
      { source: "mock", label: "CPU Package", kind: "temperature", value: 56, unit: "C" },
      { source: "mock", label: "GPU", kind: "temperature", value: 52, unit: "C" },
      { source: "mock", label: "SSD", kind: "temperature", value: 41, unit: "C" },
    ],
    battery: null,
    peripherals: [],
    dev_env: {
      languages: [
        { name: "node", version: "v20.10.0", path: "/usr/local/bin/node" },
        { name: "rustc", version: "rustc 1.92.0", path: null },
      ],
      package_managers: [{ name: "pnpm", version: "10.26.2", path: null }],
      vcs: [{ name: "git", version: "git version 2.45.0", path: null }],
      editors: [{ name: "code", version: "1.95.0", path: null }],
      containers: [],
      shells: [{ name: "zsh", version: "zsh 5.9", path: null }],
      env_keys: ["PATH", "SHELL", "EDITOR"],
    },
  };
  return _mock;
}
