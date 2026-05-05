import { useRef, useState } from "react";
import {
  Copy,
  Download,
  FileJson,
  FileText,
  Image as ImageIcon,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { save } from "@tauri-apps/plugin-dialog";
import { writeFile } from "@tauri-apps/plugin-fs";
import { toPng } from "html-to-image";
import { exportJson, exportMarkdown, getFullSnapshot, saveExport } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { PageHeader } from "@/components/layout/PageHeader";
import { useSettings, useT } from "@/lib/store";
import { ExportPoster } from "@/components/ExportPoster";
import type { SystemSnapshot } from "@/lib/types";

type Format = "markdown" | "json";

const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export default function ExportPage() {
  const t = useT();
  const [format, setFormat] = useState<Format>("markdown");
  const [content, setContent] = useState<string>("");
  const [copied, setCopied] = useState(false);
  const [loading, setLoading] = useState(false);
  const [savedTo, setSavedTo] = useState<string | null>(null);
  const exportSensitive = useSettings((s) => s.exportSensitive);
  const setExportSensitive = useSettings((s) => s.setExportSensitive);
  const [posterSnap, setPosterSnap] = useState<SystemSnapshot | null>(null);
  const [pngStatus, setPngStatus] = useState<"idle" | "rendering" | "done" | "error">(
    "idle",
  );
  const [pngError, setPngError] = useState<string | null>(null);
  const posterRef = useRef<HTMLDivElement | null>(null);

  const refresh = async (f: Format = format) => {
    setLoading(true);
    setFormat(f);
    setSavedTo(null);
    try {
      const text =
        f === "markdown"
          ? await exportMarkdown(exportSensitive)
          : await exportJson(true, exportSensitive);
      setContent(text);
    } finally {
      setLoading(false);
    }
  };

  const copy = async () => {
    if (!content) {
      await refresh();
    }
    try {
      await writeText(content);
    } catch {
      try {
        await navigator.clipboard.writeText(content);
      } catch {
        /* ignore */
      }
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  // PNG 导出流程：
  //  1. 拉一份不含敏感数据（默认）/含敏感数据（按 settings）的 SystemSnapshot
  //  2. 把它写入隐藏的 ExportPoster；等一帧让 React 完成 DOM commit
  //  3. html-to-image 的 toPng 把 DOM → PNG dataURL
  //  4. 通过 plugin-fs.writeFile 写到用户选择的路径
  const exportPng = async () => {
    setPngStatus("rendering");
    setPngError(null);
    setSavedTo(null);
    try {
      // 注：当前 PNG 不区分 sensitive，因为 Poster 渲染的字段不含 MAC / 序列号 / 公网 IP；
      // 主机名是必备身份信息，故不再脱敏。后续若加更多字段，可参考 Markdown 路径添加敏感分支。
      const snap = await getFullSnapshot();
      setPosterSnap(snap);
      // 等一帧让 ExportPoster 完成首次布局；嵌套两次 rAF 兼容 Strict Mode 下的双重渲染。
      await new Promise<void>((r) =>
        requestAnimationFrame(() => requestAnimationFrame(() => r())),
      );
      const node = posterRef.current;
      if (!node) {
        throw new Error("poster node not mounted");
      }
      const dataUrl = await toPng(node, {
        cacheBust: true,
        // 给 toPng 一个稳定背景，避免某些 GPU 驱动下抗锯齿带来的灰边
        backgroundColor: "#0a0d12",
        // 提高 pixelRatio 让 PNG 在 Retina 显示器上更清晰；2 是 Retina 实际像素密度
        pixelRatio: 2,
      });
      const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
      const defaultPath = `pc-specs-${ts}.png`;
      if (!isTauri) {
        // 浏览器预览模式：直接触发下载
        const a = document.createElement("a");
        a.href = dataUrl;
        a.download = defaultPath;
        a.click();
        setPngStatus("done");
        return;
      }
      const path = await save({
        defaultPath,
        filters: [{ name: "PNG", extensions: ["png"] }],
      });
      if (!path) {
        setPngStatus("idle");
        return;
      }
      // dataUrl 形如 "data:image/png;base64,xxxx"；切掉 schema 前缀后 base64 解码为字节
      const base64 = dataUrl.split(",", 2)[1] ?? "";
      const bytes = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
      await writeFile(path, bytes);
      setSavedTo(path);
      setPngStatus("done");
    } catch (e) {
      setPngStatus("error");
      setPngError(String(e));
    } finally {
      // 不立刻清空 posterSnap：保留 ref 一会儿，给浏览器 GPU 缓存留余地。
      setTimeout(() => setPosterSnap(null), 1500);
    }
  };

  const saveFile = async () => {
    if (!content) {
      await refresh();
    }
    const ext = format === "markdown" ? "md" : "json";
    const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
    try {
      const path = await save({
        defaultPath: `pc-specs-${ts}.${ext}`,
        filters: [
          format === "markdown"
            ? { name: "Markdown", extensions: ["md"] }
            : { name: "JSON", extensions: ["json"] },
        ],
      });
      if (!path) return;
      await saveExport(path, content);
      setSavedTo(path);
    } catch (e) {
      setSavedTo(`Error: ${String(e)}`);
    }
  };

  return (
    <div className="space-y-5">
      <PageHeader
        title={t("export_title")}
        description={t("export_desc")}
        actions={
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => refresh("markdown")}
              className={
                "flex items-center gap-1.5 px-3 py-1.5 rounded-md border text-xs " +
                (format === "markdown"
                  ? "bg-accent/10 border-accent/40 text-accent"
                  : "bg-bg-surface border-border text-text-secondary hover:text-text-primary")
              }
            >
              <FileText size={12} /> {t("export_btn_md")}
            </button>
            <button
              type="button"
              onClick={() => refresh("json")}
              className={
                "flex items-center gap-1.5 px-3 py-1.5 rounded-md border text-xs " +
                (format === "json"
                  ? "bg-accent/10 border-accent/40 text-accent"
                  : "bg-bg-surface border-border text-text-secondary hover:text-text-primary")
              }
            >
              <FileJson size={12} /> {t("export_btn_json")}
            </button>
            <button
              type="button"
              onClick={() => refresh()}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-md border border-border bg-bg-surface text-xs text-text-secondary hover:text-text-primary"
              disabled={loading}
            >
              <RefreshCw size={12} className={loading ? "animate-spin" : ""} />{" "}
              {t("export_btn_refresh")}
            </button>
            <button
              type="button"
              onClick={copy}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-md border border-accent/40 bg-accent/10 text-accent text-xs"
            >
              <Copy size={12} /> {copied ? t("common_copied") : t("export_btn_copy")}
            </button>
            <button
              type="button"
              onClick={saveFile}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-md border border-accent/40 bg-accent/10 text-accent text-xs"
            >
              <Download size={12} /> {t("export_btn_save")}
            </button>
            <button
              type="button"
              onClick={exportPng}
              disabled={pngStatus === "rendering"}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-md border border-accent-2/40 bg-accent-2/10 text-accent-2 text-xs disabled:opacity-50"
              title={t("export_png_hint")}
            >
              <ImageIcon size={12} />
              {pngStatus === "rendering"
                ? t("export_png_rendering")
                : t("export_btn_png")}
            </button>
          </div>
        }
      />

      <Card>
        <div className="flex items-center gap-3 flex-wrap">
          <ShieldAlert
            size={14}
            className={exportSensitive ? "text-warning" : "text-text-tertiary"}
          />
          <div className="text-sm text-text-secondary">
            {exportSensitive ? (
              <>
                {t("export_sensitive_on")}
                <span className="text-warning font-mono">{t("export_sensitive_keys")}</span>
              </>
            ) : (
              <>{t("export_sensitive_off")}</>
            )}
          </div>
          <button
            type="button"
            onClick={() => setExportSensitive(!exportSensitive)}
            className={
              "ml-auto px-3 py-1 rounded-md text-xs font-mono border " +
              (exportSensitive
                ? "bg-warning/10 border-warning/40 text-warning"
                : "bg-bg-surface border-border text-text-secondary")
            }
          >
            {exportSensitive ? t("common_on") : t("common_off")}
          </button>
        </div>
      </Card>

      {savedTo && (
        <div className="text-xs text-text-secondary font-mono px-3 py-2 rounded-lg bg-bg-elevated/40 border border-border/50">
          ✓ {t("export_saved")} → {savedTo}
        </div>
      )}

      {pngStatus === "error" && pngError && (
        <div className="text-xs text-danger font-mono px-3 py-2 rounded-lg bg-danger/10 border border-danger/30">
          ✗ {t("export_png_error")} — {pngError}
        </div>
      )}
      {pngStatus === "done" && !savedTo && (
        <div className="text-xs text-success font-mono px-3 py-2 rounded-lg bg-success/10 border border-success/30">
          ✓ {t("export_png_done")}
        </div>
      )}

      {/*
        离屏渲染 Poster：position:fixed + 移到 viewport 之外，但保持 visibility:visible，
        这样 html-to-image 仍能读到 layout / computed style。pointer-events:none 防止
        意外干扰用户。仅在用户点 PNG 按钮期间存在 DOM。
      */}
      {posterSnap && (
        <div
          aria-hidden
          style={{
            position: "fixed",
            left: -100000,
            top: 0,
            pointerEvents: "none",
            zIndex: -1,
          }}
        >
          <ExportPoster ref={posterRef} snap={posterSnap} />
        </div>
      )}

      <Card
        title={
          <span className="flex items-center gap-2">
            {t("export_preview")}
            <Badge tone="accent">{format.toUpperCase()}</Badge>
            <span className="text-text-tertiary text-xs font-mono">
              {content.length} {t("export_chars_suffix")}
            </span>
            {exportSensitive && <Badge tone="warning">SENSITIVE</Badge>}
          </span>
        }
      >
        {!content ? (
          <div className="text-text-tertiary text-sm py-8 text-center">
            {t("export_empty_hint")}
          </div>
        ) : (
          <pre className="text-xs leading-6 font-mono text-text-primary bg-bg-base/60 border border-border rounded-md p-4 overflow-auto max-h-[640px] whitespace-pre">
            {content}
          </pre>
        )}
      </Card>
    </div>
  );
}
