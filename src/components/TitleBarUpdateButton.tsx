import { useEffect, useRef, useState } from "react";
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

type Toast = { text: string; kind: "info" | "ok" | "err" } | null;

// 阻止冒泡，避免触发标题栏拖拽
function stopDragPropagation(e: ReactMouseEvent) {
  e.stopPropagation();
}

export function TitleBarUpdateButton() {
  const btnRef = useRef<HTMLButtonElement>(null);
  // 应用内 toast：点「检查更新」的即时反馈（原生菜单/系统通知都可能被忽略或拦截，
  // toast 画在窗口内保证可见）。
  const [toast, setToast] = useState<Toast>(null);
  const toastTimer = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (toastTimer.current) window.clearTimeout(toastTimer.current);
    };
  }, []);

  function showToast(text: string, kind: "info" | "ok" | "err" = "info") {
    setToast({ text, kind });
    if (toastTimer.current) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 2600);
  }

  async function doCheck(): Promise<UpdateInfo | null> {
    try {
      return await invoke<UpdateInfo>("check_for_update");
    } catch (err) {
      showToast(`检查更新失败：${String(err ?? "未知错误")}`, "err");
      return null;
    }
  }

  // 在按钮下方弹出更新菜单。
  async function popupMenu(info: UpdateInfo | null, errMsg: string | null): Promise<void> {
    const btn = btnRef.current;
    if (!btn) return;
    const rect = btn.getBoundingClientRect();
    const at = new LogicalPosition(rect.left, rect.bottom + 4);

    const items: Array<MenuItem | PredefinedMenuItem> = [];

    // 「检查更新」：应用内 toast 反馈（正在检查 → 结果），不再二次弹菜单。
    items.push(
      await MenuItem.new({
        text: "检查更新",
        action: async () => {
          showToast("正在检查更新…", "info");
          const r = await doCheck();
          if (r) {
            showToast(
              r.has_update
                ? `发现新版本 v${r.installed} → v${r.latest}`
                : `已是最新版本 v${r.installed}`,
              "ok"
            );
          }
        },
      })
    );

    if (errMsg || !info) {
      items.push(
        await PredefinedMenuItem.new({ item: "Separator" }),
        await MenuItem.new({
          text: `检查更新失败：${errMsg ?? "无返回"}`,
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

  async function handleClick(e: ReactMouseEvent) {
    e.stopPropagation();
    // 点开菜单：先查询（可稍慢），弹出后展示当前/最新状态。
    let info: UpdateInfo | null = null;
    let errMsg: string | null = null;
    try {
      info = await invoke<UpdateInfo>("check_for_update");
    } catch (err) {
      errMsg = String(err ?? "未知错误");
    }
    await popupMenu(info, errMsg);
  }

  return (
    <>
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
      {toast && (
        <div className={`tb-toast tb-toast--${toast.kind}`} role="status">
          {toast.text}
        </div>
      )}
    </>
  );
}
