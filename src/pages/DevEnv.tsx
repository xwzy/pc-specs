import { useQuery } from "@tanstack/react-query";
import { Copy } from "lucide-react";
import { useMemo, useState } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { getDevEnv } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Empty } from "@/components/ui/Empty";
import { Badge } from "@/components/ui/Badge";
import type { RuntimeInfo } from "@/lib/types";
import { PageHeader } from "@/components/layout/PageHeader";
import { useT } from "@/lib/store";
import type { DictKey } from "@/lib/i18n";

export default function DevEnvPage() {
  const t = useT();
  const { data: env } = useQuery({ queryKey: ["dev-env"], queryFn: getDevEnv });
  const [copied, setCopied] = useState(false);

  const summary = useMemo(() => {
    if (!env) return "";
    const lines: string[] = [];
    const block = (title: string, list: RuntimeInfo[]) => {
      if (!list.length) return;
      lines.push(`## ${title}`);
      list.forEach((r) => {
        lines.push(`- ${r.name}: ${r.version ?? "unknown"}`);
      });
      lines.push("");
    };
    block("Languages", env.languages);
    block("Package Managers", env.package_managers);
    block("VCS", env.vcs);
    block("Editors", env.editors);
    block("Containers", env.containers);
    block("Shells", env.shells);
    if (env.env_keys.length) {
      lines.push("## Env Keys");
      lines.push(env.env_keys.join(", "));
    }
    return lines.join("\n");
  }, [env]);

  if (!env) return null;

  const groups: Array<[DictKey, RuntimeInfo[]]> = [
    ["dev_languages", env.languages],
    ["dev_package_managers", env.package_managers],
    ["dev_vcs", env.vcs],
    ["dev_editors", env.editors],
    ["dev_containers", env.containers],
    ["dev_shells", env.shells],
  ];

  const copy = async () => {
    try {
      await writeText(summary);
    } catch {
      try {
        await navigator.clipboard.writeText(summary);
      } catch {
        /* ignore */
      }
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="space-y-5">
      <PageHeader
        title={t("nav_dev_env")}
        actions={
          <button
            type="button"
            onClick={copy}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-md border border-border bg-bg-surface hover:bg-bg-elevated text-xs text-text-primary"
          >
            <Copy size={12} /> {copied ? t("common_copied") : t("common_copy")}
          </button>
        }
      />

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {groups.map(([key, list]) => (
          <Card key={key} title={t(key)}>
            {list.length === 0 ? (
              <Empty title={t("dev_no_data")} />
            ) : (
              <div className="space-y-2">
                {list.map((r) => (
                  <div
                    key={r.name}
                    className="flex items-center justify-between px-3 py-2 rounded-lg bg-bg-elevated/40 border border-border/50"
                  >
                    <div className="min-w-0">
                      <div className="text-text-primary text-sm font-mono">{r.name}</div>
                      <div className="text-text-tertiary text-[11px] font-mono truncate">
                        {r.path ?? "—"}
                      </div>
                    </div>
                    <Badge tone={r.version ? "accent" : "default"}>{r.version ?? "—"}</Badge>
                  </div>
                ))}
              </div>
            )}
          </Card>
        ))}
      </div>

      {env.env_keys.length > 0 && (
        <Card title={`${t("dev_env_keys")} (${env.env_keys.length})`}>
          <div className="flex flex-wrap gap-1.5">
            {env.env_keys.map((k) => (
              <Badge key={k}>{k}</Badge>
            ))}
          </div>
          <div className="mt-3 text-text-tertiary text-xs">{t("dev_env_keys_hint")}</div>
        </Card>
      )}
    </div>
  );
}
