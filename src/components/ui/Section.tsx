import { type ReactNode, useState } from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";

interface SectionProps {
  title: ReactNode;
  defaultOpen?: boolean;
  children?: ReactNode;
  className?: string;
}

export function Section({ title, defaultOpen = false, children, className }: SectionProps) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className={cn("rounded-card border border-border bg-bg-surface overflow-hidden", className)}>
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="w-full flex items-center justify-between px-5 py-3 text-left hover:bg-bg-elevated transition-colors"
      >
        <span className="text-text-primary text-[13px] font-medium">{title}</span>
        <ChevronDown size={16} className={cn("transition-transform text-text-secondary", open && "rotate-180")} />
      </button>
      {open && <div className="px-5 pb-5 pt-1">{children}</div>}
    </div>
  );
}
