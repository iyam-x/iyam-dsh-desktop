import { invoke } from "@tauri-apps/api/core";
import { listen, type Event } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";
import { TitleBar } from "./components/TitleBar";
import "./App.css";

type AppStatus = "installing" | "loading" | "ready" | "crashed" | "error";

interface InstallState {
  status: AppStatus;
  message: string;
  port?: number;
  error?: string;
}

export default function App() {
  const [state, setState] = useState<InstallState>({
    status: "loading",
    message: "正在初始化...",
  });

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

  if (state.status === "error") {
    return (
      <div className="app-shell">
        <TitleBar />
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
