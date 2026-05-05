import { useEffect } from "react";
import {
  applyTraySettings,
  listenFloatingNetSpeedClosed,
  setFloatingNetSpeed,
  type TraySettings as TraySettingsApi,
} from "./api";
import { useSettings, type TraySettings } from "./store";

/**
 * AppShell 启动时调用一次：
 *  - 把当前持久化的 tray / floating 设置同步给后端，让后端从冷启动恢复到一致状态
 *  - 监听后端发出的 "floating://net-speed-closed" 事件（用户右键关闭 / 主动关窗），
 *    把前端 setting 同步回 disabled
 *  - settings 变化时，自动 push 一次给后端
 *
 * 实现说明：
 *  - 不再使用 `initRef` latch 区分"首次"和"后续" —— 那会让 React StrictMode 下
 *    第二次 mount 跳过 listen 注册，且 cleanup 时 listen Promise 还没 resolve
 *    会丢 unlisten 句柄。
 *  - 每个 effect 自己负责注册 / 清理。push 给后端的 invoke 都是幂等的（apply_tray_settings
 *    重建菜单一次代价可忽略；set_floating_net_speed 在状态匹配时是 no-op），
 *    StrictMode 双 mount 不会造成可见副作用。
 */
export function useTrayFloating() {
  const tray = useSettings((s) => s.tray);
  const floating = useSettings((s) => s.floatingNetSpeed);
  const setFloatingState = useSettings((s) => s.setFloatingNetSpeed);

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    listenFloatingNetSpeedClosed(() => {
      if (!alive) return;
      setFloatingState(false);
    })
      .then((u) => {
        const fn = u as unknown as () => void;
        if (!alive) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch(() => undefined);

    return () => {
      alive = false;
      unlisten?.();
    };
  }, [setFloatingState]);

  useEffect(() => {
    void applyTraySettings(toApi(tray));
  }, [tray]);

  useEffect(() => {
    void setFloatingNetSpeed(floating);
  }, [floating]);
}

function toApi(t: TraySettings): TraySettingsApi {
  return {
    show_cpu: t.show_cpu,
    show_memory: t.show_memory,
    show_disk: t.show_disk,
    show_network: t.show_network,
    show_temperature: t.show_temperature,
    macos_show_title: t.macos_show_title,
  };
}
