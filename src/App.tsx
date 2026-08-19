import { invoke } from "@tauri-apps/api/core";
import { listen, type Event } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { useEffect, useState } from "react";
import { TitleBar } from "./components/TitleBar";
import "./App.css";

type AppStatus = "installing" | "loading" | "ready" | "error";

interface InstallState {
  status: AppStatus;
  message: string;
  port?: number;
  error?: string;
}

// DSH shell 插件 postMessage 通知桥：turn/end reason → 通知文案
const TURN_END_TEXT: Record<string, string> = {
  completed: "DeepSeek Harness 已完成回复",
  "max-tokens": "回复达到 token 上限",
  error: "回复出现错误",
};

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

  // DSH iframe（跨域）内的 shell 插件通过 postMessage 上报 turn/end，
  // 窗口未聚焦时弹系统通知；通知相关异常一律静默。
  useEffect(() => {
    let disposed = false;

    async function handleTurnEnd(reason: string) {
      const text = TURN_END_TEXT[reason];
      if (!text) return;
      try {
        const win = getCurrentWindow();
        if (await win.isFocused()) return;
        let granted = await isPermissionGranted();
        if (!granted) {
          granted = (await requestPermission()) === "granted";
        }
        if (!granted || disposed) return;
        sendNotification({ title: "DeepSeek Harness", body: text });
      } catch (_) {
        // 静默
      }
    }

    const onMessage = (e: MessageEvent) => {
      const data = e.data as { source?: string; type?: string; reason?: string } | null;
      if (!data || data.source !== "iyam-dsh-shell" || data.type !== "turn-end") return;
      if (typeof data.reason === "string") {
        handleTurnEnd(data.reason).catch(() => {});
      }
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
