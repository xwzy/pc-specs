import { useState } from "react";
import { Card } from "@/components/ui/Card";
import { Logo } from "@/components/ui/Logo";
import { useSettings, useT, type TraySettings } from "@/lib/store";
import { PageHeader } from "@/components/layout/PageHeader";
import { getPublicIp } from "@/lib/api";
import type { Lang } from "@/lib/i18n";

const INTERVALS = [500, 1000, 2000, 5000];

// `navigator.platform` 在新版浏览器是 deprecated 但 Tauri webview 仍可靠返回。
// 双保险：同时看 userAgent，兼容个别隐私模式 / WebView 关闭 platform 的情况。
const IS_MAC = (() => {
  if (typeof navigator === "undefined") return false;
  const platform = navigator.platform || "";
  const ua = navigator.userAgent || "";
  return /Mac|Darwin/i.test(platform) || /Mac OS X|Macintosh/i.test(ua);
})();

export default function SettingsPage() {
  const t = useT();
  const theme = useSettings((s) => s.theme);
  const setTheme = useSettings((s) => s.setTheme);
  const geek = useSettings((s) => s.geekMode);
  const toggleGeek = useSettings((s) => s.toggleGeek);
  const interval = useSettings((s) => s.monitorIntervalMs);
  const setMonitorInterval = useSettings((s) => s.setMonitorInterval);
  const lang = useSettings((s) => s.lang);
  const setLang = useSettings((s) => s.setLang);
  const publicIpEnabled = useSettings((s) => s.publicIpEnabled);
  const setPublicIpEnabled = useSettings((s) => s.setPublicIpEnabled);
  const publicIp = useSettings((s) => s.publicIp);
  const setPublicIp = useSettings((s) => s.setPublicIp);
  const tempUnit = useSettings((s) => s.tempUnit);
  const setTempUnit = useSettings((s) => s.setTempUnit);
  const byteUnit = useSettings((s) => s.byteUnit);
  const setByteUnit = useSettings((s) => s.setByteUnit);
  const netSpeedUnit = useSettings((s) => s.netSpeedUnit);
  const setNetSpeedUnit = useSettings((s) => s.setNetSpeedUnit);
  const exportSensitive = useSettings((s) => s.exportSensitive);
  const setExportSensitive = useSettings((s) => s.setExportSensitive);
  const tray = useSettings((s) => s.tray);
  const setTray = useSettings((s) => s.setTray);
  const floatingNetSpeed = useSettings((s) => s.floatingNetSpeed);
  const setFloatingNetSpeed = useSettings((s) => s.setFloatingNetSpeed);
  const [fetching, setFetching] = useState(false);
  const [fetchError, setFetchError] = useState<string | null>(null);

  const fetchIp = async () => {
    setFetching(true);
    setFetchError(null);
    try {
      const ip = await getPublicIp();
      setPublicIp(ip);
      if (!ip) setFetchError("No response");
    } catch (e) {
      setFetchError(String(e));
    } finally {
      setFetching(false);
    }
  };

  return (
    <div className="space-y-5">
      <PageHeader title={t("settings_title")} />

      <Card title={t("settings_appearance")}>
        <Row label={t("settings_theme")} hint={t("settings_theme_hint")}>
          <Segmented
            options={[
              { value: "dark", label: "DARK" },
              { value: "light", label: "LIGHT" },
            ]}
            value={theme}
            onChange={(v) => setTheme(v as "dark" | "light")}
          />
        </Row>
        <Row label={t("settings_language")} hint={t("settings_language_hint")}>
          <Segmented
            options={[
              { value: "zh", label: "中文" },
              { value: "en", label: "EN" },
            ]}
            value={lang}
            onChange={(v) => setLang(v as Lang)}
          />
        </Row>
      </Card>

      <Card title={t("settings_geek")}>
        <Row label={t("settings_geek")} hint={t("settings_geek_hint")}>
          <button
            type="button"
            onClick={toggleGeek}
            className={
              "px-3 py-1.5 text-xs rounded-md border font-mono uppercase " +
              (geek
                ? "bg-accent/10 border-accent/40 text-accent"
                : "bg-bg-surface border-border text-text-secondary")
            }
          >
            {geek ? t("common_on") : t("common_off")}
          </button>
        </Row>
      </Card>

      <Card title={t("settings_units")}>
        <Row label={t("settings_units_temp")}>
          <Segmented
            options={[
              { value: "C", label: "°C" },
              { value: "F", label: "°F" },
            ]}
            value={tempUnit}
            onChange={(v) => setTempUnit(v as "C" | "F")}
          />
        </Row>
        <Row label={t("settings_units_bytes")}>
          <Segmented
            options={[
              { value: "binary", label: "GiB" },
              { value: "decimal", label: "GB" },
            ]}
            value={byteUnit}
            onChange={(v) => setByteUnit(v as "binary" | "decimal")}
          />
        </Row>
        <Row label={t("settings_units_netspeed")}>
          <Segmented
            options={[
              { value: "byte", label: "MB/s" },
              { value: "bit", label: "Mbps" },
            ]}
            value={netSpeedUnit}
            onChange={(v) => setNetSpeedUnit(v as "byte" | "bit")}
          />
        </Row>
      </Card>

      <Card title={t("settings_monitor")}>
        <Row label={t("settings_interval")} hint={t("settings_interval_hint")}>
          <Segmented
            options={INTERVALS.map((v) => ({ value: String(v), label: `${v}ms` }))}
            value={String(interval)}
            onChange={(v) => setMonitorInterval(Number(v))}
          />
        </Row>
      </Card>

      <Card title={t("settings_tray_section")}>
        <div className="text-text-tertiary text-xs pb-2">{t("settings_tray_hint")}</div>
        <TrayToggleRow
          label={t("settings_tray_show_cpu")}
          checked={tray.show_cpu}
          onToggle={() => setTray({ show_cpu: !tray.show_cpu })}
        />
        <TrayToggleRow
          label={t("settings_tray_show_memory")}
          checked={tray.show_memory}
          onToggle={() => setTray({ show_memory: !tray.show_memory })}
        />
        <TrayToggleRow
          label={t("settings_tray_show_disk")}
          checked={tray.show_disk}
          onToggle={() => setTray({ show_disk: !tray.show_disk })}
        />
        <TrayToggleRow
          label={t("settings_tray_show_network")}
          checked={tray.show_network}
          onToggle={() => setTray({ show_network: !tray.show_network })}
        />
        <TrayToggleRow
          label={t("settings_tray_show_temperature")}
          checked={tray.show_temperature}
          onToggle={() => setTray({ show_temperature: !tray.show_temperature })}
        />
        {IS_MAC && (
          <TrayToggleRow
            label={t("settings_tray_macos_title")}
            hint={t("settings_tray_macos_title_hint")}
            checked={tray.macos_show_title}
            onToggle={() =>
              setTray({ macos_show_title: !tray.macos_show_title } as Partial<TraySettings>)
            }
          />
        )}
      </Card>

      <Card title={t("settings_floating_section")}>
        <Row label={t("settings_floating_net")} hint={t("settings_floating_net_hint")}>
          <button
            type="button"
            onClick={() => setFloatingNetSpeed(!floatingNetSpeed)}
            className={
              "px-3 py-1.5 text-xs rounded-md border font-mono uppercase " +
              (floatingNetSpeed
                ? "bg-accent/10 border-accent/40 text-accent"
                : "bg-bg-surface border-border text-text-secondary")
            }
          >
            {floatingNetSpeed ? t("common_on") : t("common_off")}
          </button>
        </Row>
      </Card>

      <Card title={t("settings_export_section")}>
        <Row label={t("settings_export_sensitive")} hint={t("settings_export_sensitive_hint")}>
          <button
            type="button"
            onClick={() => setExportSensitive(!exportSensitive)}
            className={
              "px-3 py-1.5 text-xs rounded-md border font-mono uppercase " +
              (exportSensitive
                ? "bg-warning/10 border-warning/40 text-warning"
                : "bg-bg-surface border-border text-text-secondary")
            }
          >
            {exportSensitive ? t("common_on") : t("common_off")}
          </button>
        </Row>
      </Card>

      <Card title={t("settings_network")}>
        <Row label={`${t("net_public_prefix")} IP`} hint={t("settings_public_ip_hint")}>
          <button
            type="button"
            onClick={() => setPublicIpEnabled(!publicIpEnabled)}
            className={
              "px-3 py-1.5 text-xs rounded-md border font-mono uppercase " +
              (publicIpEnabled
                ? "bg-accent/10 border-accent/40 text-accent"
                : "bg-bg-surface border-border text-text-secondary")
            }
          >
            {publicIpEnabled ? t("common_on") : t("common_off")}
          </button>
        </Row>
        {publicIpEnabled && (
          <div className="mt-3 flex items-center gap-3 px-3 py-2 rounded-lg bg-bg-elevated/40 border border-border/50">
            <div className="font-mono text-text-primary text-sm">
              {publicIp ?? "—"}
            </div>
            <div className="ml-auto flex gap-2">
              <button
                type="button"
                onClick={fetchIp}
                disabled={fetching}
                className="px-3 py-1.5 text-xs rounded-md border border-accent/40 bg-accent/10 text-accent disabled:opacity-50"
              >
                {fetching ? t("common_loading") : t("settings_public_ip_fetch")}
              </button>
              <button
                type="button"
                onClick={() => setPublicIp(null)}
                className="px-3 py-1.5 text-xs rounded-md border border-border bg-bg-surface text-text-secondary hover:text-text-primary"
              >
                {t("settings_public_ip_clear")}
              </button>
            </div>
          </div>
        )}
        {fetchError && (
          <div className="mt-2 text-xs text-danger font-mono">{fetchError}</div>
        )}
      </Card>

      <Card title={t("settings_about")}>
        <div className="flex items-start gap-4">
          <Logo size={96} className="shrink-0 ring-1 ring-border/60 shadow-md" />
          <div className="space-y-1 text-sm text-text-secondary min-w-0">
            <div className="text-text-primary text-base font-semibold">PC Specs · v0.1.0</div>
            <div className="text-text-tertiary text-xs">{t("settings_about_desc")}</div>
            <div className="text-text-tertiary text-xs">{t("settings_about_privacy")}</div>
            <div className="text-text-tertiary text-[10px] font-mono uppercase tracking-widest pt-1">
              Windows · macOS · Linux
            </div>
          </div>
        </div>
      </Card>
    </div>
  );
}

interface RowProps {
  label: React.ReactNode;
  hint?: React.ReactNode;
  children?: React.ReactNode;
}

function Row({ label, hint, children }: RowProps) {
  return (
    <div className="flex items-center justify-between py-2 gap-4">
      <div className="min-w-0">
        <div className="text-text-primary text-sm">{label}</div>
        {hint ? <div className="text-text-tertiary text-xs mt-0.5">{hint}</div> : null}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

interface SegmentedProps {
  options: Array<{ value: string; label: string }>;
  value: string;
  onChange: (v: string) => void;
}

interface TrayToggleRowProps {
  label: React.ReactNode;
  hint?: React.ReactNode;
  checked: boolean;
  onToggle: () => void;
}

function TrayToggleRow({ label, hint, checked, onToggle }: TrayToggleRowProps) {
  return (
    <div className="flex items-center justify-between py-1.5 gap-4">
      <div className="min-w-0">
        <div className="text-text-primary text-sm">{label}</div>
        {hint ? <div className="text-text-tertiary text-xs mt-0.5">{hint}</div> : null}
      </div>
      <button
        type="button"
        onClick={onToggle}
        className={
          "shrink-0 px-3 py-1 text-xs rounded-md border font-mono uppercase " +
          (checked
            ? "bg-accent/10 border-accent/40 text-accent"
            : "bg-bg-surface border-border text-text-secondary")
        }
      >
        {checked ? "ON" : "OFF"}
      </button>
    </div>
  );
}

function Segmented({ options, value, onChange }: SegmentedProps) {
  return (
    <div className="flex items-center gap-1 bg-bg-elevated border border-border rounded-md p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          onClick={() => onChange(o.value)}
          className={
            "px-3 py-1 text-xs rounded-sm font-mono " +
            (value === o.value
              ? "bg-accent/15 text-accent"
              : "text-text-secondary hover:text-text-primary")
          }
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}
