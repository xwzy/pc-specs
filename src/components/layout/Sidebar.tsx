import { NavLink } from "react-router-dom";
import {
  Activity,
  BatteryFull,
  Cpu,
  HardDrive,
  LayoutDashboard,
  MemoryStick,
  Monitor,
  MonitorPlay,
  Network,
  Server,
  Settings as SettingsIcon,
  Share2,
  Sparkles,
  Terminal,
  Thermometer,
  Usb,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useT } from "@/lib/store";
import { Logo } from "@/components/ui/Logo";
import type { DictKey } from "@/lib/i18n";

interface NavItemDef {
  to: string;
  labelKey: DictKey;
  icon: LucideIcon;
}

const items: NavItemDef[] = [
  { to: "/", labelKey: "nav_dashboard", icon: LayoutDashboard },
  { to: "/cpu", labelKey: "nav_cpu", icon: Cpu },
  { to: "/gpu", labelKey: "nav_gpu", icon: MonitorPlay },
  { to: "/memory", labelKey: "nav_memory", icon: MemoryStick },
  { to: "/storage", labelKey: "nav_storage", icon: HardDrive },
  { to: "/motherboard", labelKey: "nav_motherboard", icon: Server },
  { to: "/os", labelKey: "nav_os", icon: Sparkles },
  { to: "/network", labelKey: "nav_network", icon: Network },
  { to: "/display", labelKey: "nav_display", icon: Monitor },
  { to: "/sensors", labelKey: "nav_sensors", icon: Thermometer },
  { to: "/battery", labelKey: "nav_battery", icon: BatteryFull },
  { to: "/peripherals", labelKey: "nav_peripherals", icon: Usb },
  { to: "/dev-env", labelKey: "nav_dev_env", icon: Terminal },
  { to: "/monitor", labelKey: "nav_monitor", icon: Activity },
  { to: "/export", labelKey: "nav_export", icon: Share2 },
  { to: "/settings", labelKey: "nav_settings", icon: SettingsIcon },
];

export function Sidebar() {
  const t = useT();
  return (
    <aside className="w-[220px] shrink-0 h-full border-r border-border bg-bg-surface flex flex-col">
      <div className="px-4 py-4 flex items-center gap-2.5 border-b border-border">
        <Logo size={32} className="shrink-0 ring-1 ring-border/60 shadow-sm" />
        <div className="flex flex-col min-w-0">
          <span className="text-text-primary text-sm font-semibold leading-none truncate">
            {t("app_name")}
          </span>
          <span className="text-text-tertiary text-[10px] uppercase tracking-widest mt-0.5">v0.1</span>
        </div>
      </div>
      <nav className="flex-1 overflow-y-auto px-3 py-3 flex flex-col gap-0.5 no-scrollbar">
        {items.map((it) => (
          <NavLink
            key={it.to}
            to={it.to}
            end={it.to === "/"}
            className={({ isActive }) => cn("nav-item", isActive && "nav-item-active")}
          >
            <it.icon size={16} />
            <span>{t(it.labelKey)}</span>
          </NavLink>
        ))}
      </nav>
      <div className="px-4 py-3 border-t border-border text-text-tertiary text-[10px] flex justify-between items-center">
        <span className="font-mono uppercase tracking-widest">{t("common_local_only")}</span>
        <span className="font-mono">v0.1.0</span>
      </div>
    </aside>
  );
}
