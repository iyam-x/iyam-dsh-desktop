import { useEffect, useRef, useState } from "react";
import type { CSSProperties, PointerEvent as ReactPointerEvent } from "react";
import { EditorView, keymap } from "@codemirror/view";
import { basicSetup } from "codemirror";
import { Compartment, type Extension } from "@codemirror/state";
import { indentWithTab } from "@codemirror/commands";
import { languages } from "@codemirror/language-data";
import { oneDark } from "@codemirror/theme-one-dark";
import { syntaxHighlighting, defaultHighlightStyle, LanguageDescription } from "@codemirror/language";
import { invoke } from "@tauri-apps/api/core";

export type Preview =
  | { kind: "image" | "audio" | "video"; name: string; path: string; dataUrl: string }
  | { kind: "text"; name: string; path: string; content: string }
  | { kind: "error"; name: string; message: string };

export type ThemeColors = {
  bg: string;
  layer: string;
  label: string;
  label2: string;
  border: string;
};
export type ThemeState = { dark: boolean; accent: string; colors: ThemeColors };

export const DEFAULT_THEME: ThemeState = {
  dark: true,
  accent: "#4D6BFE",
  colors: {
    bg: "#161618",
    layer: "#1E1E20",
    label: "#E9E9EC",
    label2: "#9B9BA1",
    border: "rgba(255,255,255,0.10)",
  },
};

function hexToRgba(hex: string, alpha: number): string {
  const m = (hex || "").replace("#", "");
  if (m.length !== 6) return `rgba(77,107,254,${alpha})`;
  const r = parseInt(m.slice(0, 2), 16);
  const g = parseInt(m.slice(2, 4), 16);
  const b = parseInt(m.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

// 用 DSH 同步来的主题(实色 + 强调色)构建编辑器外观，让预览与 DSH 视觉统一。
function buildEditorTheme(theme: ThemeState): Extension {
  const { dark, accent, colors } = theme;
  const base = EditorView.theme(
    {
      "&": { color: colors.label, backgroundColor: colors.bg, height: "100%" },
      ".cm-scroller": {
        fontFamily: '"Cascadia Code", "JetBrains Mono", Consolas, monospace',
        fontSize: "12.5px",
        lineHeight: "1.55",
      },
      ".cm-content": { caretColor: accent },
      ".cm-cursor, .cm-dropCursor": { borderLeftColor: accent },
      "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": {
        backgroundColor: hexToRgba(accent, 0.3),
      },
      ".cm-gutters": { backgroundColor: colors.layer, color: colors.label2, border: "none" },
      ".cm-activeLine": {
        backgroundColor: dark ? "rgba(255,255,255,0.04)" : "rgba(0,0,0,0.04)",
      },
      ".cm-activeLineGutter": {
        backgroundColor: dark ? "rgba(255,255,255,0.06)" : "rgba(0,0,0,0.06)",
      },
    },
    { dark }
  );
  // base 放在 oneDark 之后，同选择器下后者优先级更高，从而用 DSH 主题色覆盖。
  return dark
    ? [oneDark, base]
    : [base, syntaxHighlighting(defaultHighlightStyle, { fallback: true })];
}

type CodeEditorProps = {
  preview: Extract<Preview, { kind: "text" }>;
  theme: ThemeState;
  editorRef: React.MutableRefObject<EditorView | null>;
  onDirtyChange: (dirty: boolean) => void;
};

function CodeEditor({ preview, theme, editorRef, onDirtyChange }: CodeEditorProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const compsRef = useRef<{ lang: Compartment; theme: Compartment } | null>(null);

  // 按文件创建编辑器实例（path 变化即换文件，重建）。
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const lang = new Compartment();
    const themeComp = new Compartment();
    compsRef.current = { lang, theme: themeComp };

    const view = new EditorView({
      doc: preview.content,
      parent: host,
      extensions: [
        basicSetup,
        keymap.of([indentWithTab]),
        lang.of([]),
        themeComp.of(buildEditorTheme(theme)),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) onDirtyChange(true);
        }),
      ],
    });
    editorRef.current = view;

    // 按扩展名异步加载语言支持（/language-data 懒加载 parser）。
    const desc = LanguageDescription.matchFilename(languages, preview.name);
    let cancelled = false;
    if (desc) {
      desc
        .load()
        .then((support) => {
          if (!cancelled) view.dispatch({ effects: lang.reconfigure(support) });
        })
        .catch(() => {});
    }

    return () => {
      cancelled = true;
      view.destroy();
      editorRef.current = null;
      compsRef.current = null;
    };
    // 仅 key 在 preview.path：换文件重建；主题变化走下方单独 effect。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [preview.path]);

  // 主题变化：仅重配置主题 compartment，保留文档与编辑状态。
  useEffect(() => {
    const view = editorRef.current;
    const comp = compsRef.current;
    if (!view || !comp) return;
    view.dispatch({ effects: comp.theme.reconfigure(buildEditorTheme(theme)) });
  }, [theme]);

  return <div className="preview-code-host" ref={hostRef} />;
}

export type PreviewDockProps = {
  preview: Preview;
  theme: ThemeState;
  width: number;
  minWidth?: number;
  maxWidth?: number;
  onClose: () => void;
  onResize: (width: number) => void;
};

export function PreviewDock({
  preview,
  theme,
  width,
  minWidth = 320,
  maxWidth = 760,
  onClose,
  onResize,
}: PreviewDockProps) {
  const dockRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<EditorView | null>(null);
  const [dirty, setDirty] = useState(false);
  const [saved, setSaved] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  // 把同步来的主题色注入 dock（CSS 变量），供面板与编辑器统一着色。
  useEffect(() => {
    const el = dockRef.current;
    if (!el) return;
    el.style.setProperty("--dock-bg", theme.colors.bg);
    el.style.setProperty("--dock-layer", theme.colors.layer);
    el.style.setProperty("--dock-label", theme.colors.label);
    el.style.setProperty("--dock-label2", theme.colors.label2);
    el.style.setProperty("--dock-border", theme.colors.border);
    el.style.setProperty("--dock-accent", theme.accent);
  }, [theme]);

  // 换文件时重置脏标记（error 类型无 path，用 name 作为稳定 key）。
  const fileKey = "path" in preview ? preview.path : preview.name;
  useEffect(() => {
    setDirty(false);
    setSaved(false);
    setSaveError(null);
  }, [fileKey]);

  const startResize = (e: ReactPointerEvent) => {
    e.preventDefault();
    const handle = e.currentTarget as HTMLElement;
    // setPointerCapture：把后续 pointermove/pointerup 都定向到本元素。
    // 否则鼠标快速移出手柄/窗口后事件不再到达，拖拽"丢失"、且 pointerup 也收不到
    // （表现为松手后鼠标回来仍在调宽）。捕获后无论指针在哪都收得到，松手必触发 up。
    handle.setPointerCapture(e.pointerId);
    const startX = e.clientX;
    const startW = width;
    let raf = 0;
    const onMove = (ev: PointerEvent) => {
      const cx = ev.clientX;
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        // 面板在右侧，向左拖拽 → 宽度增大。
        const next = Math.min(maxWidth, Math.max(minWidth, startW + (startX - cx)));
        onResize(next);
      });
    };
    const onUp = (ev: PointerEvent) => {
      cancelAnimationFrame(raf);
      if (handle.hasPointerCapture(ev.pointerId)) handle.releasePointerCapture(ev.pointerId);
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onUp);
      handle.removeEventListener("pointercancel", onUp);
    };
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onUp);
    handle.addEventListener("pointercancel", onUp);
  };

  const handleSave = async () => {
    if (preview.kind !== "text") return;
    const view = editorRef.current;
    if (!view) return;
    const content = view.state.doc.toString();
    setSaveError(null);
    try {
      await invoke("write_text_file", { path: preview.path, content });
      setDirty(false);
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    } catch (err) {
      setSaveError(String(err));
    }
  };

  const isText = preview.kind === "text";
  const dockStyle: CSSProperties = { width: `${width}px` };

  return (
    <aside className="preview-dock" ref={dockRef} style={dockStyle}>
      <div className="preview-resize" onPointerDown={startResize} aria-hidden />

      <div className="preview-head">
        <span className="preview-name" title={preview.name}>
          {preview.name}
        </span>
        <div className="preview-actions">
          {isText && (
            <button
              type="button"
              className="preview-save"
              onClick={handleSave}
              disabled={!dirty}
            >
              {saved ? "已保存" : "保存"}
            </button>
          )}
          <button type="button" className="preview-close" onClick={onClose} aria-label="关闭">
            ✕
          </button>
        </div>
      </div>

      {saveError && <div className="preview-error-bar">{saveError}</div>}

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
          <CodeEditor
            preview={preview}
            theme={theme}
            editorRef={editorRef}
            onDirtyChange={setDirty}
          />
        )}
        {preview.kind === "error" && <div className="preview-error">{preview.message}</div>}
      </div>
    </aside>
  );
}
