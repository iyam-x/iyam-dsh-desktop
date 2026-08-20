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
      if (!workspaces || typeof workspaces.openPath !== "function") return;
      const orig = workspaces.openPath.bind(workspaces);
      workspaces.openPath = (path) => {
        if (isPreviewable(path)) {
          try {
            window.parent?.postMessage(
              { source: "iyam-dsh-file", type: "file-open", path },
              "*"
            );
          } catch (_e) {
            // 转发失败时退回系统默认打开，保证可用
            return orig(path).catch(() => {});
          }
          return Promise.resolve();
        }
        return orig(path).catch(() => {});
      };
    }

    exports.apply = apply;
    exports.inject = ["workspaces"];
    return module.exports;
  },
});
