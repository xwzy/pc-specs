/**
 * ExportPoster：用于「PNG 长图导出」的离屏渲染组件。
 *
 * 设计要点：
 *  1. 全部走内联 hex 色 / px 尺寸，不依赖 Tailwind 主题变量 —— html-to-image
 *     会读 computed style，多数情况能解析 CSS 变量，但深色背景下若用户
 *     强制系统色板可能错色，内联一劳永逸。
 *  2. 固定宽度 1200px，高度按内容自适应；导出长图不需要响应式。
 *  3. 全字段 fallback "—"，缺数据也排版整齐。
 *  4. 底部 watermark 写明 PC Specs + 版本 + 生成时间，便于发到论坛溯源。
 */

import { forwardRef } from "react";
import type { SystemSnapshot } from "@/lib/types";
import { fmtBytes, fmtHz, fmtUptime } from "@/lib/format";

interface Props {
  snap: SystemSnapshot;
}

const POSTER_W = 1200;
const COLORS = {
  bg0: "#0a0d12",
  bg1: "#11151c",
  bg2: "#1d2330",
  border: "#222a36",
  borderS: "#2c3645",
  text: "#e7ecf3",
  text2: "#9aa6b6",
  text3: "#5b6678",
  accent: "#22d3ee",
  accent2: "#a78bfa",
  ok: "#34d399",
  warn: "#fbbf24",
  danger: "#f87171",
};

export const ExportPoster = forwardRef<HTMLDivElement, Props>(function ExportPoster(
  { snap },
  ref,
) {
  const memPct = snap.memory.total_bytes > 0
    ? (snap.memory.used_bytes / snap.memory.total_bytes) * 100
    : 0;
  const primaryStorage = snap.storages[0];
  const storagePct = primaryStorage && primaryStorage.total_bytes > 0
    ? (primaryStorage.used_bytes / primaryStorage.total_bytes) * 100
    : 0;
  const gpu = snap.gpus[0];
  const memModule = snap.memory.modules.find((m) => m.kind || m.speed_mt_s);
  const memDetail = [memModule?.kind, memModule?.speed_mt_s ? `${memModule.speed_mt_s} MT/s` : null]
    .filter(Boolean)
    .join(" · ");
  const display = snap.displays.find((d) => d.is_primary) ?? snap.displays[0];
  const primaryNet =
    snap.network.interfaces.find((i) => !i.is_loopback && i.ipv4.length > 0) ??
    snap.network.interfaces.find((i) => !i.is_loopback);

  const generatedAt = new Date().toLocaleString();

  return (
    <div
      ref={ref}
      style={{
        width: POSTER_W,
        background: `linear-gradient(135deg, ${COLORS.bg0} 0%, ${COLORS.bg1} 60%, ${COLORS.bg0} 100%)`,
        color: COLORS.text,
        fontFamily:
          "'Inter','Segoe UI Variable','PingFang SC','Noto Sans CJK SC',system-ui,sans-serif",
        padding: "48px 56px",
        boxSizing: "border-box",
      }}
    >
      <Header snap={snap} />

      <div style={{ height: 28 }} />

      <Grid>
        <SpecRow label="CPU" highlight>
          <Big>{snap.cpu.brand}</Big>
          <Sub>
            {`${snap.cpu.physical_cores}P / ${snap.cpu.logical_cores}T · ${snap.cpu.arch}`}
            {snap.cpu.max_frequency_hz
              ? ` · ${fmtHz(snap.cpu.max_frequency_hz)}`
              : ""}
            {snap.cpu.cache_l3_bytes
              ? ` · L3 ${fmtBytes(snap.cpu.cache_l3_bytes)}`
              : ""}
          </Sub>
        </SpecRow>

        <SpecRow label="Memory">
          <Big>{fmtBytes(snap.memory.total_bytes)}</Big>
          <Sub>
            {memDetail || `${snap.memory.modules.length} module(s)`}
            {` · used ${fmtBytes(snap.memory.used_bytes)} (${memPct.toFixed(0)}%)`}
          </Sub>
        </SpecRow>

        <SpecRow label="GPU">
          {gpu ? (
            <>
              <Big>{`${gpu.vendor} · ${gpu.name}`}</Big>
              <Sub>
                {gpu.is_discrete ? "Discrete" : "Integrated"}
                {gpu.backend ? ` · ${gpu.backend}` : ""}
                {gpu.vram_total_bytes ? ` · VRAM ${fmtBytes(gpu.vram_total_bytes)}` : ""}
                {gpu.driver ? ` · drv ${gpu.driver}` : ""}
              </Sub>
            </>
          ) : (
            <Big style={{ color: COLORS.text3 }}>No GPU detected</Big>
          )}
        </SpecRow>

        {snap.gpus.slice(1, 3).map((g, i) => (
          <SpecRow key={i} label={`GPU #${i + 2}`}>
            <Big>{`${g.vendor} · ${g.name}`}</Big>
            <Sub>
              {g.is_discrete ? "Discrete" : "Integrated"}
              {g.backend ? ` · ${g.backend}` : ""}
              {g.vram_total_bytes ? ` · VRAM ${fmtBytes(g.vram_total_bytes)}` : ""}
            </Sub>
          </SpecRow>
        ))}

        <SpecRow label="Storage">
          {primaryStorage ? (
            <>
              <Big>{primaryStorage.name}</Big>
              <Sub>
                {`${primaryStorage.kind} · ${fmtBytes(primaryStorage.total_bytes)}`}
                {` · used ${fmtBytes(primaryStorage.used_bytes)} (${storagePct.toFixed(0)}%)`}
                {primaryStorage.smart_health
                  ? ` · SMART ${primaryStorage.smart_health}`
                  : ""}
              </Sub>
            </>
          ) : (
            <Big style={{ color: COLORS.text3 }}>No disk detected</Big>
          )}
        </SpecRow>

        {snap.storages.slice(1, 4).map((s, i) => (
          <SpecRow key={i} label={`Disk #${i + 2}`}>
            <Big>{s.name}</Big>
            <Sub>
              {`${s.kind} · ${fmtBytes(s.total_bytes)} · used ${fmtBytes(s.used_bytes)}`}
              {s.smart_health ? ` · SMART ${s.smart_health}` : ""}
            </Sub>
          </SpecRow>
        ))}

        <SpecRow label="Motherboard">
          {snap.motherboard ? (
            <>
              <Big>
                {[snap.motherboard.vendor, snap.motherboard.model]
                  .filter(Boolean)
                  .join(" ") || "—"}
              </Big>
              <Sub>
                {[
                  snap.motherboard.bios_vendor,
                  snap.motherboard.bios_version,
                  snap.motherboard.bios_date,
                  snap.motherboard.chassis,
                ]
                  .filter(Boolean)
                  .join(" · ") || "BIOS info unavailable"}
              </Sub>
            </>
          ) : (
            <Big style={{ color: COLORS.text3 }}>Motherboard info unavailable</Big>
          )}
        </SpecRow>

        <SpecRow label="OS">
          <Big>{`${snap.os.name} ${snap.os.version}`.trim()}</Big>
          <Sub>
            {`${snap.os.family} · ${snap.os.arch}`}
            {snap.os.kernel ? ` · kernel ${snap.os.kernel}` : ""}
            {snap.os.desktop ? ` · ${snap.os.desktop}` : ""}
          </Sub>
        </SpecRow>

        <SpecRow label="Display">
          {display ? (
            <>
              <Big>
                {`${display.width_px}×${display.height_px}`}
                {display.refresh_hz ? ` @ ${display.refresh_hz} Hz` : ""}
              </Big>
              <Sub>
                {display.name || "Primary"}
                {display.scale_factor && display.scale_factor !== 1
                  ? ` · scale ${display.scale_factor.toFixed(2)}×`
                  : ""}
                {snap.displays.length > 1
                  ? ` · ${snap.displays.length} displays`
                  : ""}
              </Sub>
            </>
          ) : (
            <Big style={{ color: COLORS.text3 }}>No display</Big>
          )}
        </SpecRow>

        <SpecRow label="Network">
          {primaryNet ? (
            <>
              <Big>
                {primaryNet.name}
                {primaryNet.ipv4[0] ? ` · ${primaryNet.ipv4[0]}` : ""}
              </Big>
              <Sub>
                {primaryNet.kind.toUpperCase()}
                {primaryNet.link_speed_mbps
                  ? ` · ${primaryNet.link_speed_mbps} Mbps`
                  : ""}
                {snap.network.default_gateway
                  ? ` · gw ${snap.network.default_gateway}`
                  : ""}
              </Sub>
            </>
          ) : (
            <Big style={{ color: COLORS.text3 }}>No active interface</Big>
          )}
        </SpecRow>

        {snap.battery && (
          <SpecRow label="Battery">
            <Big>{`${snap.battery.percentage.toFixed(0)}% · ${snap.battery.state}`}</Big>
            <Sub>
              {snap.battery.cycle_count != null
                ? `${snap.battery.cycle_count} cycles`
                : "—"}
              {snap.battery.full_capacity_mwh != null
                ? ` · ${(snap.battery.full_capacity_mwh / 1000).toFixed(0)} Wh full`
                : ""}
            </Sub>
          </SpecRow>
        )}
      </Grid>

      <div style={{ height: 28 }} />

      {/* Sensors snapshot：温度优先，最多 8 个 */}
      {snap.sensors.length > 0 && (
        <div
          style={{
            background: COLORS.bg1,
            border: `1px solid ${COLORS.border}`,
            borderRadius: 14,
            padding: "20px 24px",
          }}
        >
          <SectionTitle>Sensors snapshot</SectionTitle>
          <div
            style={{
              marginTop: 12,
              display: "grid",
              gridTemplateColumns: "repeat(4, 1fr)",
              gap: 12,
            }}
          >
            {snap.sensors
              .filter((s) => s.kind === "temperature")
              .slice(0, 8)
              .map((s, i) => (
                <div
                  key={i}
                  style={{
                    border: `1px solid ${COLORS.border}`,
                    background: COLORS.bg2,
                    borderRadius: 10,
                    padding: "10px 12px",
                    display: "flex",
                    flexDirection: "column",
                    gap: 4,
                  }}
                >
                  <div
                    style={{
                      color: COLORS.text3,
                      fontSize: 10,
                      letterSpacing: 1.5,
                      textTransform: "uppercase",
                    }}
                  >
                    {s.source}
                  </div>
                  <div style={{ color: COLORS.text2, fontSize: 12, fontFamily: "monospace" }}>
                    {s.label}
                  </div>
                  <div
                    style={{
                      color: COLORS.text,
                      fontSize: 22,
                      fontFamily: "monospace",
                      fontVariantNumeric: "tabular-nums",
                    }}
                  >
                    {s.value.toFixed(1)}°C
                  </div>
                </div>
              ))}
          </div>
        </div>
      )}

      <Footer generatedAt={generatedAt} version={snap.host.app_version} />
    </div>
  );
});

function Header({ snap }: { snap: SystemSnapshot }) {
  const familyInitial = snap.os.family.charAt(0).toUpperCase() || "?";
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 20,
        paddingBottom: 24,
        borderBottom: `1px solid ${COLORS.border}`,
      }}
    >
      <div
        style={{
          width: 64,
          height: 64,
          borderRadius: 16,
          background: `linear-gradient(135deg, ${COLORS.accent}, ${COLORS.accent2})`,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          color: COLORS.bg0,
          fontWeight: 800,
          fontSize: 32,
        }}
      >
        {familyInitial}
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{ fontSize: 32, fontWeight: 700, letterSpacing: -0.5 }}
        >{snap.host.hostname}</div>
        <div
          style={{
            marginTop: 4,
            fontSize: 14,
            color: COLORS.text2,
            fontFamily: "monospace",
          }}
        >
          {`${snap.os.name} · ${snap.os.version} · ${snap.os.arch}`}
          {` · up ${fmtUptime(snap.host.uptime_secs)}`}
        </div>
      </div>
      <div
        style={{
          padding: "8px 14px",
          borderRadius: 999,
          background: `${COLORS.accent}1A`,
          border: `1px solid ${COLORS.accent}66`,
          color: COLORS.accent,
          fontSize: 13,
          fontFamily: "monospace",
          letterSpacing: 1,
          textTransform: "uppercase",
        }}
      >
        PC Specs Report
      </div>
    </div>
  );
}

function Grid({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>{children}</div>
  );
}

function SpecRow({
  label,
  children,
  highlight,
}: {
  label: string;
  children: React.ReactNode;
  highlight?: boolean;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "stretch",
        gap: 16,
        padding: "16px 20px",
        background: COLORS.bg1,
        border: `1px solid ${highlight ? `${COLORS.accent}55` : COLORS.border}`,
        borderRadius: 14,
      }}
    >
      <div
        style={{
          width: 110,
          color: highlight ? COLORS.accent : COLORS.text3,
          fontSize: 12,
          letterSpacing: 1.5,
          textTransform: "uppercase",
          fontFamily: "monospace",
          fontWeight: 600,
          paddingTop: 4,
        }}
      >
        {label}
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>{children}</div>
    </div>
  );
}

function Big({
  children,
  style,
}: {
  children: React.ReactNode;
  style?: React.CSSProperties;
}) {
  return (
    <div
      style={{
        fontSize: 22,
        fontWeight: 600,
        color: COLORS.text,
        lineHeight: 1.25,
        ...style,
      }}
    >
      {children}
    </div>
  );
}

function Sub({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        marginTop: 4,
        fontSize: 13,
        color: COLORS.text2,
        fontFamily: "monospace",
        lineHeight: 1.5,
      }}
    >
      {children}
    </div>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        fontSize: 12,
        letterSpacing: 1.5,
        textTransform: "uppercase",
        color: COLORS.text3,
        fontFamily: "monospace",
        fontWeight: 600,
      }}
    >
      {children}
    </div>
  );
}

function Footer({
  generatedAt,
  version,
}: {
  generatedAt: string;
  version: string;
}) {
  return (
    <div
      style={{
        marginTop: 28,
        paddingTop: 16,
        borderTop: `1px solid ${COLORS.border}`,
        display: "flex",
        justifyContent: "space-between",
        alignItems: "center",
        fontSize: 11,
        fontFamily: "monospace",
        color: COLORS.text3,
        letterSpacing: 1,
        textTransform: "uppercase",
      }}
    >
      <span>{`Generated by PC Specs v${version}`}</span>
      <span>{generatedAt}</span>
    </div>
  );
}
