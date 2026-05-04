import { type ReactNode } from "react";

interface EmptyProps {
  title: string;
  hint?: ReactNode;
  icon?: ReactNode;
}

export function Empty({ title, hint, icon }: EmptyProps) {
  return (
    <div className="flex flex-col items-center justify-center py-10 text-center">
      {icon && <div className="text-text-tertiary mb-3">{icon}</div>}
      <div className="text-text-secondary text-sm">{title}</div>
      {hint && <div className="text-text-tertiary text-xs mt-1.5">{hint}</div>}
    </div>
  );
}
