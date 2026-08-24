import { invoke } from "@tauri-apps/api/core";
import { listen, type Event } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useRef, useState } from "react";
import { TitleBar } from "./components/TitleBar";
import { PreviewDock, DEFAULT_THEME, type Preview, type ThemeState } from "./components/PreviewDock";
import "./App.css";

type AppStatus = "installing" | "loading" | "ready" | "crashed" | "error";

interface InstallState {
  status: AppStatus;
  message: string;
  port?: number;
  error?: string;
  progress?: number;
}

const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif", "ico"]);
const AUDIO_EXTS = new Set(["mp3", "wav", "ogg", "oga", "m4a", "flac", "aac", "opus", "weba"]);
const VIDEO_EXTS = new Set(["mp4", "webm", "mov", "m4v"]);

const MIME: Record<string, string> = {
  png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif",
  webp: "image/webp", svg: "image/svg+xml", bmp: "image/bmp", avif: "image/avif", ico: "image/x-icon",
  mp3: "audio/mpeg", wav: "audio/wav", ogg: "audio/ogg", oga: "audio/ogg", m4a: "audio/mp4",
  flac: "audio/flac", aac: "audio/aac", opus: "audio/opus", weba: "audio/webm",
  mp4: "video/mp4", webm: "video/webm", mov: "video/quicktime", m4v: "video/mp4",
};

const DOCK_MIN = 320;
const DOCK_MAX = 760;
const DOCK_DEFAULT = 460;
const DOCK_STORAGE_KEY = "iyam-dsh-dock-width";

// 手动启动 DSH 的终端命令：Windows 下安装的包装脚本是 dsh.cmd，其余平台是 dsh。
const DSH_CLI = /Windows/i.test(navigator.userAgent) ? "dsh.cmd" : "dsh";

function loadDockWidth(): number {
  try {
    const raw = localStorage.getItem(DOCK_STORAGE_KEY);
    if (raw) {
      const n = parseInt(raw, 10);
      if (!Number.isNaN(n)) return Math.min(DOCK_MAX, Math.max(DOCK_MIN, n));
    }
  } catch {
    /* localStorage 不可用时退回默认 */
  }
  return DOCK_DEFAULT;
}

export default function App() {
  const [state, setState] = useState<InstallState>({
    status: "loading",
    message: "正在初始化...",
  });
  const [preview, setPreview] = useState<Preview | null>(null);
  const [theme, setTheme] = useState<ThemeState>(DEFAULT_THEME);
  const [dockWidth, setDockWidth] = useState<number>(loadDockWidth);
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const closePreview = () => setPreview(null);

  // 打开 DSH 转发来的文件预览：按扩展名分图片/音视频(读二进制)与文本/代码(读全文)。
  async function openPreview(path: string) {
    // 向 DSH iframe 请求一次当前主题（dsh-rtui-ui 收到后回发），
    // 保证 dock 一打开就跟随主题，不受首次消息时序影响。
    iframeRef.current?.contentWindow?.postMessage(
      { source: "iyam-dsh", type: "request-theme" },
      "*"
    );
    const name = path.split(/[\\/]/).pop() || path;
    const ext = name.includes(".") ? name.slice(name.lastIndexOf(".") + 1).toLowerCase() : "";
    try {
      if (IMAGE_EXTS.has(ext) || AUDIO_EXTS.has(ext) || VIDEO_EXTS.has(ext)) {
        const data = await invoke<{ base64: string }>("read_file_data", { path });
        const kind = IMAGE_EXTS.has(ext) ? "image" : AUDIO_EXTS.has(ext) ? "audio" : "video";
        const mime = MIME[ext] || "application/octet-stream";
        setPreview({ kind, name, path, dataUrl: `data:${mime};base64,${data.base64}` });
      } else {
        const data = await invoke<{ content: string }>("read_text_file", { path });
        setPreview({ kind: "text", name, path, content: data.content });
      }
    } catch (err) {
      setPreview({ kind: "error", name, message: String(err) });
    }
  }

  useEffect(() => {
    let cancelled = false;

    async function init() {
      // Check installation status first
      try {
        const checkResult = await invoke<any>("get_install_status");
        if (cancelled) return;

        if (checkResult.status === "Installed") {
          setState({ status: "loading", message: "正在启动 DeepSeek Harness..." });
          return;
        }

        // Not installed — trigger install
        setState({
          status: "installing",
          message: "正在安装 DeepSeek Harness...",
          progress: 0,
        });
        // 前端兜底超时：后端 npm install 有 15 分钟整体超时，此处 16 分钟作为
        // 最后防线，确保任何意外都不会让界面永久卡在转圈。
        const installPromise = invoke<any>("check_and_install");
        const timeoutPromise = new Promise<never>((_, reject) =>
          setTimeout(
            () => reject(new Error("安装超时，请检查网络后重试")),
            16 * 60 * 1000
          )
        );
        await Promise.race([installPromise, timeoutPromise]);
        if (cancelled) return;
        setState({ status: "loading", message: "正在启动 DeepSeek Harness..." });
      } catch (err) {
        if (cancelled) return;
        setState({
          status: "error",
          message: String(err),
          error: String(err),
        });
        return;
      }

      // Start DSH and wait for port
      try {
        const port = await invoke<number>("start_dsh");
        if (cancelled) return;
        setState({ status: "ready", message: "", port });
      } catch (err) {
        if (cancelled) return;
        setState({
          status: "error",
          message: "DSH 启动失败",
          error: String(err),
        });
      }
    }

    init();

    // 安装进度（后端 dsh-install-progress 事件）：首次安装需联网下载运行环境，
    // 把阶段与进度透出，避免"永久转圈=卡死"的体感。
    listen<{ stage: string; progress: number }>("dsh-install-progress", (event: Event<{ stage: string; progress: number }>) => {
      const { stage, progress } = event.payload;
      const stageText: Record<string, string> = {
        "downloading-node": "正在下载 Node 运行环境...",
        "installing-dsh": "正在准备安装 DeepSeek Harness...",
        "resolving-deps": "正在解析依赖...",
        "downloading-deps": "正在下载依赖（首次较慢，请耐心等待）...",
        "finalizing": "正在收尾部署...",
        "done": "安装完成",
      };
      setState((prev) => {
        if (prev.status !== "installing") return prev;
        return {
          ...prev,
          message: stageText[stage] || "正在安装 DeepSeek Harness...",
          progress: typeof progress === "number" ? progress : prev.progress,
        };
      });
    }).catch(() => {});

    // Listen for port-ready events (from existing process or fresh start)
    listen<number>("dsh-port-ready", (event: Event<number>) => {
      setState((prev) => {
        if (prev.status !== "ready" && prev.status !== "crashed") {
          return { status: "ready", message: "", port: event.payload };
        }
        return prev;
      });
    }).catch(() => {});

    // DSH 进程意外退出时通知前端切换到崩溃状态
    listen<void>("dsh-process-exit", () => {
      setState((prev) => {
        if (prev.status === "ready") {
          return { ...prev, status: "crashed" };
        }
        return prev;
      });
    }).catch(() => {});

    return () => {
      cancelled = true;
    };
  }, []);

  // DSH iframe（跨域）内的 shell 插件通过 postMessage 上报会话事件
  // （对话完成 / 等待授权 / 等待回复），转发给 Rust notify 命令；
  // obscured 判断与弹系统通知都在 Rust 侧完成，通知相关异常一律静默。
  useEffect(() => {
    let disposed = false;

    const onMessage = (e: MessageEvent) => {
      const data = e.data as
        | { source?: string; type?: string; title?: string; reason?: string }
        | null;
      if (!data || data.source !== "iyam-dsh-shell" || data.type !== "turn-end") return;
      const title = typeof data.title === "string" ? data.title : "DeepSeek Harness";
      const body = typeof data.reason === "string" ? data.reason : "";
      if (!body || disposed) return;
      invoke("notify", { title, body }).catch(() => {});
    };

    window.addEventListener("message", onMessage);
    return () => {
      disposed = true;
      window.removeEventListener("message", onMessage);
    };
  }, []);

  // DSH 文件内联预览桥：dsh-file-handler 插件把文件点击转发到这里，按类型读取并展示
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      const data = e.data as { source?: string; type?: string; path?: string } | null;
      if (!data || data.source !== "iyam-dsh-file" || data.type !== "file-open") return;
      if (typeof data.path !== "string" || !data.path) return;
      void openPreview(data.path);
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  // DSH 主题同步：dsh-rtui-ui 插件把当前生效实色 postMessage 过来，
  // 预览面板/编辑器据此着色，与 DSH 视觉统一（消除割裂感）。
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      const data = e.data as
        | { source?: string; type?: string; dark?: boolean; accent?: string; colors?: ThemeState["colors"] }
        | null;
      if (!data || data.source !== "iyam-dsh-theme" || data.type !== "theme") return;
      if (!data.colors || typeof data.accent !== "string") return;
      setTheme({
        dark: data.dark === true,
        accent: data.accent,
        colors: data.colors,
      });
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  // Esc 关闭预览
  useEffect(() => {
    if (!preview) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setPreview(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [preview]);

  // 开发者工具：F12 / Cmd(Ctrl)+Shift+I（壳聚焦时直接响应；DSH iframe 内聚焦时由
  // dsh-rtui-ui 插件 postMessage 转发）。release 构建需 tauri `devtools` 特性。
  useEffect(() => {
    const openDevtools = () => void invoke("open_devtools");
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (e.key === "F12" || (mod && e.shiftKey && (e.key === "I" || e.key === "i"))) {
        e.preventDefault();
        openDevtools();
      }
    };
    const onMessage = (e: MessageEvent) => {
      const data = e.data as { source?: string; type?: string } | null;
      if (data?.source === "iyam-dsh" && data.type === "open-devtools") openDevtools();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("message", onMessage);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("message", onMessage);
    };
  }, []);

  // 预览面板宽度变化：记忆到 localStorage，下次打开保持。
  const handleResize = (w: number) => {
    setDockWidth(w);
    try {
      localStorage.setItem(DOCK_STORAGE_KEY, String(w));
    } catch {
      /* 忽略持久化失败 */
    }
  };

  if (state.status === "error") {
    return (
      <div className="app-shell">
        <TitleBar rightOffset={preview ? dockWidth : 0} />
        <div className="app error">
          <div className="error-card">
            <div className="error-icon">⚠</div>
            <h2>启动失败</h2>
            <p className="error-msg">{state.error || state.message}</p>
            <button onClick={() => window.location.reload()}>重试</button>
            <p className="error-hint">
              也可手动在终端运行：
              <code>{`~/.dsh/bin/${DSH_CLI} web`}</code>
            </p>
          </div>
        </div>
      </div>
    );
  }

  // DSH 进程意外退出
  if (state.status === "crashed") {
    return (
      <div className="app-shell">
        <TitleBar rightOffset={preview ? dockWidth : 0} />
        <div className="app crashed">
          <div className="error-card">
            <div className="error-icon">⚡</div>
            <h2>DeepSeek Harness 已退出</h2>
            <p className="error-msg">
              后台进程意外终止，可能是内存不足或内部错误。
            </p>
            <button onClick={() => invoke("restart_dsh")}>重启 DSH</button>
            <p className="error-hint">
              也可手动在终端运行：
              <code>{`~/.dsh/bin/${DSH_CLI} web`}</code>
            </p>
          </div>
        </div>
      </div>
    );
  }

  if (state.status === "installing") {
    const pct = Math.round((state.progress ?? 0) * 100);
    return (
      <div className="app-shell">
        <TitleBar rightOffset={preview ? dockWidth : 0} />
        <div className="app installing">
          <div className="install-card">
            <div className="spinner" />
            <h2>正在安装 DeepSeek Harness</h2>
            <p>{state.message}</p>
            <div className="install-progress">
              <div
                className="install-progress-bar"
                style={{ width: `${pct}%` }}
              />
            </div>
            <div className="install-progress-pct">{pct}%</div>
            <div className="install-tip">
              首次安装需联网下载运行环境（国内镜像），请保持网络畅通...
            </div>
          </div>
        </div>
      </div>
    );
  }

  if (state.status === "loading") {
    return (
      <div className="app-shell">
        <TitleBar rightOffset={preview ? dockWidth : 0} />
        <div className="app loading">
          <div className="install-card">
            <div className="spinner" />
            <h2>正在启动 DeepSeek Harness</h2>
            <p>{state.message}</p>
          </div>
        </div>
      </div>
    );
  }

  // Ready — embed DSH web UI，预览作为右侧停靠面板与其同屏。
  return (
    <div className="app-shell">
      <TitleBar rightOffset={preview ? dockWidth : 0} />
      <div className="app ready">
        <iframe
          src={`http://127.0.0.1:${state.port}`}
          title="DeepSeek Harness"
          className="webview"
          ref={iframeRef}
        />
        {preview && (
          <PreviewDock
            preview={preview}
            theme={theme}
            width={dockWidth}
            minWidth={DOCK_MIN}
            maxWidth={DOCK_MAX}
            onClose={closePreview}
            onResize={handleResize}
          />
        )}
      </div>
    </div>
  );
}
