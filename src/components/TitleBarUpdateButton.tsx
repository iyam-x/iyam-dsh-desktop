import { useRef } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Menu, MenuItem, PredefinedMenuItem } from "@tauri-apps/api/menu";
import { LogicalPosition } from "@tauri-apps/api/dpi";

// 下拉箭头用内联 SVG 渲染，跨平台一致（不依赖 Windows 专属的 Segoe MDL2 字体）。
function ChevronDown() {
  return (
    <svg
      className="tb-chevron"
      width="12"
      height="12"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M4 6l4 4 4-4" />
    </svg>
  );
}

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

    // 始终提供「检查更新」操作：点击重新向 Rust 查询并提示结果。
    items.push(
      await MenuItem.new({
        text: "检查更新",
        action: async () => {
          try {
            const r = await invoke<UpdateInfo>("check_for_update");
            await invoke("notify", {
              title: r.has_update ? "发现新版本" : "已是最新版本",
              body: r.has_update
                ? `dsh v${r.installed} → v${r.latest}`
                : `dsh 当前 v${r.installed}`,
            });
          } catch (err) {
            await invoke("notify", {
              title: "检查更新失败",
              body: String(err ?? "未知错误"),
            });
          }
        },
      })
    );

    if (!info) {
      items.push(
        await MenuItem.new({
          text: "检查更新失败",
          enabled: false,
        })
      );
    } else {
      items.push(
        await PredefinedMenuItem.new({ item: "Separator" }),
        await MenuItem.new({ text: `dsh  当前 v${info.installed}`, enabled: false }),
        await MenuItem.new({ text: `最新 v${info.latest}`, enabled: false }),
        await PredefinedMenuItem.new({ item: "Separator" })
      );

      if (info.has_update) {
        // 有新版本：无论是否由本应用托管，都给出更新入口。
        if (info.managed) {
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
        } else {
          // 非本应用托管：提示在终端更新（可执行操作，点击复制提示）。
          items.push(
            await MenuItem.new({
              text: "在终端更新：npm i -g @deepseek-ai/dsh",
              action: async () => {
                await invoke("notify", {
                  title: "请手动更新 dsh",
                  body: "终端运行：npm i -g @deepseek-ai/dsh",
                });
              },
            })
          );
        }
      } else {
        items.push(await MenuItem.new({ text: "已是最新版本", enabled: false }));
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
      <ChevronDown />
    </button>
  );
}
