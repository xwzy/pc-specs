import { type ReactNode } from "react";
import { cn } from "@/lib/utils";

interface BadgeProps {
  tone?: "default" | "accent" | "success" | "warning" | "danger";
  children?: ReactNode;
  className?: string;
}

export function Badge({ tone = "default", children, className }: BadgeProps) {
  const cls =
    tone === "accent"
      ? "badge-accent"
      : tone === "success"
        ? "badge-success"
        : tone === "warning"
          ? "badge-warning"
          : tone === "danger"
            ? "badge-danger"
            : "";
  return <span className={cn("badge", cls, className)}>{children}</span>;
}
