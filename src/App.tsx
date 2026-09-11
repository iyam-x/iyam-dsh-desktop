import { invoke } from "@tauri-apps/api/core";
import { listen, type Event } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { TitleBar } from "./components/TitleBar";
import "./App.css";

type AppStatus = "installing" | "loading" | "ready" | "crashed" | "error";

interface InstallState {
  status: AppStatus;
  message: string;
  url?: string;
  error?: string;
  progress?: number;
  exiting?: boolean;
  kind?: "install" | "launch";
}

// 手动启动 DSH 的终端命令：Windows 下安装的包装脚本是 dsh.cmd，其余平台是 dsh。
const DSH_CLI = /Windows/i.test(navigator.userAgent) ? "dsh.cmd" : "dsh";

export default function App() {
  const [state, setState] = useState<InstallState>({
    status: "loading",
    message: "正在初始化...",
  });
  // 标记应用正在退出（用户主动退出），用于抑制退出时 DSH 进程被杀触发的崩溃卡片一闪。
  const exitingRef = useRef(false);
  // 启动完成后是否展示「安装插件市场」询问弹窗（后端仅在首次启动且 dshmarket 未安装时发来事件）。
  const [marketOffer, setMarketOffer] = useState(false);
  const [marketInstalling, setMarketInstalling] = useState(false);
  const [marketError, setMarketError] = useState<string | null>(null);

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
          kind: "install",
        });
        return;
      }

      // Start DSH and wait for its authenticated web URL
      try {
        const url = await invoke<string>("start_dsh");
        if (cancelled) return;
        setState({ status: "ready", message: "", url });
      } catch (err) {
        if (cancelled) return;
        setState({
          status: "error",
          message: "DSH 启动失败",
          error: String(err),
          kind: "launch",
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

    // Listen for web-URL events (from existing process or fresh start).
    // dsh 0.1.2-rc.1 起带 launch-token 认证：必须加载后端传来的完整 URL
    // （含 token，首次访问换取签名 cookie），裸地址会得到 401。
    listen<string>("dsh-port-ready", (event: Event<string>) => {
      setState((prev) => {
        if (prev.status !== "ready" && prev.status !== "crashed") {
          return { status: "ready", message: "", url: event.payload };
        }
        return prev;
      });
    }).catch(() => {});

    // 应用主动退出：Rust 侧先杀 DSH 进程再关窗，标记 exiting 以抑制下方崩溃卡片闪现，
    // 并隐藏 iframe 避免看到 DSH 后端被杀时的「加载失败」错误页。
    listen<void>("dsh-app-exiting", () => {
      exitingRef.current = true;
      setState((prev) => ({ ...prev, exiting: true }));
    }).catch(() => {});

    // DSH 进程意外退出时通知前端切换到崩溃状态（主动退出时已由 exiting 标记跳过）。
    listen<void>("dsh-process-exit", () => {
      if (exitingRef.current) return;
      setState((prev) => {
        if (prev.status === "ready") {
          return { ...prev, status: "crashed" };
        }
        return prev;
      });
    }).catch(() => {});

    // 启动成功后，若 dshmarket 尚未安装，后端会发来该事件，前端弹窗让用户选择是否安装。
    listen<void>("dshmarket-offer-install", () => {
      if (exitingRef.current) return;
      setMarketOffer(true);
    }).catch(() => {});

    // 启动过程中自动禁用了与当前 dsh 不兼容的第三方插件：系统通知告知用户
    // （核心功能不受影响；dsh 版本再次变化时会自动恢复重试）。
    listen<string[]>("dsh-plugins-auto-disabled", (event: Event<string[]>) => {
      if (exitingRef.current) return;
      const names = (event.payload || []).filter(Boolean);
      if (!names.length) return;
      invoke("notify", {
        title: "已自动禁用不兼容插件",
        body: `${names.join("、")} 与当前 dsh 版本不兼容，已暂时禁用以保证启动；待插件适配新版本后会自动恢复`,
      }).catch(() => {});
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

  // 启动后自动检查本 app 自身的更新（后端 24h 节流；离线/查不到一律静默）。
  // 有新版才弹系统通知，且 force=true 跳过"窗口聚焦就不弹"的判断——新版本值得打断一次，
  // 否则启动时窗口正聚焦，Windows 上这条提示等于不会出现。
  useEffect(() => {
    invoke<{ current: string; latest: string; has_update: boolean }>("check_app_update", {
      force: false,
    })
      .then((info) => {
        if (!info.has_update) return;
        return invoke("notify", {
          force: true,
          title: "iyam-dsh 有新版本",
          body: `应用更新 v${info.current} → v${info.latest}，可在标题栏下拉菜单「检查应用更新」前往下载`,
        });
      })
      .catch(() => {});
  }, []);

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

  // 用户确认安装插件市场 dshmarket（后端幂等：已装跳过；失败回传错误）。
  async function installMarket() {
    setMarketInstalling(true);
    setMarketError(null);
    try {
      await invoke("install_dshmarket");
      setMarketOffer(false);
    } catch (err) {
      setMarketError(String(err));
    } finally {
      setMarketInstalling(false);
    }
  }

  // 用户拒绝安装：仅关闭弹窗，不影响任何功能。
  function declineMarket() {
    setMarketOffer(false);
  }

  if (state.status === "error") {
    const heading = state.kind === "install" ? "安装失败" : "启动失败";
    return (
      <div className="app-shell">
        <TitleBar />
        <div className="app error">
          <div className="error-card">
            <div className="error-icon">⚠</div>
            <h2>{heading}</h2>
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
        <TitleBar />
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

  // Ready — embed DSH web UI。
  return (
    <div className="app-shell">
      <TitleBar />
      <div className="app ready">
        <iframe
          src={state.url}
          title="DeepSeek Harness"
          className="webview"
          style={state.exiting ? { display: "none" } : undefined}
        />
      </div>
      {marketOffer && (
        <div className="modal-overlay" onClick={declineMarket}>
          <div className="modal-card" onClick={(e) => e.stopPropagation()}>
            <h3>安装插件市场？</h3>
            <p className="modal-desc">
              是否安装 DeepSeek Harness 插件市场（dshmarket）？安装后可在应用内浏览与安装更多插件。
              稍后也可在终端运行 <code>{`~/.dsh/bin/${DSH_CLI} plugin --profile web add dshmarket`}</code> 手动安装。
            </p>
            {marketError && <p className="modal-error">{marketError}</p>}
            <div className="modal-actions">
              <button className="btn-ghost" onClick={declineMarket} disabled={marketInstalling}>
                暂不安装
              </button>
              <button className="btn-primary" onClick={installMarket} disabled={marketInstalling}>
                {marketInstalling ? "安装中..." : "安装"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
