import { useQuery } from "@tanstack/react-query";
import { getMotherboard } from "@/lib/api";
import { Card } from "@/components/ui/Card";
import { Empty } from "@/components/ui/Empty";
import { KeyValueTable } from "@/components/ui/KeyValueTable";
import { nullable } from "@/lib/format";
import { PageHeader } from "@/components/layout/PageHeader";
import { useT } from "@/lib/store";

export default function MotherboardPage() {
  const t = useT();
  const { data: mb } = useQuery({ queryKey: ["motherboard"], queryFn: getMotherboard });

  return (
    <div className="space-y-5">
      <PageHeader title={t("nav_motherboard")} />
      <Card>
        {!mb ? (
          <Empty title={t("mb_no_data")} hint={t("mb_no_data_hint")} />
        ) : (
          <KeyValueTable
            rows={[
              { key: t("spec_vendor"), value: nullable(mb.vendor) },
              { key: t("spec_brand"), value: nullable(mb.model) },
              { key: t("mb_board_version"), value: nullable(mb.version) },
              { key: t("spec_serial"), value: nullable(mb.serial) },
              { key: t("mb_bios_vendor"), value: nullable(mb.bios_vendor) },
              { key: t("mb_bios_version"), value: nullable(mb.bios_version) },
              { key: t("mb_bios_date"), value: nullable(mb.bios_date) },
              { key: t("mb_chassis"), value: nullable(mb.chassis) },
            ]}
          />
        )}
      </Card>
    </div>
  );
}
