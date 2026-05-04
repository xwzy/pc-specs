import { cn } from "@/lib/utils";

interface LogoProps {
  size?: number;
  className?: string;
  rounded?: boolean;
  alt?: string;
}

/**
 * 应用品牌 logo。源图位于 `public/icon.png`（256）/ `public/logo.png`（512）；
 * 大尺寸（≥96）使用 logo.png 以获得更清晰的细节，小尺寸使用 icon.png。
 */
export function Logo({ size = 28, className, rounded = true, alt = "PC Specs" }: LogoProps) {
  const src = size >= 96 ? "/logo.png" : "/icon.png";
  return (
    <img
      src={src}
      alt={alt}
      width={size}
      height={size}
      draggable={false}
      className={cn(
        "object-contain select-none",
        rounded && "rounded-md",
        className,
      )}
      style={{ width: size, height: size }}
    />
  );
}
