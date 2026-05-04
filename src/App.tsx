import { Navigate, Route, Routes } from "react-router-dom";
import { AppShell } from "@/components/layout/AppShell";
import Dashboard from "@/pages/Dashboard";
import CpuPage from "@/pages/Cpu";
import GpuPage from "@/pages/Gpu";
import MemoryPage from "@/pages/Memory";
import StoragePage from "@/pages/Storage";
import MotherboardPage from "@/pages/Motherboard";
import OsPage from "@/pages/OsPage";
import NetworkPage from "@/pages/Network";
import DisplayPage from "@/pages/Display";
import SensorsPage from "@/pages/Sensors";
import BatteryPage from "@/pages/Battery";
import PeripheralsPage from "@/pages/Peripherals";
import DevEnvPage from "@/pages/DevEnv";
import MonitorPage from "@/pages/Monitor";
import ExportPage from "@/pages/Export";
import SettingsPage from "@/pages/Settings";

export default function App() {
  return (
    <AppShell>
      <Routes>
        <Route path="/" element={<Dashboard />} />
        <Route path="/cpu" element={<CpuPage />} />
        <Route path="/gpu" element={<GpuPage />} />
        <Route path="/memory" element={<MemoryPage />} />
        <Route path="/storage" element={<StoragePage />} />
        <Route path="/motherboard" element={<MotherboardPage />} />
        <Route path="/os" element={<OsPage />} />
        <Route path="/network" element={<NetworkPage />} />
        <Route path="/display" element={<DisplayPage />} />
        <Route path="/sensors" element={<SensorsPage />} />
        <Route path="/battery" element={<BatteryPage />} />
        <Route path="/peripherals" element={<PeripheralsPage />} />
        <Route path="/dev-env" element={<DevEnvPage />} />
        <Route path="/monitor" element={<MonitorPage />} />
        <Route path="/export" element={<ExportPage />} />
        <Route path="/settings" element={<SettingsPage />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </AppShell>
  );
}
