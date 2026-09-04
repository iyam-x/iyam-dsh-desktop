import { useEffect, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
  /** registry 确有更新，但版本超过内置插件兼容上限，自动更新被拦住 */
  update_held: boolean;
  /** 自动更新兼容上限版本 */
  compat_max: string;
};

type ToastAction = { label: string; onClick: () => void };
type Toast = { text: string; kind: "info" | "ok" | "err"; action?: ToastAction } | null;

// 备货阶段文案（后端 dsh-install-progress 事件的 stage）。
const STAGE_TEXT: Record<string, string> = {
  "downloading-node": "正在下载 Node 运行环境...",
  "staging-download": "正在准备更新...",
  "installing-dsh": "正在安装新版本...",
  "resolving-deps": "正在解析依赖...",
  "downloading-deps": "正在下载依赖...",
  "repairing-dsh": "正在校验并修复...",
  "staging-deploy": "正在部署新版本...",
  "staging-ready": "备货完成...",
  finalizing: "正在收尾...",
};

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

  // sticky：不自动消失（用于需要用户操作的提示，如备货完成后的「立即重启」）。
  function showToast(
    text: string,
    kind: "info" | "ok" | "err" = "info",
    action?: ToastAction,
    sticky = false
  ) {
    setToast({ text, kind, action });
    if (toastTimer.current) window.clearTimeout(toastTimer.current);
    toastTimer.current = sticky
      ? null
      : window.setTimeout(() => setToast(null), 2600);
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
            if (r.has_update && r.update_held) {
              showToast(
                `发现新版本 v${r.latest}，但自动更新已暂停（兼容上限 v${r.compat_max}）`,
                "info"
              );
            } else {
              showToast(
                r.has_update
                  ? `发现新版本 v${r.installed} → v${r.latest}`
                  : `已是最新版本 v${r.installed}`,
                "ok"
              );
            }
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
        if (info.update_held) {
          // registry 确有更新，但版本超兼容上限：只提示、不提供「下载并更新」，
          // 避免把破坏性 dsh 版本装进来弄崩内置插件。
          items.push(
            await MenuItem.new({
              text: `新版本 v${info.latest} 已发布（自动更新已暂停）`,
              enabled: false,
            }),
            await MenuItem.new({
              text: `兼容上限 v${info.compat_max}，等待应用适配`,
              enabled: false,
            })
          );
        } else if (info.managed) {
          items.push(
            await MenuItem.new({
              text: "下载并更新（下次启动生效）",
              action: async () => {
                // 备货要走一遍 npm 安装（约 1 分钟），实时透出阶段与百分比，
                // 避免点了之后长时间没有任何反馈。
                const unlisten = await listen<{
                  stage: string;
                  progress: number;
                }>("dsh-install-progress", (event) => {
                  const { stage, progress } = event.payload;
                  const label = STAGE_TEXT[stage] ?? "正在更新...";
                  const pct =
                    typeof progress === "number"
                      ? ` ${Math.round(progress * 100)}%`
                      : "";
                  showToast(`${label}${pct}`, "info", undefined, true);
                }).catch(() => null);
                showToast("正在下载更新...", "info", undefined, true);
                try {
                  await invoke("trigger_dsh_update");
                  await invoke("notify", {
                    title: "dsh 更新已备货",
                    body: "重启应用后生效",
                  });
                  showToast(
                    "更新已备货，重启后生效",
                    "ok",
                    { label: "立即重启", onClick: () => void invoke("restart_app") },
                    true
                  );
                } catch (err) {
                  await invoke("notify", {
                    title: "dsh 更新失败",
                    body: String(err ?? "未知错误"),
                  });
                  showToast(`更新失败：${String(err ?? "未知错误")}`, "err");
                } finally {
                  unlisten?.();
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
          <span>{toast.text}</span>
          {toast.action && (
            <button
              className="tb-toast-action"
              onClick={toast.action.onClick}
              onMouseDown={stopDragPropagation}
            >
              {toast.action.label}
            </button>
          )}
        </div>
      )}
    </>
  );
}
