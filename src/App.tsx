import { invoke } from "@tauri-apps/api/core";
import { listen, type Event } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import "./App.css";

type AppStatus = "installing" | "loading" | "ready" | "error";

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

    // Listen for port-ready events (from existing process)
    listen<number>("dsh-port-ready", (event: Event<number>) => {
      setState((prev) => {
        if (prev.status !== "ready") {
          return { status: "ready", message: "", port: event.payload };
        }
        return prev;
      });
    }).catch(() => {});

    return () => {
      cancelled = true;
    };
  }, []);

  if (state.status === "error") {
    return (
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
    );
  }

  if (state.status === "installing") {
    return (
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
    );
  }

  if (state.status === "loading") {
    return (
      <div className="app loading">
        <div className="install-card">
          <div className="spinner" />
          <h2>正在启动 DeepSeek Harness</h2>
          <p>{state.message}</p>
        </div>
      </div>
    );
  }

  // Ready — embed DSH web UI
  return (
    <div className="app ready">
      <iframe
        src={`http://127.0.0.1:${state.port}`}
        title="DeepSeek Harness"
        className="webview"
      />
    </div>
  );
}
