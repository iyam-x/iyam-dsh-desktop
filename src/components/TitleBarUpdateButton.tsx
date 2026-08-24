import { useRef } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Menu, MenuItem, PredefinedMenuItem } from "@tauri-apps/api/menu";
import { LogicalPosition } from "@tauri-apps/api/dpi";

// 更新图标（Segoe Fluent/MDL2 Assets，与标题栏其他按钮同字体）
const ICON_UPDATE = ""; // \uE895 Refresh

type UpdateInfo = {
  installed: string;
  latest: string;
  has_update: boolean;
  managed: boolean;
};

// 阻止冒泡，避免触发标题栏拖拽
function stopDragPropagation(e: ReactMouseEvent) {
  e.stopPropagation();
}

export function TitleBarUpdateButton() {
  const btnRef = useRef<HTMLButtonElement>(null);

  async function handleClick(e: ReactMouseEvent) {
    e.stopPropagation();
    const btn = btnRef.current;
    if (!btn) return;

    const rect = btn.getBoundingClientRect();
    const at = new LogicalPosition(rect.left, rect.bottom + 4);

    let info: UpdateInfo | null = null;
    try {
      info = await invoke<UpdateInfo>("check_for_update");
    } catch (err) {
      // 网络/命令失败：菜单内给出可读提示，不弹窗打扰
    }

    const items: Array<MenuItem | PredefinedMenuItem> = [];

    if (!info) {
      items.push(
        await MenuItem.new({
          text: "检查更新失败",
          enabled: false,
        })
      );
    } else {
      items.push(
        await MenuItem.new({ text: `dsh  当前 v${info.installed}`, enabled: false }),
        await MenuItem.new({ text: `最新 v${info.latest}`, enabled: false }),
        await PredefinedMenuItem.new({ item: "Separator" })
      );

      if (info.managed && info.has_update) {
        items.push(
          await MenuItem.new({
            text: "下载并更新（下次启动生效）",
            action: async () => {
              try {
                await invoke("trigger_dsh_update");
                await invoke("notify", {
                  title: "dsh 更新已备货",
                  body: "重启应用后生效",
                });
              } catch (err) {
                await invoke("notify", {
                  title: "dsh 更新失败",
                  body: String(err ?? "未知错误"),
                });
              }
            },
          })
        );
      } else if (info.managed && !info.has_update) {
        items.push(await MenuItem.new({ text: "已是最新版本", enabled: false }));
      } else {
        // 系统/PATH 的 dsh，app 不托管
        items.push(
          await MenuItem.new({
            text: "由你的环境管理 · 终端 npm i -g @deepseek-ai/dsh",
            enabled: false,
          })
        );
      }
    }

    const menu = await Menu.new({ items });
    await menu.popup(at).catch(() => {});
  }

  return (
    <button
      ref={btnRef}
      className="tb-btn tb-update"
      onClick={handleClick}
      onMouseDown={stopDragPropagation}
      aria-label="检查 dsh 更新"
      title="检查 dsh 更新"
    >
      <span className="tb-icon">{ICON_UPDATE}</span>
    </button>
  );
}
