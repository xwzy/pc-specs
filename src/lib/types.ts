export interface SystemSnapshot {
  timestamp: number;
  host: HostInfo;
  os: OsInfo;
  cpu: CpuInfo;
  gpus: GpuInfo[];
  memory: MemoryInfo;
  storages: StorageInfo[];
  motherboard: MotherboardInfo | null;
  network: NetworkInfo;
  displays: DisplayInfo[];
  sensors: SensorReading[];
  battery: BatteryInfo | null;
  peripherals: PeripheralInfo[];
  dev_env: DevEnvInfo;
}

export interface HostInfo {
  hostname: string;
  username: string;
  uptime_secs: number;
  boot_time: number;
  app_version: string;
}

export interface OsInfo {
  family: string;
  name: string;
  version: string;
  kernel: string;
  arch: string;
  locale: string;
  shell: string | null;
  desktop: string | null;
}

export interface CpuTopology {
  sockets: number;
  p_cores: number | null;
  e_cores: number | null;
  numa_nodes: number;
}

export interface CpuInfo {
  vendor: string;
  brand: string;
  arch: string;
  physical_cores: number;
  logical_cores: number;
  base_frequency_hz: number;
  max_frequency_hz: number;
  current_frequency_hz: number;
  cache_l1_bytes: number | null;
  cache_l2_bytes: number | null;
  cache_l3_bytes: number | null;
  features: string[];
  virtualization: boolean | null;
  usage_per_core: number[];
  usage_overall: number;
  temperature_c: number | null;
  topology: CpuTopology | null;
}

export interface GpuInfo {
  index: number;
  vendor: string;
  name: string;
  backend: string;
  driver: string | null;
  vram_total_bytes: number | null;
  vram_used_bytes: number | null;
  utilization: number | null;
  temperature_c: number | null;
  power_w: number | null;
  pcie_link: string | null;
  is_discrete: boolean;
}

export interface MemoryModule {
  slot: string;
  manufacturer: string | null;
  part_number: string | null;
  capacity_bytes: number;
  speed_mt_s: number | null;
  kind: string | null;
  form_factor: string | null;
}

export interface MemoryInfo {
  total_bytes: number;
  used_bytes: number;
  available_bytes: number;
  swap_total_bytes: number;
  swap_used_bytes: number;
  modules: MemoryModule[];
}

export interface StorageInfo {
  name: string;
  mount_point: string | null;
  filesystem: string | null;
  kind: string;
  total_bytes: number;
  used_bytes: number;
  read_bytes_per_sec: number;
  write_bytes_per_sec: number;
  temperature_c: number | null;
  smart_health: string | null;
  serial: string | null;
}

export interface MotherboardInfo {
  vendor: string | null;
  model: string | null;
  version: string | null;
  serial: string | null;
  bios_vendor: string | null;
  bios_version: string | null;
  bios_date: string | null;
  chassis: string | null;
}

export interface NetworkInterface {
  name: string;
  mac: string | null;
  ipv4: string[];
  ipv6: string[];
  is_up: boolean;
  is_loopback: boolean;
  kind: string;
  link_speed_mbps: number | null;
  rx_bytes_per_sec: number;
  tx_bytes_per_sec: number;
  rx_total_bytes: number;
  tx_total_bytes: number;
}

export interface NetworkInfo {
  interfaces: NetworkInterface[];
  public_ip: string | null;
  default_gateway: string | null;
  dns_servers: string[];
}

export interface DisplayInfo {
  name: string;
  width_px: number;
  height_px: number;
  refresh_hz: number | null;
  scale_factor: number | null;
  is_primary: boolean;
  physical_width_mm: number | null;
  physical_height_mm: number | null;
  color_depth: number | null;
}

export interface SensorReading {
  source: string;
  label: string;
  kind: string;
  value: number;
  unit: string;
}

export interface BatteryInfo {
  vendor: string | null;
  model: string | null;
  state: string;
  percentage: number;
  cycle_count: number | null;
  design_capacity_mwh: number | null;
  full_capacity_mwh: number | null;
  current_capacity_mwh: number | null;
  temperature_c: number | null;
  time_to_empty_secs: number | null;
  time_to_full_secs: number | null;
  power_now_mw: number | null;
}

export interface PeripheralInfo {
  kind: string;
  name: string;
  vendor_id: string | null;
  product_id: string | null;
  bus: string | null;
}

export interface RuntimeInfo {
  name: string;
  version: string | null;
  path: string | null;
}

export interface DevEnvInfo {
  languages: RuntimeInfo[];
  package_managers: RuntimeInfo[];
  vcs: RuntimeInfo[];
  editors: RuntimeInfo[];
  containers: RuntimeInfo[];
  shells: RuntimeInfo[];
  env_keys: string[];
}

export interface MonitorTick {
  timestamp: number;
  cpu_overall: number;
  cpu_per_core: number[];
  mem_used_bytes: number;
  mem_total_bytes: number;
  net_rx_bps: number;
  net_tx_bps: number;
  disk_read_bps: number;
  disk_write_bps: number;
  gpu_utilizations: number[];
  temperatures: SensorReading[];
}
