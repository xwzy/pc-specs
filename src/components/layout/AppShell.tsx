import { type ReactNode } from "react";
import { Sidebar } from "./Sidebar";
import { Topbar } from "./Topbar";
import { useTrayFloating } from "@/lib/useTrayFloating";

interface AppShellProps {
  children: ReactNode;
}

export function AppShell({ children }: AppShellProps) {
  // 主窗口挂载时把托盘 / 悬浮窗状态同步给后端 + 监听悬浮窗被外部关闭事件。
  // 注意：floating 窗口走 main.tsx 的另一分支，不挂 AppShell，所以这里不会重复执行。
  useTrayFloating();
  return (
    <div className="h-full w-full flex bg-bg-base">
      <Sidebar />
      <div className="flex-1 flex flex-col overflow-hidden">
        <Topbar />
        <main className="flex-1 overflow-y-auto">
          <div className="max-w-[1440px] mx-auto px-6 py-6 w-full">{children}</div>
        </main>
      </div>
    </div>
  );
}
