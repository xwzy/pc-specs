import { useQuery } from "@tanstack/react-query";
import { getHost, getOs } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { KeyValueTable } from "@/components/ui/KeyValueTable";
import { Stat } from "@/components/ui/Stat";
import { fmtTimestamp, fmtUptime, nullable } from "@/lib/format";
import { PageHeader } from "@/components/layout/PageHeader";
import { useT } from "@/lib/store";

export default function OsPage() {
  const t = useT();
  const { data: os } = useQuery({ queryKey: ["os"], queryFn: getOs });
  const { data: host } = useQuery({ queryKey: ["host"], queryFn: getHost });
  if (!os || !host) return null;

  return (
    <div className="space-y-5">
      <PageHeader
        title={t("nav_os")}
        description={`${os.name} · ${os.arch}`}
      />

      <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
        <Card>
          <Stat label={t("os_family")} value={os.family} sub={os.name} />
        </Card>
        <Card>
          <Stat
            label={t("spec_uptime")}
            value={fmtUptime(host.uptime_secs)}
            sub={`${t("os_since")} ${fmtTimestamp(host.boot_time * 1000)}`}
          />
        </Card>
        <Card>
          <Stat label={t("spec_hostname")} value={host.hostname} sub={host.username} />
        </Card>
        <Card>
          <Stat label={t("spec_locale")} value={os.locale} sub={nullable(os.desktop)} />
        </Card>
      </div>

      <Card title={t("cpu_details")}>
        <KeyValueTable
          rows={[
            { key: t("os_family"), value: os.family },
            { key: t("os_name"), value: os.name },
            { key: t("os_version"), value: os.version },
            { key: t("spec_kernel"), value: os.kernel },
            { key: t("spec_arch"), value: os.arch },
            { key: t("spec_locale"), value: os.locale },
            { key: t("spec_shell"), value: nullable(os.shell) },
            { key: t("spec_desktop"), value: nullable(os.desktop) },
            { key: t("spec_hostname"), value: host.hostname },
            { key: t("spec_user"), value: host.username },
            { key: t("spec_uptime"), value: fmtUptime(host.uptime_secs) },
            { key: t("spec_boot_time"), value: fmtTimestamp(host.boot_time * 1000) },
            { key: t("os_app_version"), value: host.app_version },
          ]}
        />
      </Card>
    </div>
  );
}
