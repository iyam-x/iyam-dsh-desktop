// iyam-dsh 文件查看插件。
// 运行在 DSH web UI iframe(http://127.0.0.1:<port>/) 内，使用 window.__ModuleLoader__
// 老式客户端插件 API(与 @iyam/dsh-desktop-shell 同源)。
//
// 作用: 包装 workspaces.openPath(所有"打开文件"入口的唯一汇聚点——产出文件 chip、
// 工具行文件链接、markdown 文件提及、Show in folder)：
//   - 图片/音频/视频/文本/代码 → 转发给桌面壳做应用内联预览，不再调系统默认程序；
//   - 其余类型(压缩包/可执行文件等) → 保持原行为(系统默认程序打开)。
// 桌面壳侧(src/App.tsx)监听 source="iyam-dsh-file" 的消息，调 Tauri 读文件命令展示预览。

window.__ModuleLoader__.load({
  id: "@iyam/dsh-file-handler",
  factory: (require) => {
    const module = { exports: {} };
    const exports = module.exports;

    // 可内联预览的文件类型(按小写扩展名匹配; 无扩展名则匹配完整文件名,如 Dockerfile)。
    const PREVIEWABLE = new Set([
      // 图片
      "png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif", "ico",
      // 音频
      "mp3", "wav", "ogg", "oga", "m4a", "flac", "aac", "opus", "weba",
      // 视频
      "mp4", "webm", "mov", "m4v",
      // 文本 / 代码
      "txt", "md", "markdown", "json", "jsonc", "js", "mjs", "cjs", "ts", "tsx", "jsx",
      "py", "rs", "c", "h", "cpp", "hpp", "cc", "java", "kt", "go", "rb", "php", "swift",
      "sh", "bash", "zsh", "ps1", "bat", "cmd", "yml", "yaml", "toml", "ini", "cfg", "conf",
      "log", "csv", "tsv", "xml", "html", "htm", "css", "scss", "sass", "less", "sql",
      "vue", "svelte", "proto", "graphql", "gql",
      "dockerfile", "makefile", "gitignore", "editorconfig", "env",
    ]);

    function isPreviewable(path) {
      const base = (path.split(/[\\/]/).pop() || "").toLowerCase();
      if (!base) return false;
      if (PREVIEWABLE.has(base)) return true;
      const i = base.lastIndexOf(".");
      if (i < 0) return false;
      return PREVIEWABLE.has(base.slice(i + 1));
    }

    function apply(ctx) {
      const workspaces = ctx.workspaces;
      // 诊断：确认插件是否成功拿到 workspaces 并完成包装（决定"是不是我们插件干的"）。
      console.log("[fh] apply ctx.workspaces=", typeof workspaces, "| openPath=", typeof workspaces?.openPath);
      if (!workspaces || typeof workspaces.openPath !== "function") {
        console.warn("[fh] workspaces/openPath 不可用，跳过包装 → 文件打开不会被拦截");
        return;
      }
      const orig = workspaces.openPath.bind(workspaces);
      const isNoExtKnown = (base) => PREVIEWABLE.has(base);
      // "是否像真文件"：无扩展名 且 不是已知无扩展名文件(dockerfile/makefile/...) → 非文件路径。
      // 这类路径(如 DSH 内部标识符/临时名 "use_default")走系统 open 会触发"选择应用"对话框。
      const isNonFilePath = (path) => {
        const s = path ? String(path) : "";
        // 含路径分隔符 → 真实文件/目录路径，必须放行给系统/DSH 打开。
        // 否则"选择工作目录/工作区"传入的目录绝对路径（无扩展名）会被误判为负样本，
        // 被吞掉后 DSH 收不到真实选择、反复弹"选择工作区"。
        if (/[\\/]/.test(s)) return false;
        const base = s.toLowerCase();
        return !!base && !base.includes(".") && !isNoExtKnown(base);
      };
      const postToShell = (path, name) => {
        try {
          window.parent?.postMessage(
            { source: "iyam-dsh-file", type: "file-open", path, name },
            "*"
          );
        } catch (_e) { /* 壳未就绪时静默 */ }
      };

      workspaces.openPath = (path) => {
        const base = (path.split(/[\\/]/).pop() || "").toLowerCase();
        console.log("[fh] openPath:", JSON.stringify(path), "| base:", JSON.stringify(base), "| previewable:", isPreviewable(path));
        if (isPreviewable(path)) {
          postToShell(path, base);
          return Promise.resolve();
        }
        // 非可预览分支：只有"像真文件"才走系统打开，否则交给壳处理。
        if (isNonFilePath(path)) {
          console.log("[fh] 无扩展名非已知文件 → 跳过系统 open，交给壳");
          postToShell(path, base);
          return Promise.resolve();
        }
        console.log("[fh] 有扩展名/已知无扩展名 → 系统打开");
        return orig(path).catch(() => {});
      };

      // 兜底：直接包装 api.host.openPath / openTextFile，捕获绕过 workspaces.openPath 的调用
      // （实测 DSH 回答中的代码块"打开"动作会直连 api.host.openPath，绕开上面的包装）。
      // 非文件路径在此层截断，返回与 host 一致的成功响应，避免"选择应用"对话框。
      const host = workspaces.api?.host;
      console.log("[fh] api.host 可用:", typeof host, "| openPath:", typeof host?.openPath, "| openTextFile:", typeof host?.openTextFile);
      if (host) {
        for (const method of ["openPath", "openTextFile"]) {
          const origHost = host[method];
          if (typeof origHost !== "function") continue;
          host[method] = (payload, signal) => {
            const path = typeof payload === "string" ? payload : payload?.path;
            if (isNonFilePath(path)) {
              console.log(`[fh] api.host.${method} 非文件路径 → 截断:`, JSON.stringify(path));
              return Promise.resolve({ result: { ok: true, value: { opened: true } } });
            }
            console.log(`[fh] api.host.${method} 放行(真文件):`, JSON.stringify(path));
            return origHost(payload, signal);
          };
        }
      }
    }

    // ── 浏览器侧兜底：webview 可能把非 http(s) 的 window.open / 自定义 scheme 链接
    //    / location 导航交给系统打开（触发"选择应用"对话框）。统一拦截并记录。 ──
    const ALLOWED_URL = /^(https?:|data:|blob:|about:|javascript:|#)/i;
    const origWindowOpen = window.open.bind(window);
    window.open = function (url, ...rest) {
      const u = String(url ?? "");
      // 仅拦截带 scheme 且非白名单的地址（自定义 scheme）；相对地址放行，避免误伤 SPA 路由
      if (u && !ALLOWED_URL.test(u) && /^[a-z][a-z0-9+.-]*:/i.test(u)) {
        console.log("[fh] window.open 非 http(s) scheme 地址 → 拦截:", u);
        return null;
      }
      return origWindowOpen(url, ...rest);
    };
    document.addEventListener(
      "click",
      (e) => {
        const el = e.target && e.target.closest ? e.target.closest("a[href]") : null;
        if (!el) return;
        const href = el.getAttribute("href") || "";
        if (!href || ALLOWED_URL.test(href)) return;
        console.log("[fh] 点击自定义 scheme 链接 → 拦截:", href);
        e.preventDefault();
        e.stopPropagation();
      },
      true
    );
    // 拦截 location 导航（location.href= / assign / replace）到自定义 scheme。
    // 仅拦截"带 scheme 且非白名单"的地址（如 dsh://、app://、file://），避免 WebView2
    // 把自定义 scheme 交给系统弹"选择应用"对话框。无 scheme 的相对地址（/path、./path）
    // 是 SPA 内部路由，必须放行，否则"添加模型后返回列表"等相对跳转被误拦 → 界面不刷新。
    const blockScheme = (url) => {
      const u = String(url || "");
      if (!u) return false;
      if (ALLOWED_URL.test(u)) return false;
      if (!/^[a-z][a-z0-9+.-]*:/i.test(u)) return false; // 无 scheme → 内部路由，放行
      console.log("[fh] 导航到非 http(s) scheme → 拦截:", u);
      return true;
    };
    const loc = window.Location?.prototype;
    if (loc) {
      const hrefDesc = Object.getOwnPropertyDescriptor(loc, "href");
      if (hrefDesc && hrefDesc.set) {
        Object.defineProperty(loc, "href", {
          configurable: true,
          enumerable: true,
          get: hrefDesc.get,
          set(v) { if (!blockScheme(v)) hrefDesc.set.call(this, v); },
        });
      }
      for (const m of ["assign", "replace"]) {
        if (typeof loc[m] === "function") {
          const orig = loc[m];
          loc[m] = function (url) {
            if (blockScheme(url)) return undefined;
            return orig.apply(this, arguments);
          };
        }
      }
    }

    exports.apply = apply;
    exports.inject = ["workspaces"];
    return module.exports;
  },
});
