import { type ReactNode } from "react";

export interface KvRow {
  key: string;
  value: ReactNode;
}

interface KeyValueTableProps {
  rows: KvRow[];
}

export function KeyValueTable({ rows }: KeyValueTableProps) {
  return (
    <div>
      {rows.map((r, i) => (
        <div key={i} className="kv-row">
          <div className="kv-key">{r.key}</div>
          <div className="kv-val">{r.value}</div>
        </div>
      ))}
    </div>
  );
}
