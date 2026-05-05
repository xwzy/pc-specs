import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import App from "./App";
import FloatingNetSpeed from "./pages/FloatingNetSpeed";
import "./index.css";

// 同一份前端 bundle 服务两类窗口：
//   - 主窗口 (label "main") → 走 BrowserRouter + AppShell
//   - 悬浮窗 (label "floating-net-speed") → URL 带 hash `#/floating/net-speed`，
//     渲染独立的极简 UI，不挂 BrowserRouter / AppShell / QueryClient
//
// hash 在初始 navigation 后立刻可读，不会出现 race。
const isFloatingNetSpeed =
  typeof window !== "undefined" &&
  window.location.hash.startsWith("#/floating/net-speed");

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
      staleTime: 5_000,
      retry: 1,
    },
  },
});

const root = ReactDOM.createRoot(document.getElementById("root")!);

if (isFloatingNetSpeed) {
  root.render(
    <React.StrictMode>
      <FloatingNetSpeed />
    </React.StrictMode>,
  );
} else {
  root.render(
    <React.StrictMode>
      <QueryClientProvider client={queryClient}>
        <BrowserRouter>
          <App />
        </BrowserRouter>
      </QueryClientProvider>
    </React.StrictMode>,
  );
}
