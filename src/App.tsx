import { invoke } from "@tauri-apps/api/core";
import { listen, type Event } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useMemo, useState } from "react";
import { TitleBar } from "./components/TitleBar";
import hljs from "highlight.js/lib/common";
import "highlight.js/styles/github-dark.css";
import "./App.css";

type AppStatus = "installing" | "loading" | "ready" | "crashed" | "error";

interface InstallState {
  status: AppStatus;
  message: string;
  port?: number;
  error?: string;
}

/** 文件内联预览状态（来自 DSH 的 dsh-file-handler 插件转发）。 */
type Preview =
  | { kind: "image" | "audio" | "video"; name: string; dataUrl: string }
  | { kind: "text"; name: string; content: string }
  | { kind: "error"; name: string; message: string };

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

function escapeHtml(s: string): string {
  return s.replace(/[&<>]/g, (c) => (c === "&" ? "&amp;" : c === "<" ? "&lt;" : "&gt;"));
}

function PreviewOverlay({ preview, onClose }: { preview: Preview; onClose: () => void }) {
  const isText = preview.kind === "text";

  // 代码高亮（内容过大时跳过高亮只转义，避免卡死；读取仍是全量）
  const highlighted = useMemo(() => {
    if (preview.kind !== "text") return "";
    if (preview.content.length > 1_000_000) return escapeHtml(preview.content);
    try {
      return hljs.highlightAuto(preview.content).value;
    } catch {
      return escapeHtml(preview.content);
    }
  }, [preview]);

  const gutter = useMemo(() => {
    if (preview.kind !== "text") return "";
    const n = preview.content.split("\n").length;
    return Array.from({ length: n }, (_, i) => String(i + 1)).join("\n");
  }, [preview]);

  return (
    <div className="preview-backdrop" onClick={onClose}>
      <div className="preview-panel" onClick={(e) => e.stopPropagation()}>
        <div className="preview-head">
          <span className="preview-name" title={preview.name}>
            {preview.name}
          </span>
          <button type="button" className="preview-close" onClick={onClose} aria-label="关闭">
            ✕
          </button>
        </div>
        <div className="preview-body">
          {preview.kind === "image" && (
            <img className="preview-media" src={preview.dataUrl} alt={preview.name} />
          )}
          {preview.kind === "video" && (
            <video className="preview-media" src={preview.dataUrl} controls autoPlay />
          )}
          {preview.kind === "audio" && (
            <audio className="preview-audio" src={preview.dataUrl} controls autoPlay />
          )}
          {preview.kind === "text" && (
            <div className="preview-code">
              <pre className="preview-gutter">{gutter}</pre>
              <pre className="preview-code-block">
                <code className="hljs" dangerouslySetInnerHTML={{ __html: highlighted }} />
              </pre>
            </div>
          )}
          {preview.kind === "error" && <div className="preview-error">{preview.message}</div>}
        </div>
      </div>
    </div>
  );
}


export default function App() {
  const [state, setState] = useState<InstallState>({
    status: "loading",
    message: "正在初始化...",
  });
  const [preview, setPreview] = useState<Preview | null>(null);
  const closePreview = () => setPreview(null);

  // 打开 DSH 转发来的文件预览：按扩展名分图片/音视频(读二进制)与文本/代码(读全文)。
  async function openPreview(path: string) {
    const name = path.split(/[\\/]/).pop() || path;
    const ext = name.includes(".") ? name.slice(name.lastIndexOf(".") + 1).toLowerCase() : "";
    try {
      if (IMAGE_EXTS.has(ext) || AUDIO_EXTS.has(ext) || VIDEO_EXTS.has(ext)) {
        const data = await invoke<{ base64: string }>("read_file_data", { path });
        const kind = IMAGE_EXTS.has(ext) ? "image" : AUDIO_EXTS.has(ext) ? "audio" : "video";
        const mime = MIME[ext] || "application/octet-stream";
        setPreview({ kind, name, dataUrl: `data:${mime};base64,${data.base64}` });
      } else {
        const data = await invoke<{ content: string }>("read_text_file", { path });
        setPreview({ kind: "text", name, content: data.content });
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
        });
        const installResult = await invoke<any>("check_and_install");
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

  // Esc 关闭预览
  useEffect(() => {
    if (!preview) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setPreview(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [preview]);

  if (state.status === "error") {
    return (
      <div className="app-shell">
        <TitleBar />
        {preview && <PreviewOverlay preview={preview} onClose={closePreview} />}
        <div className="app error">
          <div className="error-card">
            <div className="error-icon">⚠</div>
            <h2>启动失败</h2>
            <p className="error-msg">{state.error || state.message}</p>
            <button onClick={() => window.location.reload()}>重试</button>
            <p className="error-hint">
              也可手动在终端运行：
              <code>~/.iyam-dsh/bin/dsh web</code>
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
        <TitleBar />
        {preview && <PreviewOverlay preview={preview} onClose={closePreview} />}
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
              <code>~/.iyam-dsh/bin/dsh web</code>
            </p>
          </div>
        </div>
      </div>
    );
  }

  if (state.status === "installing") {
    return (
      <div className="app-shell">
        <TitleBar />
        {preview && <PreviewOverlay preview={preview} onClose={closePreview} />}
        <div className="app installing">
          <div className="install-card">
            <div className="spinner" />
            <h2>正在安装 DeepSeek Harness</h2>
            <p>{state.message}</p>
            <div className="install-tip">
              正在从内置资源部署（约 300MB），无需网络，请耐心等待...
            </div>
          </div>
        </div>
      </div>
    );
  }

  if (state.status === "loading") {
    return (
      <div className="app-shell">
        <TitleBar />
        {preview && <PreviewOverlay preview={preview} onClose={closePreview} />}
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

  // Ready — embed DSH web UI
  return (
      <div className="app-shell">
        <TitleBar />
        {preview && <PreviewOverlay preview={preview} onClose={closePreview} />}
        <div className="app ready">
        <iframe
          src={`http://127.0.0.1:${state.port}`}
          title="DeepSeek Harness"
          className="webview"
        />
      </div>
    </div>
  );
}
