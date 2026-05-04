import { useState } from "react";
import { Copy, Download, FileJson, FileText, RefreshCw, ShieldAlert } from "lucide-react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { save } from "@tauri-apps/plugin-dialog";
import { exportJson, exportMarkdown, saveExport } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { PageHeader } from "@/components/layout/PageHeader";
import { useSettings, useT } from "@/lib/store";

type Format = "markdown" | "json";

export default function ExportPage() {
  const t = useT();
  const [format, setFormat] = useState<Format>("markdown");
  const [content, setContent] = useState<string>("");
  const [copied, setCopied] = useState(false);
  const [loading, setLoading] = useState(false);
  const [savedTo, setSavedTo] = useState<string | null>(null);
  const exportSensitive = useSettings((s) => s.exportSensitive);
  const setExportSensitive = useSettings((s) => s.setExportSensitive);

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
