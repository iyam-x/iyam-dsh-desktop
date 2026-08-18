import { useEffect, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { Menu, MenuItem } from "@tauri-apps/api/menu";

// Windows 系统标题按钮字形（Segoe Fluent/MDL2 Assets 系统字体，Win10 1809+ 自带）
const ICON = {
  minimize: "\uE921",
  maximize: "\uE922",
  restore: "\uE923",
  close: "\uE106",
} as const;

// 控制按钮需阻止冒泡，否则按下按钮也会触发标题栏的窗口拖动
function stopDragPropagation(e: ReactMouseEvent) {
  e.stopPropagation();
}

const isMac = (() => {
  try {
    return navigator.userAgent.toLowerCase().includes("mac");
  } catch {
    return false;
  }
})();

const isWindows = (() => {
  try {
    return navigator.userAgent.toLowerCase().includes("win");
  } catch {
    return false;
  }
})();

// macOS 无右侧系统按钮，右键用原生 NSMenu 模拟系统标题栏菜单（关闭/最小化/缩放）
async function showMacTitleBarMenu(isMax: boolean): Promise<void> {
  const win = getCurrentWindow();
  const close = await MenuItem.new({ text: "关闭", action: () => win.close() });
  const minimize = await MenuItem.new({ text: "最小化", action: () => win.minimize() });
  const zoom = await MenuItem.new({
    text: isMax ? "还原" : "缩放",
    action: () => win.toggleMaximize(),
  });
  const menu = await Menu.new({ items: [close, minimize, zoom] });
  // 不传位置 → 在右键光标处弹出
  await menu.popup();
}

export function TitleBar() {
  const [maximized, setMaximized] = useState(false);
  const lastClickTime = useRef(0);

  useEffect(() => {
    getCurrentWindow()
      .isMaximized()
      .then(setMaximized)
      .catch(() => {});
    const cleanup = getCurrentWindow()
      .onResized(async () => {
        setMaximized(await getCurrentWindow().isMaximized().catch(() => false));
      })
      .catch(() => () => {});
    return () => {
      cleanup.then((fn) => fn());
    };
  }, []);

  // 单击开始拖拽，300ms 内第二次单击 → 缩放（macOS 双击缩放原生手感）
  function handleTitleBarMouseDown(e: ReactMouseEvent) {
    if (e.button !== 0) return;
    const now = Date.now();
    if (now - lastClickTime.current < 300) {
      lastClickTime.current = 0;
      e.preventDefault();
      getCurrentWindow().toggleMaximize().catch(() => {});
      return;
    }
    lastClickTime.current = now;
    e.preventDefault();
    getCurrentWindow().startDragging().catch(() => {});
  }

  // macOS：纯透明拖拽层，红绿灯由系统原生绘制（titleBarStyle Overlay）
  if (isMac) {
    return (
      <div
        className="title-bar title-bar--mac"
        onMouseDown={handleTitleBarMouseDown}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          showMacTitleBarMenu(maximized).catch(() => {});
        }}
      />
    );
  }

  // Windows / Linux：透明悬浮层 + 右上角自绘三键
  return (
    <div
      className="title-bar"
      onMouseDown={handleTitleBarMouseDown}
      onContextMenu={(e) => {
        e.preventDefault();
        if (isWindows) {
          // 弹出原生 Windows 系统菜单（还原/移动/大小/最小化/最大化/关闭）
          invoke("show_system_menu").catch(() => {});
        }
      }}
    >
      <div className="title-bar-controls" onMouseDown={stopDragPropagation}>
        <button
          className="tb-btn tb-minimize"
          onClick={() => getCurrentWindow().minimize()}
          aria-label="最小化"
        >
          <span className="tb-icon">{ICON.minimize}</span>
        </button>
        <button
          className="tb-btn tb-maximize"
          onClick={() => getCurrentWindow().toggleMaximize()}
          aria-label="最大化"
        >
          <span className="tb-icon">{maximized ? ICON.restore : ICON.maximize}</span>
        </button>
        <button
          className="tb-btn tb-close"
          onClick={() => getCurrentWindow().close()}
          aria-label="关闭"
        >
          <span className="tb-icon">{ICON.close}</span>
        </button>
      </div>
    </div>
  );
}
